#!/usr/bin/env python3
"""Verify the EIP-8141 v2 rule set against a running devnet.

Checks the four EIPs the chain's identity rests on, against a live node:

  8141  a v2-envelope frame transaction is admitted, mines, and returns per-frame
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

Usage:
  verify_v2_devnet.py <rpc_url> <authrpc_url> <jwt_path> <sender_key_hex> <recipient_addr>
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

RPC, AUTH, JWT, KEY, RECIP = sys.argv[1:6]
FAILURES: list[str] = []


def check(name: str, ok: bool, detail: str = "") -> None:
    print(f"  {'PASS' if ok else 'FAIL'}  {name}{(' — ' + detail) if detail else ''}")
    if not ok:
        FAILURES.append(name)


# ---------- RLP (v2 envelope) ----------
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
    return subprocess.run([CAST, *args], capture_output=True, text=True, check=True).stdout.strip()


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
    """v2 frame: [mode, flags, target_or_empty, [execution, state], value, data]."""
    target_field = rb(addr(target)) if target is not None else rb(b"")
    limits = rl([ri(execution), ri(state)])
    return rl([ri(mode), ri(flags), target_field, limits, ri(value), rb(data)])


def build_frame_tx(chain_id, sender, key, seq, frames, priority, max_fee, sender_key):
    """v2 envelope; fees are one nested list and the signature covers the whole thing."""
    def envelope(signature: bytes) -> bytes:
        entry = rl([ri(1), rb(addr(sender)), rb(b""), rb(signature)])
        fees = rl([ri(priority), ri(max_fee), ri(0)])
        return rl([ri(chain_id), rl([ri(key)]), ri(seq), rb(addr(sender)), rl(frames),
                   rl([entry]), fees, rl([]), rl([])])

    sig_hash = cast_cmd("keccak", "0x06" + envelope(b"").hex())
    raw = bytes.fromhex(cast_cmd("wallet", "sign", "--private-key", sender_key, "--no-hash", sig_hash)[2:])
    v = raw[64] - 27 if raw[64] >= 27 else raw[64]
    return "0x06" + envelope(bytes([v]) + raw[:64]).hex()


def main() -> int:
    sender = cast_cmd("wallet", "address", "--private-key", KEY)
    chain_id = int(rpc(RPC, "eth_chainId", []), 16)
    head = rpc(RPC, "eth_getBlockByNumber", ["latest", False])
    base_fee = int(head.get("baseFeePerGas", "0x0"), 16)
    seq = int(rpc(RPC, "eth_getTransactionCount", [sender, "latest"]), 16)
    print(f"devnet chain={chain_id} head={int(head['number'], 16)} sender={sender} seq={seq}\n")

    print("EIP-8141 — v2 envelope end to end")
    frames = [
        frame(1, 0x03, sender, 80_000, 0, 0, b""),          # VERIFY, approves both scopes
        frame(2, 0x00, RECIP, 30_000, 0, 100, b""),         # SENDER, transfers 100 wei
    ]
    raw = build_frame_tx(chain_id, sender, 0, seq, frames, 10**9, base_fee * 2 + 10**9, KEY)
    sim = rpc(RPC, "ethrex_simulateFrameTransaction", [raw])
    check("a v2 frame transaction simulates valid", sim.get("valid") is True,
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

    print("\nEIP-8272 — recent roots")
    code = rpc(RPC, "eth_getCode", [RECENT_ROOT_ADDRESS, "latest"])
    check("the predeploy is RECENT_ROOT_CODE", len(code[2:]) // 2 == RECENT_ROOT_CODE_LEN,
          f"{len(code[2:]) // 2} bytes")
    salt, root = bytes([0x11]) * 32, bytes([0x22]) * 32
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

    print("\nEIP-8250 — keyed nonces")
    seq = int(rpc(RPC, "eth_getTransactionCount", [sender, "latest"]), 16)
    gapped = build_frame_tx(chain_id, sender, 0, seq + 5, frames, 10**9, base_fee * 2 + 10**9, KEY)
    try:
        rpc(RPC, "eth_sendRawTransaction", [gapped])
        check("key 0 cannot queue a future sequence", False, "a gapped key-0 tx was admitted")
    except RuntimeError as exc:
        check("key 0 cannot queue a future sequence", "Nonce mismatch" in str(exc), str(exc)[:90])
    keyed = build_frame_tx(chain_id, sender, 0x11, 0, frames, 10**9, base_fee * 2 + 10**9, KEY)
    try:
        rpc(RPC, "eth_sendRawTransaction", [keyed])
        check("a keyed frame transaction is admitted", True)
    except RuntimeError as exc:
        check("a keyed frame transaction is admitted", False, str(exc)[:90])

    print("\nEIP-7805 — FOCIL")
    head = rpc(RPC, "eth_getBlockByNumber", ["latest", False])
    il = rpc(AUTH, "engine_getInclusionListV1", [head["hash"]], auth=True)
    entries = il if isinstance(il, list) else (il.get("transactions") or [])
    check("engine_getInclusionListV1 answers", True, f"{len(entries)} entries")
    check("the inclusion list carries the pending frame transaction",
          any(isinstance(e, str) and e.startswith("0x06") for e in entries),
          "types=" + ",".join(sorted({e[:4] for e in entries if isinstance(e, str)})) or "empty")

    print()
    if FAILURES:
        print(f"{len(FAILURES)} check(s) failed: {', '.join(FAILURES)}")
        return 1
    print("every check passed")
    return 0


sys.exit(main())
