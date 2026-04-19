#!/usr/bin/env python3
"""Test a sponsored frame tx using the CanonicalPaymaster.

The CanonicalPaymaster requires the owner's secp256k1 signature over the
frame tx sig_hash as VERIFY frame calldata: r(32) || s(32) || v(1) = 65 bytes.

Three frames:
  Frame 0 (VERIFY, scope=1): Sender verifies themselves
  Frame 1 (VERIFY, scope=2): CanonicalPaymaster verifies owner sig, APPROVE as payer
  Frame 2 (SENDER):          Transfer 0.01 ETH

Usage:
    python3 test-canonical-paymaster.py \
        --rpc-url http://127.0.0.1:32003 \
        --sender-key <sender-private-key> \
        --owner-key <paymaster-owner-private-key> \
        --paymaster <paymaster-address>
"""
import argparse
import json
import sys
import time
import urllib.request

from eth_keys import keys as eth_keys
from eth_hash.auto import keccak


# RLP helpers
def rlp_encode_uint(v):
    if v == 0: return b"\x80"
    b = v.to_bytes((v.bit_length() + 7) // 8, "big")
    return b if len(b) == 1 and b[0] < 0x80 else bytes([0x80 + len(b)]) + b

def rlp_encode_bytes(d):
    if not d: return b"\x80"
    if len(d) == 1 and d[0] < 0x80: return d
    if len(d) <= 55: return bytes([0x80 + len(d)]) + d
    lb = len(d).to_bytes((len(d).bit_length() + 7) // 8, "big")
    return bytes([0xB7 + len(lb)]) + lb + d

def rlp_encode_address(a):
    assert len(a) == 20
    return bytes([0x80 + 20]) + a

def rlp_encode_list(items):
    p = b"".join(items)
    if len(p) <= 55: return bytes([0xC0 + len(p)]) + p
    lb = len(p).to_bytes((len(p).bit_length() + 7) // 8, "big")
    return bytes([0xF7 + len(lb)]) + lb + p

def encode_frame(mode, flags, target, gas_limit, data):
    # Post-spec-update: 5-field RLP [mode, flags, target, gas_limit, data]
    return rlp_encode_list([rlp_encode_uint(mode), rlp_encode_uint(flags),
                            rlp_encode_address(target),
                            rlp_encode_uint(gas_limit), rlp_encode_bytes(data)])

def build_payload(chain_id, nonce, sender, frames_rlp, mpf, mf):
    return rlp_encode_list([rlp_encode_uint(chain_id), rlp_encode_uint(nonce),
        rlp_encode_address(sender), rlp_encode_list(frames_rlp),
        rlp_encode_uint(mpf), rlp_encode_uint(mf), rlp_encode_uint(0), rlp_encode_list([])])

def compute_sig_hash(chain_id, nonce, sender, frames, mpf, mf):
    elided = [encode_frame(f["mode"], f["flags"], f["target"], f["gas_limit"],
              b"" if f["exec_mode"] == 1 else f["data"]) for f in frames]
    return keccak(b"\x06" + build_payload(chain_id, nonce, sender, elided, mpf, mf))

def rpc(url, method, params):
    req = urllib.request.Request(url,
        data=json.dumps({"jsonrpc":"2.0","method":method,"params":params,"id":1}).encode(),
        headers={"Content-Type":"application/json"})
    with urllib.request.urlopen(req, timeout=30) as resp: return json.loads(resp.read())


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--rpc-url", required=True)
    parser.add_argument("--sender-key", required=True, help="Sender's private key")
    parser.add_argument("--owner-key", required=True, help="Paymaster owner's private key (signs the sponsorship)")
    parser.add_argument("--paymaster", required=True, help="CanonicalPaymaster address")
    parser.add_argument("--recipient", default="0x0000000000000000000000000000000000C0FFEE")
    args = parser.parse_args()

    sender_pk = eth_keys.PrivateKey(bytes.fromhex(args.sender_key))
    owner_pk = eth_keys.PrivateKey(bytes.fromhex(args.owner_key))
    sender_addr = bytes.fromhex(sender_pk.public_key.to_checksum_address()[2:])
    paymaster_addr = bytes.fromhex(args.paymaster[2:])
    recipient_addr = bytes.fromhex(args.recipient[2:])

    chain_id = int(rpc(args.rpc_url, "eth_chainId", [])["result"], 16)
    # Use "pending" rather than "latest" so a tx that is mined but not yet
    # reflected in the canonical head does not produce a stale nonce and a
    # "Nonce mismatch" rejection at submission time.
    nonce = int(rpc(args.rpc_url, "eth_getTransactionCount", ["0x" + sender_addr.hex(), "pending"])["result"], 16)
    gas_price = int(rpc(args.rpc_url, "eth_gasPrice", [])["result"], 16)

    sender_bal = int(rpc(args.rpc_url, "eth_getBalance", ["0x" + sender_addr.hex(), "latest"])["result"], 16)
    pm_bal = int(rpc(args.rpc_url, "eth_getBalance", [args.paymaster, "latest"])["result"], 16)

    print("=== CanonicalPaymaster Sponsored TX Test ===")
    print(f"Sender:          0x{sender_addr.hex()}")
    print(f"Sender balance:  {sender_bal / 10**18:.4f} ETH")
    print(f"Owner:           {owner_pk.public_key.to_checksum_address()}")
    print(f"Paymaster:       {args.paymaster}")
    print(f"Paymaster bal:   {pm_bal / 10**18:.4f} ETH")
    print(f"Recipient:       {args.recipient}")
    print(f"Nonce:           {nonce}")
    print()

    mpf = 1_000_000_000  # 1 gwei
    mf = max(gas_price * 2, 10_000_000_000)
    transfer_value = 10**16  # 0.01 ETH

    # Frame 0: VERIFY(scope=1) — sender verifies with ECDSA
    # Frame 1: VERIFY(scope=2) — CanonicalPaymaster verifies owner sig
    # Frame 2: SENDER — transfer ETH
    sender_call = rlp_encode_list([rlp_encode_address(recipient_addr),
                                   rlp_encode_uint(transfer_value), rlp_encode_bytes(b"")])
    sender_data = rlp_encode_list([sender_call])

    # Post-spec-update: mode/flags split.
    # Scope bitmask: 0x01=PAYMENT, 0x02=EXECUTION.
    # Frame 0: sender authorizes EXECUTION (scope=0x02) via self-ECDSA
    # Frame 1: paymaster authorizes PAYMENT  (scope=0x01)
    # Frame 2: SENDER-mode, no flags
    frames = [
        {"mode": 1, "flags": 0x02, "exec_mode": 1, "target": sender_addr,    "gas_limit": 100_000, "data": b""},
        {"mode": 1, "flags": 0x01, "exec_mode": 1, "target": paymaster_addr, "gas_limit": 200_000, "data": b""},
        {"mode": 2, "flags": 0x00, "exec_mode": 2, "target": sender_addr,    "gas_limit": 100_000, "data": sender_data},
    ]

    print("Frames:")
    print("  [0] VERIFY(scope=1) → sender EOA default code (ECDSA)")
    print("  [1] VERIFY(scope=2) → CanonicalPaymaster (owner signs sig_hash)")
    print("  [2] SENDER          → transfer 0.01 ETH")
    print()

    # Compute sig_hash (VERIFY data elided)
    sig_hash = compute_sig_hash(chain_id, nonce, sender_addr, frames, mpf, mf)
    print(f"Sig hash: 0x{sig_hash.hex()}")

    # Sign for Frame 0: sender signs with default EOA format [type=0x00, v, r, s] = 66 bytes
    sender_sig = sender_pk.sign_msg_hash(sig_hash)
    frames[0]["data"] = bytes([0x00, sender_sig.v + 27]) + sender_sig.r.to_bytes(32, "big") + sender_sig.s.to_bytes(32, "big")

    # Sign for Frame 1: owner signs with CanonicalPaymaster format [r, s, v] = 65 bytes
    owner_sig = owner_pk.sign_msg_hash(sig_hash)
    frames[1]["data"] = owner_sig.r.to_bytes(32, "big") + owner_sig.s.to_bytes(32, "big") + bytes([owner_sig.v + 27])

    print(f"Sender sig: v={sender_sig.v + 27}")
    print(f"Owner sig:  v={owner_sig.v + 27}")
    print()

    # Build raw tx
    frames_rlp = [encode_frame(f["mode"], f["flags"], f["target"], f["gas_limit"], f["data"]) for f in frames]
    payload = build_payload(chain_id, nonce, sender_addr, frames_rlp, mpf, mf)
    raw_tx = "0x" + (b"\x06" + payload).hex()
    print(f"Tx size: {len(raw_tx)//2} bytes")

    # Record balances
    sender_before = sender_bal
    pm_before = pm_bal
    recip_before = int(rpc(args.rpc_url, "eth_getBalance", [args.recipient, "latest"])["result"], 16)

    # Send
    print("\nSending...")
    result = rpc(args.rpc_url, "eth_sendRawTransaction", [raw_tx])
    if "error" in result:
        print(f"ERROR: {result['error']}")
        sys.exit(1)

    tx_hash = result["result"]
    print(f"TX HASH: {tx_hash}\n")

    # Wait for receipt
    for attempt in range(30):
        time.sleep(2)
        r = rpc(args.rpc_url, "eth_getTransactionReceipt", [tx_hash])
        if r.get("result"):
            receipt = r["result"]
            status = receipt.get("status")
            print("=== CANONICAL PAYMASTER RECEIPT ===")
            print(f"Status:     {'SUCCESS' if status == '0x1' else 'FAILED (' + str(status) + ')'}")
            print(f"Block:      {int(receipt.get('blockNumber', '0x0'), 16)}")
            print(f"Gas used:   {int(receipt.get('gasUsed', '0x0'), 16)}")
            print(f"Payer:      {receipt.get('payer', 'N/A')}")

            frs = receipt.get("frameReceipts", [])
            labels = ["VERIFY(sender)", "VERIFY(payer) ", "SENDER        "]
            for i, fr in enumerate(frs):
                s = "OK" if fr.get("status") in (True, "0x1", 1) else "FAIL"
                g = fr.get("gasUsed", "?")
                if isinstance(g, str) and g.startswith("0x"): g = int(g, 16)
                print(f"  Frame {i} [{labels[i] if i < len(labels) else '???'}]: {s}, gas={g}")

            sender_after = int(rpc(args.rpc_url, "eth_getBalance", ["0x" + sender_addr.hex(), "latest"])["result"], 16)
            pm_after = int(rpc(args.rpc_url, "eth_getBalance", [args.paymaster, "latest"])["result"], 16)
            recip_after = int(rpc(args.rpc_url, "eth_getBalance", [args.recipient, "latest"])["result"], 16)

            print("\n=== BALANCE CHANGES ===")
            sd = sender_after - sender_before
            pd = pm_after - pm_before
            rd = recip_after - recip_before
            print(f"Sender:    {sd/10**18:+.6f} ETH {'(only transfer, no gas!)' if sd >= -transfer_value else '(paid gas!)'}")
            print(f"Paymaster: {pd/10**18:+.6f} ETH {'(PAID GAS)' if pd < 0 else ''}")
            print(f"Recipient: {rd/10**18:+.6f} ETH")

            is_sponsored = receipt.get("payer", "").lower() != ("0x" + sender_addr.hex()).lower()
            print()
            if is_sponsored and status == "0x1":
                print("*** CANONICAL PAYMASTER SPONSORSHIP CONFIRMED ***")
            elif status == "0x1":
                print("Tx succeeded but sender paid gas (self-sponsored)")
            else:
                print("Transaction FAILED")
            print("===================================")
            return
        if attempt % 5 == 4:
            print(f"  ... waiting ({attempt+1}/30)")

    print("Timed out")
    sys.exit(1)

if __name__ == "__main__":
    main()
