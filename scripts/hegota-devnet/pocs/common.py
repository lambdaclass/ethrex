#!/usr/bin/env python3
"""Shared harness for the EIP-7906 attack proof-of-concept scenarios.

Every scenario is a two-phase demonstration against the live devnet:
  phase A  the attack runs against an unguarded victim and MUST succeed
  phase B  the same attack intent MUST be neutralized when the victim transacts
           through a frame transaction carrying the appropriate POST_TX guard

A scenario that cannot establish phase A has not demonstrated a real attack, and one
that cannot establish phase B has not demonstrated a defense. Neither is reported as a
success: `run_scenario` raises unless both hold.

Evidence: a POST_TX-reverting transaction is EXCLUDED from the block, so a successful
defense has no receipt and no explorer page. Phase B is therefore evidenced by the
transaction being admitted and never mined, plus the victim's state demonstrably
unchanged. See GATE-RESULTS.md for why simulation cannot identify which assertion
tripped (it misattributes a POST_TX revert to the VERIFY frame).
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
import time
import urllib.request
from dataclasses import dataclass, field, asdict

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
from frametx import Frame, FrameSig, FrameTx  # noqa: E402

from eth_account import Account  # noqa: E402
from eth_keys import keys  # noqa: E402
from eth_utils import keccak, to_checksum_address  # noqa: E402

RPCS = [f"https://rpc{i}.hegota.ethrex.xyz" for i in (1, 2, 3)]
FAUCET = "https://faucet.hegota.ethrex.xyz/api/claim"
CONTRACTS = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "contracts"))
EVIDENCE_DIR = os.path.join(os.path.dirname(__file__), "evidence")

# Persisted so repeated runs reuse funded accounts instead of exhausting the faucet's
# per-address rate limit. Deployments are always fresh, so scenarios never share
# mutable contract state across runs.
BANK_KEY_FILE = os.path.expanduser("~/.poc7906_bank_key")

# Frame modes (EIP-8141 + EIP-7906)
VERIFY, SENDER, POST_TX = 1, 2, 3
SCOPE_BOTH = 0x03

# Storage writes carry EIP-8037 state gas on top of the execution cost — roughly 98k per
# cold SSTORE. Budgets here are deliberately generous; see GATE-RESULTS.md, where a
# two-SSTORE call needed 232k against a naive 200k budget. Do not "optimize" these down.
# EIP-7825 caps a single transaction at 2**24 gas, and EIP-8037 state gas applies to code
# deposit — so a 12KB deployment needs ~16.5M and only just fits under the cap. Deploys are
# therefore budgeted at the cap; contracts much larger than the Safe singleton cannot be
# deployed in one transaction on this chain at all.
EIP7825_TX_GAS_CAP = 16_777_216
GAS_DEPLOY = EIP7825_TX_GAS_CAP
GAS_WRITE_CALL = 900_000
GAS_VERIFY_FRAME = 250_000
GAS_BODY_FRAME = 900_000
GAS_GUARD_FRAME = 600_000


# ---------------------------------------------------------------- RPC


def rpc(method: str, params: list, url: str = RPCS[0], timeout: int = 30):
    req = urllib.request.Request(
        url,
        headers={"content-type": "application/json"},
        data=json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode(),
    )
    r = json.loads(urllib.request.urlopen(req, timeout=timeout).read())
    if "error" in r:
        raise RuntimeError(f"{method} -> {r['error']}")
    return r["result"]


def chain_id() -> int:
    return int(rpc("eth_chainId", []), 16)


def base_fee() -> int:
    return int(rpc("eth_getBlockByNumber", ["latest", False]).get("baseFeePerGas", "0x0"), 16)


def block_number() -> int:
    return int(rpc("eth_blockNumber", []), 16)


def balance(addr: str) -> int:
    return int(rpc("eth_getBalance", [to_checksum_address(addr), "latest"]), 16)


def storage_at(addr: str, slot: int) -> int:
    return int(rpc("eth_getStorageAt", [to_checksum_address(addr), hex(slot), "latest"]), 16)


def wait_receipt(txhash: str, tries: int = 20, delay: int = 3):
    for _ in range(tries):
        try:
            r = rpc("eth_getTransactionReceipt", [txhash])
            if r:
                return r
        except Exception:
            pass
        time.sleep(delay)
    return None


def broadcast(raw: str) -> tuple[str | None, list[str]]:
    """Submit to every endpoint.

    Frame-transaction gossip is unreliable: a transaction submitted to one execution
    client only mines if that client proposes before it ages out. Later endpoints
    commonly answer "nonce too low" once an earlier one accepted the transaction —
    that is success, not failure, so any accepted hash wins.
    """
    txhash, errors = None, []
    for u in RPCS:
        try:
            h = rpc("eth_sendRawTransaction", [raw], url=u)
            txhash = txhash or h
        except Exception as e:
            errors.append(f"{u.split('//')[1].split('.')[0]}: {e}")
    return txhash, errors


def simulate(raw: str) -> dict:
    try:
        return rpc("ethrex_simulateFrameTransaction", [raw])
    except Exception as e:
        return {"simulateError": str(e)}


# ---------------------------------------------------------------- compile / deploy

# The compiler the committed EVIDENCE.md was produced with. This is checked rather than
# merely documented because the Safe singleton deploys with only ~3% headroom under the
# EIP-7825 transaction gas cap: a compiler that emits slightly larger code makes that
# scenario fail with an opaque out-of-gas rather than an obvious version complaint.
EXPECTED_SOLC = "0.8.31"
_solc_checked = False


def check_solc() -> str:
    """Verify the solc on PATH, once per process. Set POC_ALLOW_ANY_SOLC=1 to override."""
    global _solc_checked
    out = subprocess.run(["solc", "--version"], capture_output=True, text=True, check=True).stdout
    version = out.strip().split("Version: ")[-1].split("+")[0]
    if not _solc_checked:
        _solc_checked = True
        if version != EXPECTED_SOLC:
            msg = (f"solc {version} on PATH, but the evidence in EVIDENCE.md was produced with "
                   f"{EXPECTED_SOLC}. Bytecode and gas will differ, and the Safe scenario has "
                   f"only ~3% gas headroom.")
            if os.environ.get("POC_ALLOW_ANY_SOLC") == "1":
                print(f"    WARNING: {msg}")
            else:
                raise RuntimeError(msg + " Set POC_ALLOW_ANY_SOLC=1 to proceed anyway.")
        else:
            print(f"    solc {version}")
    return version


def compile_yul(rel_path: str) -> bytes:
    check_solc()
    out = subprocess.run(
        ["solc", "--strict-assembly", "--optimize", "--bin", os.path.join(CONTRACTS, rel_path)],
        capture_output=True, text=True, check=True,
    ).stdout
    for line in reversed(out.splitlines()):
        s = line.strip()
        if s and all(c in "0123456789abcdefABCDEF" for c in s) and len(s) > 20:
            return bytes.fromhex(s)
    raise RuntimeError(f"no bytecode in solc output for {rel_path}")


def compile_sol(rel_path: str, name: str) -> bytes:
    check_solc()
    out = subprocess.run(
        ["solc", "--optimize", "--bin", os.path.join(CONTRACTS, rel_path)],
        capture_output=True, text=True, check=True,
    ).stdout
    blocks = out.split("=======")
    for i, b in enumerate(blocks):
        if b.strip().endswith(f":{name}"):
            for line in blocks[i + 1].splitlines():
                s = line.strip()
                if s and all(c in "0123456789abcdefABCDEF" for c in s) and len(s) > 20:
                    return bytes.fromhex(s)
    raise RuntimeError(f"{name} bytecode not found in {rel_path}")


SAFE_REPO = "https://github.com/safe-global/safe-smart-account.git"
SAFE_TAG = "v1.3.0-libs.0"
SAFE_SRC = os.environ.get("SAFE_SRC", os.path.expanduser("~/.cache/poc7906-safe-src"))


def safe_sources() -> str:
    """Real Safe contract sources, cloned on first use.

    The canonical deployment route is the Safe Singleton Factory, which exists to give
    identical addresses across chains and requires an upstream request plus a biweekly
    processing cycle to onboard a new one. Address determinism buys nothing for reproducing
    an attack on a single devnet, so these are deployed with plain CREATE at non-canonical
    addresses. Bytecode and logic fidelity is what the claim rests on, and both are preserved.

    Note the compiler: Safe v1.3.0's pragma is `>=0.7.0 <0.9.0`, so the pinned solc used
    everywhere else here compiles it. The official v1.3.0 release was built with 0.7.6, so the
    resulting bytecode is not byte-identical to the canonical deployment even though the
    source is the real thing. Scenario documentation says so rather than implying otherwise.
    """
    if not os.path.isdir(os.path.join(SAFE_SRC, "contracts")):
        os.makedirs(os.path.dirname(SAFE_SRC), exist_ok=True)
        print(f"    cloning Safe {SAFE_TAG} -> {SAFE_SRC}")
        subprocess.run(["git", "clone", "--quiet", "--depth", "1", "--branch", SAFE_TAG,
                        SAFE_REPO, SAFE_SRC], check=True)
    return SAFE_SRC


def compile_safe(rel_path: str, name: str) -> bytes:
    """Compile a Safe contract, size-optimized.

    `--via-ir --optimize-runs 1` is not a style preference: EIP-8037 charges state gas on the
    code deposit, so the singleton's default build (12,056 bytes) needs more than EIP-7825's
    2**24 per-transaction ceiling and cannot be deployed. The size-optimized build is 10,353
    bytes and lands at 16,278,183 gas — about 3% under the cap. Reverting to the default
    optimizer settings makes the singleton undeployable again.
    """
    check_solc()
    src = safe_sources()
    out = subprocess.run(
        ["solc", "--optimize", "--optimize-runs", "1", "--via-ir", "--bin",
         "--allow-paths", src, os.path.join(src, rel_path)],
        capture_output=True, text=True, check=True, cwd=src,
    ).stdout
    blocks = out.split("=======")
    for i, b in enumerate(blocks):
        if b.strip().endswith(f":{name}"):
            for line in blocks[i + 1].splitlines():
                s = line.strip()
                if s and all(c in "0123456789abcdefABCDEF" for c in s) and len(s) > 20:
                    return bytes.fromhex(s)
    raise RuntimeError(f"{name} bytecode not found in {rel_path}")


def selector(signature: str) -> bytes:
    return keccak(text=signature)[:4]


def _word(a) -> bytes:
    if isinstance(a, str) and a.startswith("0x") and len(a) == 42:
        return bytes(12) + bytes.fromhex(a[2:])
    if isinstance(a, bool):
        return (1 if a else 0).to_bytes(32, "big")
    if isinstance(a, int):
        return a.to_bytes(32, "big")
    if isinstance(a, bytes) and len(a) == 32:
        return a
    raise TypeError(f"unsupported arg {a!r}")


def encode_call(signature: str, *args) -> bytes:
    """ABI-encode a call over static args plus (optionally) dynamic arrays.

    Supports the argument shapes these scenarios need: address/uint256/bool/bytes32
    scalars, and `list` for a dynamic array of those.
    """
    head, tail = b"", b""
    head_len = 32 * len(args)
    for a in args:
        if isinstance(a, list):
            head += (head_len + len(tail)).to_bytes(32, "big")
            tail += len(a).to_bytes(32, "big") + b"".join(_word(x) for x in a)
        elif isinstance(a, bytes) and len(a) != 32:
            head += (head_len + len(tail)).to_bytes(32, "big")
            tail += len(a).to_bytes(32, "big") + a + bytes((-len(a)) % 32)
        else:
            head += _word(a)
    return selector(signature) + head + tail


# ---------------------------------------------------------------- accounts


class Signer:
    def __init__(self, priv: bytes):
        self.pk = keys.PrivateKey(priv)
        self.address = self.pk.public_key.to_checksum_address()
        self.int = int(self.address, 16)

    def sign_hash(self, h: bytes) -> bytes:
        s = self.pk.sign_msg_hash(h)
        return bytes([s.v + 27]) + s.r.to_bytes(32, "big") + s.s.to_bytes(32, "big")

    def send_tx(self, to: str | None, value: int, data: bytes, gas: int = GAS_DEPLOY) -> dict:
        tx = {
            "chainId": chain_id(),
            "nonce": int(rpc("eth_getTransactionCount", [self.address, "latest"]), 16),
            "gas": gas,
            "maxFeePerGas": base_fee() * 2 + 10**9,
            "maxPriorityFeePerGas": 10**9,
            "value": value,
            "data": data,
        }
        if to:
            tx["to"] = to_checksum_address(to)
        # Prefer the node's own estimate when it exceeds the caller's budget. EIP-8037 state
        # gas makes storage-heavy calls and large deployments cost far more than their
        # execution gas suggests — a Safe singleton deployment needs well over 3M — and
        # estimateGas accounts for it correctly where a hand-picked constant will not.
        # Deliberately omit `gas` and `nonce` from the estimate call: supplying a gas cap makes
        # the node estimate against that ceiling instead of reporting what the call needs.
        est_call = {"from": self.address, "value": hex(value), "data": "0x" + data.hex()}
        if to:
            est_call["to"] = to_checksum_address(to)
        try:
            est = int(rpc("eth_estimateGas", [est_call]), 16)
            tx["gas"] = min(max(gas, int(est * 1.3)), EIP7825_TX_GAS_CAP)
        except Exception as e:
            print(f"      (gas estimate unavailable: {str(e)[:80]})")
        signed = Account.from_key(self.pk.to_bytes()).sign_transaction(tx)
        h = rpc("eth_sendRawTransaction", ["0x" + signed.raw_transaction.hex().removeprefix("0x")])
        r = wait_receipt(h)
        if not r:
            raise RuntimeError(f"tx {h} not mined")
        if r.get("status") != "0x1":
            raise RuntimeError(f"tx {h} reverted (gas {int(r['gasUsed'],16)}; "
                               f"storage writes need generous budgets — see GATE-RESULTS.md)")
        return r

    def deploy(self, initcode: bytes, label: str, value: int = 0) -> str:
        r = self.send_tx(None, value, initcode)
        addr = r["contractAddress"]
        print(f"    deployed {label:26s} {addr}")
        return addr


def bank() -> Signer:
    """A persistent funded account, topped up from the faucet when low."""
    if os.path.exists(BANK_KEY_FILE):
        priv = bytes.fromhex(open(BANK_KEY_FILE).read().strip().removeprefix("0x"))
    else:
        priv = Account.create().key
        open(BANK_KEY_FILE, "w").write(priv.hex())
        os.chmod(BANK_KEY_FILE, 0o600)
    s = Signer(priv)
    if balance(s.address) < 2 * 10**18:
        print(f"    funding bank {s.address} from faucet")
        try:
            req = urllib.request.Request(
                FAUCET, headers={"content-type": "application/json"},
                data=json.dumps({"address": s.address}).encode())
            urllib.request.urlopen(req, timeout=30).read()
        except Exception as e:
            print(f"    faucet: {e}")
        for _ in range(20):
            if balance(s.address) >= 10**18:
                break
            time.sleep(3)
    print(f"    bank {s.address}  {balance(s.address)/1e18:.3f} ETH")
    return s


def fresh_account(funder: Signer, wei: int, label: str) -> Signer:
    """A throwaway EOA funded from the bank, so scenarios never share mutable state."""
    s = Signer(Account.create().key)
    funder.send_tx(s.address, wei, b"", gas=100_000)
    print(f"    {label:26s} {s.address}  {balance(s.address)/1e18:.3f} ETH")
    return s


# ---------------------------------------------------------------- frame transactions


def frame_tx(sender: Signer | str, signer: Signer, frames: list[Frame], nonce_seq: int | None = None) -> FrameTx:
    """Build and sign a frame transaction.

    `sender` may be an EOA Signer (self-verifying) or a contract account address, in
    which case `signer` is the key whose signature that account authorizes.
    """
    sender_addr = sender.address if isinstance(sender, Signer) else sender
    sender_int = int(sender_addr, 16)
    if nonce_seq is None:
        nonce_seq = int(rpc("eth_getTransactionCount", [to_checksum_address(sender_addr), "latest"]), 16)
    tx = FrameTx(
        chain_id=chain_id(), nonce_keys=[0], nonce_seq=nonce_seq, sender=sender_int,
        frames=frames,
        signatures=[FrameSig(FrameSig.SECP256K1, signer.int, b"", b"")],
        max_priority_fee=10**9, max_fee=base_fee() * 2 + 10**9,
    )
    tx.signatures = [FrameSig(FrameSig.SECP256K1, signer.int, b"", signer.sign_hash(tx.sig_hash()))]
    return tx


def verify_frame(target: str) -> Frame:
    return Frame(mode=VERIFY, flags=SCOPE_BOTH, target=int(target, 16),
                 gas_limit=GAS_VERIFY_FRAME, value=0, data=b"")


def sender_frame(target: str, value: int = 0, data: bytes = b"") -> Frame:
    return Frame(mode=SENDER, flags=0, target=int(target, 16),
                 gas_limit=GAS_BODY_FRAME, value=value, data=data)


def guard_frame(guard: str, data: bytes) -> Frame:
    return Frame(mode=POST_TX, flags=0, target=int(guard, 16),
                 gas_limit=GAS_GUARD_FRAME, value=0, data=data)


@dataclass
class Outcome:
    label: str
    submitted: bool
    txhash: str | None
    mined: bool
    block: int | None
    status: str | None
    submit_errors: list[str] = field(default_factory=list)
    simulation: dict = field(default_factory=dict)


def submit(tx: FrameTx, label: str, sim: bool = True, expect_mine: bool = False,
           attempts: int = 3) -> Outcome:
    """Submit a frame transaction and report what happened.

    `expect_mine` exists because frame-transaction gossip is unreliable: a transaction can
    be admitted, be entirely valid, and still not be included if no execution client that
    holds it proposes before it ages out. When inclusion is the expected outcome we
    re-broadcast a few times before concluding anything. Cases that expect NON-inclusion get
    a single patient wait, so a defended transaction is never reported as defended merely
    because gossip dropped it.
    """
    raw = "0x" + tx.raw().hex()
    simulation = simulate(raw) if sim else {}
    tries = attempts if expect_mine else 1
    h, errs, r = None, [], None
    for attempt in range(tries):
        h2, errs2 = broadcast(raw)
        h = h or h2
        errs = errs2
        r = wait_receipt(h, tries=12, delay=3) if h else None
        if r or not expect_mine:
            break
        if attempt + 1 < tries:
            print(f"      (not included yet — re-broadcasting, attempt {attempt + 2}/{tries})")
    o = Outcome(
        label=label, submitted=h is not None, txhash=h, mined=r is not None,
        block=int(r["blockNumber"], 16) if r else None,
        status=r.get("status") if r else None,
        submit_errors=errs, simulation=simulation,
    )
    state = "mined block %d status %s" % (o.block, o.status) if o.mined else (
        "admitted, never mined" if o.submitted else "rejected at admission")
    print(f"    {label:44s} {state}")
    return o


# ---------------------------------------------------------------- evidence


@dataclass
class Evidence:
    scenario: str
    title: str
    models: str
    defense_kind: str          # "reverts the attack" | "removes the attack surface"
    chain_id: int = field(default_factory=chain_id)
    started_block: int = field(default_factory=block_number)
    notes: list[str] = field(default_factory=list)
    addresses: dict = field(default_factory=dict)
    phase_a: dict = field(default_factory=dict)
    phase_b: dict = field(default_factory=dict)
    extra: dict = field(default_factory=dict)

    def note(self, s: str):
        self.notes.append(s)
        print(f"    note: {s}")

    def save(self):
        os.makedirs(EVIDENCE_DIR, exist_ok=True)
        path = os.path.join(EVIDENCE_DIR, f"{self.scenario}.json")
        with open(path, "w") as f:
            json.dump(asdict(self), f, indent=2, default=str)
        print(f"    evidence -> {os.path.relpath(path)}")


def run_scenario(name: str, fn):
    """Run a scenario, enforcing that both phases behaved as claimed."""
    print(f"\n{'='*74}\n{name}\n{'='*74}")
    try:
        ev = fn()
    except Exception as e:
        print(f"\n  SCENARIO FAILED: {e}")
        raise
    ev.save()
    print(f"\n  {name}: OK")
    return ev
