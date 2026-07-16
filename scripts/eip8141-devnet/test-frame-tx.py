#!/usr/bin/env python3
"""Submit a self-verified EIP-8141 frame transaction (type 0x06) and confirm it mines.

Current wire format (frame tuple is a 6-element list, plus a top-level `signatures`
list — matches ethrex `FrameTransaction`/`Frame`/`FrameSignature` RLP):

    0x06 || rlp([chain_id, nonce, sender, frames, signatures,
                 max_priority_fee_per_gas, max_fee_per_gas,
                 max_fee_per_blob_gas, blob_versioned_hashes])
    frame     = [mode, flags, target, gas_limit, value, data]
    signature = [scheme, signer, msg, signature]   # scheme 0 = secp256k1, sig = v||r||s (v in {27,28})

The "self-verified transfer" shape (no paymaster needed — sender pays its own gas):

    Frame 0: VERIFY (mode=1), flags=3 (allow APPROVE scope EXECUTION_AND_PAYMENT), target=null(=sender)
             -> the sender EOA's built-in default code matches the outer secp256k1 signature
                (signer == sender, empty msg, over sig_hash) and calls APPROVE(scope=3),
                approving the sender as both executor and payer.
    Frame 1: SENDER (mode=2), flags=0, target=recipient, value=<amount>
             -> default code performs the value transfer (logged via EIP-7708).

Constraint: the validation-prefix gas budget — sum of VERIFY-frame gas_limits + signature
verification cost (2800 per secp256k1 sig) — must stay <= MAX_VERIFY_GAS (100_000), so keep
--verify-gas comfortably below that.

Usage:
    python3 test-frame-tx.py --rpc-url http://127.0.0.1:8545 --private-key <FUNDED_KEY>
"""
import argparse, json, time, urllib.request
from eth_keys import keys as eth_keys
from eth_hash.auto import keccak


# --- minimal RLP ---
def _len(off, n):
    if n < 56:
        return bytes([off + n])
    lb = n.to_bytes((n.bit_length() + 7) // 8, "big")
    return bytes([off + 55 + len(lb)]) + lb

def rb(b):  # rlp of a byte string
    return b if (len(b) == 1 and b[0] < 0x80) else _len(0x80, len(b)) + b

def rl(items):  # rlp of a list of already-encoded items
    body = b"".join(items)
    return _len(0xC0, len(body)) + body

def ui(n):  # rlp of an unsigned integer (minimal big-endian)
    return rb(b"" if n == 0 else n.to_bytes((n.bit_length() + 7) // 8, "big"))


def frame(mode, flags, target, gas, value, data):
    tgt = rb(target) if target is not None else rb(b"")  # None -> RLP null -> resolves to sender
    return rl([ui(mode), ui(flags), tgt, ui(gas), ui(value), rb(data)])

def sig(scheme, signer, msg, signature):
    return rl([ui(scheme), rb(signer), rb(msg), rb(signature)])

def body(cid, nonce, sender, frames, sigs, mpf, mf):
    return rl([ui(cid), ui(nonce), rb(sender), rl(frames), rl(sigs),
               ui(mpf), ui(mf), ui(0), rl([])])


def rpc(url, method, params):
    req = urllib.request.Request(
        url,
        data=json.dumps({"jsonrpc": "2.0", "method": method, "params": params, "id": 1}).encode(),
        headers={"Content-Type": "application/json"},
    )
    return json.loads(urllib.request.urlopen(req, timeout=30).read())


def main():
    ap = argparse.ArgumentParser(description="Submit a self-verified EIP-8141 frame transaction")
    ap.add_argument("--rpc-url", required=True, help="e.g. http://127.0.0.1:8545")
    ap.add_argument("--private-key", required=True, help="funded sender key (hex)")
    ap.add_argument("--recipient", default="0x00000000000000000000000000000000C0FFEE00")
    ap.add_argument("--value", default=str(10**15), help="wei to transfer (default 0.001 ETH)")
    ap.add_argument("--verify-gas", type=int, default=50_000, help="VERIFY-frame gas (prefix budget + 2800 sig cost must be <= 100000)")
    ap.add_argument("--sender-gas", type=int, default=100_000, help="SENDER-frame gas")
    ap.add_argument("--max-fee", type=int, default=2_000_000_000)
    ap.add_argument("--max-priority-fee", type=int, default=1_000_000_000)
    a = ap.parse_args()

    pk = eth_keys.PrivateKey(bytes.fromhex(a.private_key.removeprefix("0x")))
    sender = pk.public_key.to_canonical_address()
    recipient = bytes.fromhex(a.recipient.removeprefix("0x"))
    value = int(a.value)

    cid = int(rpc(a.rpc_url, "eth_chainId", [])["result"], 16)
    nonce = int(rpc(a.rpc_url, "eth_getTransactionCount", ["0x" + sender.hex(), "latest"])["result"], 16)

    frames = [
        frame(1, 3, None, a.verify_gas, 0, b""),            # VERIFY self (scope EXECUTION_AND_PAYMENT)
        frame(2, 0, recipient, a.sender_gas, value, b""),   # SENDER transfer
    ]
    # sig_hash elides the (empty-msg) signature bytes -> compute over an empty-signature placeholder.
    sig_hash = keccak(b"\x06" + body(cid, nonce, sender, frames, [sig(0, sender, b"", b"")],
                                     a.max_priority_fee, a.max_fee))
    s = pk.sign_msg_hash(sig_hash)
    sigb = bytes([s.v + 27]) + s.r.to_bytes(32, "big") + s.s.to_bytes(32, "big")  # v||r||s, v in {27,28}
    raw = "0x06" + body(cid, nonce, sender, frames, [sig(0, sender, b"", sigb)],
                        a.max_priority_fee, a.max_fee).hex()

    print(f"sender=0x{sender.hex()} nonce={nonce} chain_id={cid} sig_hash=0x{sig_hash.hex()}")
    res = rpc(a.rpc_url, "eth_sendRawTransaction", [raw])
    print("eth_sendRawTransaction:", json.dumps(res))
    txh = res.get("result")
    if not txh:
        raise SystemExit(1)
    for _ in range(25):
        r = rpc(a.rpc_url, "eth_getTransactionReceipt", [txh]).get("result")
        if r:
            print("RECEIPT:", json.dumps(r, indent=2))
            raise SystemExit(0 if r.get("status") == "0x1" else 2)
        time.sleep(2)
    print("timed out waiting for receipt")
    raise SystemExit(3)


if __name__ == "__main__":
    main()
