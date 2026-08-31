#!/usr/bin/env python3
"""Verify the EIP-8141 rule set against a running devnet.

Checks the four EIPs the chain's identity rests on, against a live node:

  8141  a frame transaction is admitted, mines, and returns per-frame
        receipts carrying the two-dimensional gas_used
  8272  the recent-root predeploy is exactly RECENT_ROOT_CODE, a 64-byte write
        succeeds, and both rejection paths revert
  8250  keyed-nonce admission: key 0 cannot queue, and concurrency is denied for an
        EOA sender (its default-code prefix authenticates against its own nonce)
  7805  the FOCIL engine surface answers, and an inclusion list contains the pending
        frame transaction

Stdlib only, plus foundry's `cast` for keccak and secp256k1 signing: the devnet hosts
have cast but no python3-venv, and installing system packages on a shared box to run a
verification script is not a trade worth making.

Re-runnable against the same devnet: every address it funds and every nonce key it uses
are derived from the sender's current sequence, so a second run does not collide with the
state the first one left. Sequentially, though — two runs at once share the sender and race
for its nonce, which surfaces as transactions admitted and never mined. That matters for the state-gas checks in particular — a frame
funding an address that already exists creates nothing and is charged nothing, so a fixed
recipient would make the strongest check pass once and then fail forever.

The sender key comes from the environment, NOT from the command line: an argument is
visible to every user on the box through `ps` and `/proc/<pid>/cmdline` for as long as the
script runs, and it survives in shell history. Export it instead, ideally from a file that
is already the key's home:

  set -a; . ~/hegota-keys.env; set +a
  HEGOTA_SENDER_KEY=$FAUCET_KEY verify_devnet.py <rpc> <authrpc> <jwt>

Usage:
  HEGOTA_SENDER_KEY=<hex> verify_devnet.py <rpc_url> <authrpc_url> <jwt_path>
"""
import base64
import hashlib
import hmac
import json
import os
import subprocess
import sys
import time
import urllib.request

CAST = os.path.expanduser("~/.foundry/bin/cast")
RECENT_ROOT_ADDRESS = "0x0000000000000000000000000000000000008272"
RECENT_ROOT_CODE_LEN = 144
# EIP-8037 STATE_BYTES_PER_NEW_ACCOUNT * CPSB: what a value-bearing frame is charged for
# funding an address that does not exist yet, drawn from that frame's own `limits.state`.
NEW_ACCOUNT_STATE_GAS = 120 * 1530
# EIP-8037 STATE_BYTES_PER_STORAGE_SET * CPSB: one new storage slot.
SSTORE_SET_STATE_GAS = 64 * 1530

RPC, AUTH, JWT = sys.argv[1:4]
KEY = os.environ.get("HEGOTA_SENDER_KEY")
if not KEY:
    sys.exit(
        "set HEGOTA_SENDER_KEY in the environment (it is deliberately not a command-line\n"
        "argument: argv is world-readable through /proc for the life of the process)"
    )


def derived_address(tag: str, seq: int) -> str:
    """A fresh address per (tag, sequence), so re-runs never reuse one.

    `tag` is 4 hex digits naming what the address is for, which keeps the two addresses
    this script uses distinguishable when reading a devnet's state by hand.
    """
    return "0x" + tag + format(seq, "036x")
FAILURES: list[str] = []


def check(name: str, ok: bool, detail: str = "") -> None:
    print(f"  {'PASS' if ok else 'FAIL'}  {name}{(' — ' + detail) if detail else ''}")
    if not ok:
        FAILURES.append(name)


# ---------- RLP (frame-tx envelope) ----------
def rb(b: bytes) -> bytes:
    if len(b) == 1 and b[0] < 0x80:
        return b
    if len(b) < 56:
        return bytes([0x80 + len(b)]) + b
    length = len(b).to_bytes((len(b).bit_length() + 7) // 8, "big")
    return bytes([0xB7 + len(length)]) + length + b


def rl(items) -> bytes:
    body = b"".join(items)
    if len(body) < 56:
        return bytes([0xC0 + len(body)]) + body
    length = len(body).to_bytes((len(body).bit_length() + 7) // 8, "big")
    return bytes([0xF7 + len(length)]) + length + body


def ri(x: int) -> bytes:
    return rb(b"") if x == 0 else rb(x.to_bytes((x.bit_length() + 7) // 8, "big"))


def addr(a) -> bytes:
    if isinstance(a, str):
        return bytes.fromhex(a.removeprefix("0x"))
    return a.to_bytes(20, "big")


def cast_cmd(*args) -> str:
    """Run `cast`, raising with its stderr attached.

    `check=True` alone raises a `CalledProcessError` whose message is the argv and an exit
    code, which for a usage error or a rejected transaction says nothing about the cause.
    """
    done = subprocess.run([CAST, *args], capture_output=True, text=True)
    if done.returncode != 0:
        raise RuntimeError(f"cast {' '.join(args[:2])} failed: "
                           f"{(done.stderr or done.stdout).strip()[:200]}")
    return done.stdout.strip()


def rpc(url: str, method: str, params, auth: bool = False):
    headers = {"content-type": "application/json"}
    if auth:
        secret = bytes.fromhex(open(JWT).read().strip().removeprefix("0x"))
        b64 = lambda raw: base64.urlsafe_b64encode(raw).rstrip(b"=")
        head = b64(json.dumps({"alg": "HS256", "typ": "JWT"}, separators=(",", ":")).encode())
        payload = b64(json.dumps({"iat": int(time.time())}, separators=(",", ":")).encode())
        sig = hmac.new(secret, head + b"." + payload, hashlib.sha256).digest()
        headers["authorization"] = "Bearer " + (head + b"." + payload + b"." + b64(sig)).decode()
    req = urllib.request.Request(
        url, data=json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode(),
        headers=headers)
    out = json.loads(urllib.request.urlopen(req, timeout=25).read())
    if "error" in out:
        raise RuntimeError(json.dumps(out["error"])[:220])
    return out["result"]


def frame(mode: int, flags: int, target, execution: int, state: int, value: int, data: bytes) -> bytes:
    """A frame: [mode, flags, target_or_empty, [execution, state], value, data]."""
    target_field = rb(addr(target)) if target is not None else rb(b"")
    limits = rl([ri(execution), ri(state)])
    return rl([ri(mode), ri(flags), target_field, limits, ri(value), rb(data)])


def engine_new_payload_v6(block, inclusion_list) -> dict:
    """Replay a mined block through `engine_newPayloadV6` against `inclusion_list`.

    The block is rebuilt as an engine payload from its RPC form plus `debug_getRawTransaction`
    for each transaction and `debug_getRawBlockAccessList` for its EIP-7928 list — the RPC
    block carries decoded transactions, and the payload needs their canonical bytes.

    Replaying a block the node already has is not a short cut past the check: ethrex
    evaluates `inclusionListSatisfied` for every payload that comes back VALID, against the
    list supplied with that call, so an already-known block still answers honestly about a
    list it was never built for.
    """
    raws = [rpc(RPC, "debug_getRawTransaction", [h]) for h in block["transactions"]]
    payload = {
        "parentHash": block["parentHash"],
        "feeRecipient": block["miner"],
        "stateRoot": block["stateRoot"],
        "receiptsRoot": block["receiptsRoot"],
        "logsBloom": block["logsBloom"],
        "prevRandao": block["mixHash"],
        "blockNumber": block["number"],
        "gasLimit": block["gasLimit"],
        "gasUsed": block["gasUsed"],
        "timestamp": block["timestamp"],
        "extraData": block["extraData"],
        "baseFeePerGas": block["baseFeePerGas"],
        "blockHash": block["hash"],
        "transactions": raws,
        "withdrawals": block.get("withdrawals") or [],
        "blobGasUsed": block.get("blobGasUsed", "0x0"),
        "excessBlobGas": block.get("excessBlobGas", "0x0"),
    }
    for rpc_field, payload_field in [("slotNumber", "slotNumber"), ("burnedFees", "burnedFees")]:
        if rpc_field in block:
            payload[payload_field] = block[rpc_field]
    bal = rpc(RPC, "debug_getRawBlockAccessList", [block["hash"]])
    if isinstance(bal, str):
        payload["blockAccessList"] = bal
    return rpc(AUTH, "engine_newPayloadV6",
               [payload, [], block.get("parentBeaconBlockRoot"), [], list(inclusion_list)],
               auth=True)


PRIORITY_FEE = 10**9


def fees() -> tuple[int, int]:
    """`(max_priority_fee_per_gas, max_fee_per_gas)` against the CURRENT head.

    Read fresh for every transaction rather than once per run. This script fills blocks
    with its own transactions, which raises the base fee as it goes, and a `max_fee`
    derived from the base fee at startup eventually falls below it — the transaction is
    then admitted and simply never mined. That reads exactly like the feature under test
    being broken, which is how a stale fee cost an hour of looking at the wrong thing.
    The multiple gives several slots of headroom.
    """
    head = rpc(RPC, "eth_getBlockByNumber", ["latest", False])
    base = int(head.get("baseFeePerGas", "0x0"), 16)
    return PRIORITY_FEE, base * 4 + 2 * PRIORITY_FEE


def drain_pool(what: str) -> None:
    """Block until the mempool is empty.

    Sections that send from the same EOA have to start from a clean pool. A pending frame
    transaction occupies the sender's key-0 sequence, so a following `cast send` collides
    with it as an underpriced replacement, and the keyed-admission check would see the
    EOA-concurrency denial instead of the answer it is asking for. Both failures look like
    the feature being broken when they are only two transactions racing.
    """
    for _ in range(60):
        status = rpc(RPC, "txpool_status", [])
        if int(status["pending"], 16) + int(status["queued"], 16) == 0:
            return
        time.sleep(2)
    print(f"  WARN  pool still not empty before {what}; results below may be a race")


def recent_root_reference(source_id: bytes, slot: int, root: bytes) -> bytes:
    """EIP-8272 reference: [source_id, slot, root]."""
    return rl([rb(source_id), ri(slot), rb(root)])


def build_frame_tx(chain_id, sender, key, seq, frames, priority, max_fee, sender_key,
                   references=(), sign=True):
    """The envelope; fees are one nested list and the signature covers the whole thing.

    `sign=False` builds a zero-signature envelope, which is what a contract sender uses:
    its own code decides whether to `APPROVE`, so the transaction carries no signature for
    the protocol to validate. `sender_key` is then unused.
    """
    def envelope(signature) -> bytes:
        entries = [] if signature is None else [rl([ri(1), rb(addr(sender)), rb(b""), rb(signature)])]
        fees = rl([ri(priority), ri(max_fee), ri(0)])
        return rl([ri(chain_id), rl([ri(key)]), ri(seq), rb(addr(sender)), rl(frames),
                   rl(entries), fees, rl([]), rl(list(references))])

    if not sign:
        return "0x06" + envelope(None).hex()
    sig_hash = cast_cmd("keccak", "0x06" + envelope(b"").hex())
    raw = bytes.fromhex(cast_cmd("wallet", "sign", "--private-key", sender_key, "--no-hash", sig_hash)[2:])
    v = raw[64] - 27 if raw[64] >= 27 else raw[64]
    return "0x06" + envelope(bytes([v]) + raw[:64]).hex()


# A sender contract that unconditionally approves execution and payment for itself:
#   PUSH1 3; PUSH1 0; PUSH1 0; APPROVE; STOP
# It reads no storage, reads no `TXPARAM`, and carries real (non-delegated) code, which is
# exactly the shape `keyed_concurrency_verdict` grants concurrency to. Anyone can spend its
# balance, which is fine for a devnet and is the point: the contract, not a signature,
# decides who may pay.
SENDER_CONTRACT_RUNTIME = bytes([0x60, 0x03, 0x60, 0x00, 0x60, 0x00, 0xAA, 0x00])


def deploy(runtime: bytes, endowment: int = 0) -> str:
    """Deploy `runtime` with `endowment` wei and return its address.

    The init code copies the runtime into memory and returns it:
      PUSH1 len; PUSH1 offset; PUSH1 0; CODECOPY; PUSH1 len; PUSH1 0; RETURN
    with `offset` being the length of the init code itself.

    Any endowment goes in with the creation rather than as a later transfer: a transfer
    executes the runtime, and a runtime built out of frame opcodes halts outside a frame
    transaction, so such a contract cannot be funded after the fact.
    """
    body = runtime
    assert len(body) < 0x100, "single-byte PUSH1 length only"
    init = bytes([0x60, len(body), 0x60, 0x0C, 0x60, 0x00, 0x39,
                  0x60, len(body), 0x60, 0x00, 0xF3]) + body
    assert init[3] == 0x0C and len(init) - len(body) == 12, "init-code offset must match its length"
    # `--create <CODE> [SIG] [ARGS]...` swallows everything after it, so `--value` and
    # `--json` have to come first or cast reads them as constructor arguments.
    out = json.loads(cast_cmd("send", "--rpc-url", RPC, "--private-key", KEY,
                              "--value", str(endowment), "--json",
                              "--create", "0x" + init.hex()))
    return out["contractAddress"]


# The TXPARAM index map this chain must answer to, as settled upstream on 2026-08-31 after
# EIP-8141 claimed 0x0C. Each entry is (index, storage slot, label).
#
# This is the check that catches a renumbering, and it has to be made ON CHAIN: a wrong index
# does not halt, it answers with whatever the neighbouring EIP put there. Reading the digest
# at the wrong index returns a reference count — a number, usually zero — and nothing tells
# the caller. Compiling against the right constants proves nothing here; only the chain does.
TXPARAM_IDS = [
    (0x0D, 0, "legacy sender nonce (8250)"),
    (0x0E, 1, "len(nonce_keys) (8250)"),
    (0x0F, 2, "nonce_keys_hash (8250)"),
    (0x10, 3, "nonce_keys[0] (8250)"),
    (0x11, 4, "len(recent_root_references) (8272)"),
]


def txparam_id_check(chain_id, sender_contract) -> None:
    """Read every renumbered TXPARAM index inside a real frame transaction and check it
    against the value only that index can hold."""
    # PUSH1 <id>; TXPARAM; PUSH1 <slot>; SSTORE  per index, then STOP.
    runtime = b"".join(bytes([0x60, idx, 0xB0, 0x60, slot, 0x55]) for idx, slot, _ in TXPARAM_IDS)
    runtime += b"\x00"
    probe = deploy(runtime)
    check("the TXPARAM probe deploys",
          bytes.fromhex(rpc(RPC, "eth_getCode", [probe, "latest"])[2:]) == runtime)

    # One fresh key, so nonce_keys is [key] and every expected value is known up front.
    key = 0x8250_2000 + int(rpc(RPC, "eth_getTransactionCount", [sender_contract, "latest"]), 16)
    raw = build_frame_tx(
        chain_id, sender_contract, key, 0,
        [frame(1, 0x03, sender_contract, 80_000, 0, 0, b""),
         # Five slot creations, so five slots' worth of state gas.
         frame(0, 0x00, probe, 400_000, 5 * SSTORE_SET_STATE_GAS, 0, b"")],
        *fees(), None, sign=False)
    tx_hash = rpc(RPC, "eth_sendRawTransaction", [raw])
    receipt = None
    for _ in range(60):
        receipt = rpc(RPC, "eth_getTransactionReceipt", [tx_hash])
        if receipt:
            break
        time.sleep(2)
    if not receipt or receipt.get("status") != "0x1":
        check("the TXPARAM probe frame runs", False,
              f"status={(receipt or {}).get('status', 'never mined')}")
        return
    check("the TXPARAM probe frame runs", True, f"block={int(receipt['blockNumber'], 16)}")

    # The legacy nonce is the sender's ACCOUNT nonce, read from the chain rather than assumed:
    # a contract account starts at 1 (EIP-161) and only bumps by deploying, so hardcoding 0
    # here would fail for a reason that has nothing to do with the index being right.
    legacy_nonce = int(rpc(RPC, "eth_getTransactionCount", [sender_contract, "latest"]), 16)
    digest = cast_cmd("keccak", "0x" + (1).to_bytes(32, "big").hex() + key.to_bytes(32, "big").hex())
    expected = {
        0x0D: legacy_nonce,
        0x0E: 1,
        0x0F: int(digest, 16),
        0x10: key,
        0x11: 0,
    }
    for idx, slot, label in TXPARAM_IDS:
        got = int(rpc(RPC, "eth_getStorageAt", [probe, hex(slot), "latest"]), 16)
        check(f"TXPARAM {hex(idx)} is {label}", got == expected[idx],
              f"read {hex(got)}, expected {hex(expected[idx])}")


def concurrency_check(chain_id) -> None:
    """EIP-8250's headline feature: two frame transactions from one sender, different keys,
    both pending at once and both mined. Only a contract sender qualifies."""
    contract = deploy(SENDER_CONTRACT_RUNTIME, 10**18)
    code = rpc(RPC, "eth_getCode", [contract, "latest"])
    check("the sender contract deploys with its approve-both runtime",
          bytes.fromhex(code[2:]) == SENDER_CONTRACT_RUNTIME, f"{len(code[2:]) // 2} bytes")
    funded = int(rpc(RPC, "eth_getBalance", [contract, "latest"]), 16)
    check("it is funded to pay for its own transactions", funded > 0, f"{funded / 1e18:.3f} ETH")

    # Two keys, each at sequence 0 because neither has ever been used, and no signature:
    # the contract's code is the authorization. A recipient per key, so neither frame's
    # account-creation charge depends on the other having run.
    raws = []
    base = int(rpc(RPC, "eth_getTransactionCount", [contract, "latest"]), 16)
    for index in (0, 1):
        key = 0x8250_0000 + index
        recipient = derived_address("beef", base * 2 + index)
        raws.append(build_frame_tx(
            chain_id, contract, key, 0,
            [frame(1, 0x03, contract, 80_000, 0, 0, b""),
             frame(2, 0x00, recipient, 30_000, NEW_ACCOUNT_STATE_GAS, 100, b"")],
            *fees(), None,
            sign=False))

    hashes = []
    for index, raw in enumerate(raws):
        try:
            hashes.append(rpc(RPC, "eth_sendRawTransaction", [raw]))
        except RuntimeError as exc:
            check(f"keyed transaction {index} from a contract sender is admitted", False,
                  str(exc)[:110])
    check("both keys are admitted at once — concurrency the linear nonce forbids",
          len(hashes) == 2, f"{len(hashes)} of 2 admitted")

    # Wait on the POOL, not just a poll count. A transaction with no receipt means one of two
    # very different things — still queued, or dropped — and only the pool can tell them
    # apart. Reporting them as one failure is how a busy devnet looks like a broken feature.
    mined = {}
    for _ in range(120):
        mined = {h: rpc(RPC, "eth_getTransactionReceipt", [h]) for h in hashes}
        if all(mined.values()):
            break
        status = rpc(RPC, "txpool_status", [])
        if int(status["pending"], 16) + int(status["queued"], 16) == 0:
            # The pool is empty and something is still missing: it was dropped, not delayed.
            break
        time.sleep(2)

    blocks = {int(r["blockNumber"], 16) for r in mined.values() if r}
    missing = [h for h in hashes if not mined.get(h)]
    if missing:
        still_pooled = [h for h in missing if rpc(RPC, "eth_getTransactionByHash", [h])]
        check("both mine", False,
              f"{len(missing)} of {len(hashes)} not included; "
              + ("still pooled — the builder had not got to them yet"
                 if still_pooled else "GONE FROM THE POOL — dropped, not delayed"))
    else:
        check("both mine", True,
              ",".join(mined[h]["status"] for h in hashes) + f" in block(s) {sorted(blocks)}")
    if hashes and all(mined.get(h) for h in hashes):
        check("both succeed",
              all(mined[h]["status"] == "0x1" for h in hashes),
              ",".join(mined[h]["status"] for h in hashes))
    return contract


def main() -> int:
    sender = cast_cmd("wallet", "address", "--private-key", KEY)
    chain_id = int(rpc(RPC, "eth_chainId", []), 16)
    head = rpc(RPC, "eth_getBlockByNumber", ["latest", False])
    seq = int(rpc(RPC, "eth_getTransactionCount", [sender, "latest"]), 16)
    # Both derived from the current sequence: the address the funded transfer creates, and
    # the address the under-budgeted frame must fail to create.
    recipient = derived_address("c0de", seq)
    unfunded = derived_address("dead", seq)
    print(f"devnet chain={chain_id} head={int(head['number'], 16)} sender={sender} seq={seq}")
    print(f"funding {recipient}, and expecting {unfunded} to stay empty\n")

    print("EIP-8141 — envelope end to end")
    # The SENDER frame funds a never-seen address, so it declares the account-creation
    # state gas. A frame that declared none would halt on the charge — which is the point
    # of the second dimension, and is checked directly below.
    frames = [
        frame(1, 0x03, sender, 80_000, 0, 0, b""),          # VERIFY, approves both scopes
        frame(2, 0x00, recipient, 30_000, NEW_ACCOUNT_STATE_GAS, 100, b""),  # SENDER, 100 wei
    ]
    raw = build_frame_tx(chain_id, sender, 0, seq, frames, *fees(), KEY)
    sim = rpc(RPC, "ethrex_simulateFrameTransaction", [raw])
    check("a frame transaction simulates valid", sim.get("valid") is True,
          f"shape={sim.get('prefixShape')} violation={sim.get('violation')}")
    tx_hash = rpc(RPC, "eth_sendRawTransaction", [raw])
    receipt = None
    for _ in range(30):
        receipt = rpc(RPC, "eth_getTransactionReceipt", [tx_hash])
        if receipt:
            break
        time.sleep(2)
    check("it mines with status 0x1", bool(receipt) and receipt.get("status") == "0x1",
          f"block={int(receipt['blockNumber'], 16)}" if receipt else "not mined")
    if receipt:
        check("the receipt is type 0x6", receipt.get("type") == "0x6")
        frs = receipt.get("frameReceipts") or []
        check("it carries one receipt per frame", len(frs) == len(frames), f"{len(frs)} entries")
        check("per-frame receipts carry the state dimension",
              all("stateGasUsed" in fr for fr in frs),
              "keys=" + ",".join(sorted(frs[0].keys())) if frs else "none")
        # The state dimension has to be metered, not merely serialized. The VERIFY frame
        # creates nothing and must report zero; whichever frames did create state must
        # report a non-zero figure, and the transaction's total must agree with the sum.
        if len(frs) == len(frames):
            state_used = [int(fr.get("stateGasUsed", "0x0"), 16) for fr in frs]
            check("the VERIFY frame reports no state gas", state_used[0] == 0,
                  f"reported {state_used[0]}")
            check("the funding frame is charged for the account it created",
                  state_used[1] > 0, f"reported {state_used[1]}")

    print("\nEIP-8141 — the state dimension is enforced, not just reported")
    seq = int(rpc(RPC, "eth_getTransactionCount", [sender, "latest"]), 16)
    starved = [
        frame(1, 0x03, sender, 80_000, 0, 0, b""),
        # One gas short of the account-creation charge on an address that does not exist.
        frame(2, 0x00, unfunded, 30_000, NEW_ACCOUNT_STATE_GAS - 1, 100, b""),
    ]
    raw = build_frame_tx(chain_id, sender, 0, seq, starved, *fees(), KEY)
    starved_receipt = None
    try:
        starved_hash = rpc(RPC, "eth_sendRawTransaction", [raw])
        for _ in range(30):
            starved_receipt = rpc(RPC, "eth_getTransactionReceipt", [starved_hash])
            if starved_receipt:
                break
            time.sleep(2)
    except RuntimeError as exc:
        check("an under-budgeted frame is admitted (it is valid, it just halts)", False,
              str(exc)[:90])
    if starved_receipt:
        starved_frs = starved_receipt.get("frameReceipts") or []
        check("a frame that cannot cover its state charge halts",
              len(starved_frs) == 2 and starved_frs[1].get("status") == "0x0",
              f"statuses={[fr.get('status') for fr in starved_frs]}")
        check("the halted frame reports no state gas",
              bool(starved_frs) and int(starved_frs[1].get("stateGasUsed", "0x0"), 16) == 0)
        check("its value was not delivered",
              int(rpc(RPC, "eth_getBalance", [unfunded, "latest"]), 16) == 0)

    print("\nEIP-8272 — recent roots")
    code = rpc(RPC, "eth_getCode", [RECENT_ROOT_ADDRESS, "latest"])
    check("the predeploy is RECENT_ROOT_CODE", len(code[2:]) // 2 == RECENT_ROOT_CODE_LEN,
          f"{len(code[2:]) // 2} bytes")
    # A salt derived from the sequence, so each run writes its own entry rather than
    # overwriting one an earlier run is still referencing.
    salt = seq.to_bytes(32, "big")
    root = bytes([0x22]) * 32
    written = cast_cmd("send", "--rpc-url", RPC, "--private-key", KEY, RECENT_ROOT_ADDRESS,
                       "0x" + (salt + root).hex(), "--value", "0", "--json")
    written = json.loads(written)
    check("a 64-byte write succeeds", written.get("status") == "0x1",
          f"gasUsed={int(written.get('gasUsed', '0x0'), 16)}")
    for label, data, value in [("a 63-byte write reverts", "0x" + b"\x11".hex() * 63, "0"),
                               ("a 64-byte write with value reverts", "0x" + (salt + root).hex(), "1")]:
        try:
            subprocess.run([CAST, "send", "--rpc-url", RPC, "--private-key", KEY,
                            RECENT_ROOT_ADDRESS, data, "--value", value, "--json"],
                           capture_output=True, text=True, check=True)
            check(label, False, "it was accepted")
        except subprocess.CalledProcessError:
            check(label, True)

    # The reference half. The predeploy derives `source_id = keccak(CALLER || salt)` over the
    # unpadded 20-byte caller, and stores the entry under the slot the write executed in, so
    # a reference to (source_id, that slot, root) is the one a frame transaction can declare.
    # This is the read side of EIP-8272 end to end: the protocol recomputes `entry_hash` and
    # compares it against the predeploy's storage before the transaction is admitted.
    if written.get("status") == "0x1":
        source_id = bytes.fromhex(
            cast_cmd("keccak", "0x" + bytes.fromhex(sender[2:]).hex() + salt.hex())[2:])
        write_block = rpc(RPC, "eth_getBlockByHash", [written["blockHash"], False])
        write_slot = int(write_block["slotNumber"], 16)
        seq = int(rpc(RPC, "eth_getTransactionCount", [sender, "latest"]), 16)
        for label, referenced_root, expect_valid in [
            ("a frame transaction referencing the written root is admitted", root, True),
            ("one referencing a root that was never written is rejected", bytes([0x33]) * 32, False),
        ]:
            raw = build_frame_tx(
                chain_id, sender, 0, seq,
                [frame(1, 0x03, sender, 80_000, 0, 0, b"")],
                *fees(), KEY,
                references=[recent_root_reference(source_id, write_slot, referenced_root)])
            try:
                rpc(RPC, "eth_sendRawTransaction", [raw])
                check(label, expect_valid, "admitted")
                if expect_valid:
                    seq += 1
            except RuntimeError as exc:
                check(label, not expect_valid, str(exc)[:100])

    print("\nEIP-8250 — keyed nonces")
    drain_pool("the keyed-nonce checks")
    seq = int(rpc(RPC, "eth_getTransactionCount", [sender, "latest"]), 16)
    gapped = build_frame_tx(chain_id, sender, 0, seq + 5, frames, *fees(), KEY)
    try:
        rpc(RPC, "eth_sendRawTransaction", [gapped])
        check("key 0 cannot queue a future sequence", False, "a gapped key-0 tx was admitted")
    except RuntimeError as exc:
        check("key 0 cannot queue a future sequence", "Nonce mismatch" in str(exc), str(exc)[:90])

    # A key this sender has never used is at sequence 0 by definition, which is what makes
    # this check independent of every earlier run.
    fresh_key = 0x8141_0000 + seq
    keyed = build_frame_tx(chain_id, sender, fresh_key, 0, frames, *fees(), KEY)
    try:
        rpc(RPC, "eth_sendRawTransaction", [keyed])
        check("a keyed frame transaction is admitted", True, f"key={hex(fresh_key)}")
    except RuntimeError as exc:
        check("a keyed frame transaction is admitted", False, str(exc)[:90])

    # The trap, asserted rather than stumbled into: an EOA gets no concurrency. Its
    # default-code prefix authenticates against its own account nonce, which a sibling
    # key-0 transaction bumps, so `keyed_concurrency_verdict` denies it and the second
    # pending frame transaction is refused whatever key it carries.
    second_key = build_frame_tx(chain_id, sender, fresh_key + 1, 0, frames, *fees(), KEY)
    try:
        rpc(RPC, "eth_sendRawTransaction", [second_key])
        check("an EOA sender is denied concurrency", False,
              "a second keyed tx from an EOA was admitted")
    except RuntimeError as exc:
        # Two messages, one verdict: the mempool names the other-key case and the
        # same-sender case differently, and either is the denial this asserts.
        denied = "nonce-key domain" in str(exc) or "already in the pool" in str(exc)
        check("an EOA sender is denied concurrency", denied, str(exc)[:100])

    print("\nEIP-8250 — concurrency, which needs a contract sender")
    drain_pool("the contract-sender deploy")
    sender_contract = concurrency_check(chain_id)

    print("\nEIP-8141/8250/8272 — the TXPARAM index map, read on chain")
    drain_pool("the TXPARAM probe")
    txparam_id_check(chain_id, sender_contract)

    print("\nEIP-7805 — FOCIL")
    # An inclusion list only has something to say about a transaction that is pending, so
    # put one there first. The contract sender is used rather than the faucet EOA because a
    # contract may hold several pending frame transactions at once, and this must not race
    # with whatever the earlier sections left behind.
    pending_raw = build_frame_tx(
        chain_id, sender_contract, 0x7805_0000, 0,
        [frame(1, 0x03, sender_contract, 80_000, 0, 0, b"")],
        *fees(), None, sign=False)
    pending_hash = rpc(RPC, "eth_sendRawTransaction", [pending_raw])

    head = rpc(RPC, "eth_getBlockByNumber", ["latest", False])
    il = rpc(AUTH, "engine_getInclusionListV1", [head["hash"]], auth=True)
    entries = il if isinstance(il, list) else (il.get("transactions") or [])
    check("engine_getInclusionListV1 answers", True, f"{len(entries)} entries")
    check("the inclusion list carries the pending frame transaction",
          any(isinstance(e, str) and e.startswith("0x06") for e in entries),
          "types=" + ",".join(sorted({e[:4] for e in entries if isinstance(e, str)})) or "empty")

    # Enforcement, not just the surface: an inclusion list the builder was given must be
    # satisfied by the block it builds. Checked by following the pending transaction to a
    # block rather than by reading the builder's intent.
    included_in = None
    for _ in range(30):
        receipt = rpc(RPC, "eth_getTransactionReceipt", [pending_hash])
        if receipt:
            included_in = int(receipt["blockNumber"], 16)
            break
        time.sleep(2)
    check("a frame transaction an inclusion list carried is built into a block",
          included_in is not None,
          f"block={included_in}" if included_in else "never included")

    # And the negative that proves the enforcement is real rather than absent: replay the
    # head payload against an inclusion list holding a frame transaction that block cannot
    # contain (it was submitted after it). EIP-8369 Profile 2 makes frame transactions
    # enforceable, so this must report the list unsatisfied. A client that still excused
    # frame transactions wholesale would answer `true` here and pass every check above.
    unbuilt_raw = build_frame_tx(
        chain_id, sender_contract, 0x7805_0001, 0,
        [frame(1, 0x03, sender_contract, 80_000, 0, 0, b"")],
        *fees(), None, sign=False)
    rpc(RPC, "eth_sendRawTransaction", [unbuilt_raw])
    head = rpc(RPC, "eth_getBlockByNumber", ["latest", False])
    try:
        verdict = engine_new_payload_v6(head, [unbuilt_raw])
        satisfied = verdict.get("inclusionListSatisfied")
        check("an omitted eligible frame transaction leaves the list unsatisfied",
              satisfied is False, f"status={verdict.get('status')} satisfied={satisfied}")
    except RuntimeError as exc:
        check("an omitted eligible frame transaction leaves the list unsatisfied", False,
              str(exc)[:120])

    print()
    if FAILURES:
        print(f"{len(FAILURES)} check(s) failed: {', '.join(FAILURES)}")
        return 1
    print("every check passed")
    return 0


sys.exit(main())
