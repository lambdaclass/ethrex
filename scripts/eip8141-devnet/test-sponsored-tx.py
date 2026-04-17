#!/usr/bin/env python3
"""Send a SPONSORED frame transaction on the EIP-8141 devnet.

Three frames:
  Frame 0 (VERIFY, scope=1): Sender verifies themselves (APPROVE sender only)
  Frame 1 (VERIFY, scope=2): GasSponsor verifies and approves as PAYER
  Frame 2 (SENDER):          Actual operation — transfer 0.01 ETH

The sender pays NO gas — the GasSponsor contract pays.

Usage:
    python3 test-sponsored-tx.py --rpc-url http://127.0.0.1:32003 \
        --private-key <FUNDED_PRIVATE_KEY>
"""
import argparse
import json
import sys
import time
import urllib.request

from eth_keys import keys as eth_keys
from eth_hash.auto import keccak


# ---------------------------------------------------------------------------
# RLP encoding
# ---------------------------------------------------------------------------

def rlp_encode_uint(value: int) -> bytes:
    if value == 0:
        return b"\x80"
    b = value.to_bytes((value.bit_length() + 7) // 8, "big")
    if len(b) == 1 and b[0] < 0x80:
        return b
    return bytes([0x80 + len(b)]) + b

def rlp_encode_bytes(data: bytes) -> bytes:
    if len(data) == 0:
        return b"\x80"
    if len(data) == 1 and data[0] < 0x80:
        return data
    if len(data) <= 55:
        return bytes([0x80 + len(data)]) + data
    len_bytes = len(data).to_bytes((len(data).bit_length() + 7) // 8, "big")
    return bytes([0xB7 + len(len_bytes)]) + len_bytes + data

def rlp_encode_address(addr: bytes) -> bytes:
    assert len(addr) == 20
    return bytes([0x80 + 20]) + addr

def rlp_encode_list(items: list[bytes]) -> bytes:
    payload = b"".join(items)
    if len(payload) <= 55:
        return bytes([0xC0 + len(payload)]) + payload
    len_bytes = len(payload).to_bytes((len(payload).bit_length() + 7) // 8, "big")
    return bytes([0xF7 + len(len_bytes)]) + len_bytes + payload


# ---------------------------------------------------------------------------
# Frame TX
# ---------------------------------------------------------------------------

def encode_frame(mode: int, target: bytes, gas_limit: int, data: bytes) -> bytes:
    return rlp_encode_list([
        rlp_encode_uint(mode),
        rlp_encode_address(target),
        rlp_encode_uint(gas_limit),
        rlp_encode_bytes(data),
    ])

def build_payload(chain_id, nonce, sender, frames_rlp, max_priority_fee, max_fee):
    return rlp_encode_list([
        rlp_encode_uint(chain_id),
        rlp_encode_uint(nonce),
        rlp_encode_address(sender),
        rlp_encode_list(frames_rlp),
        rlp_encode_uint(max_priority_fee),
        rlp_encode_uint(max_fee),
        rlp_encode_uint(0),
        rlp_encode_list([]),
    ])

def compute_sig_hash(chain_id, nonce, sender, frames, max_priority_fee, max_fee):
    elided = []
    for f in frames:
        if f["exec_mode"] == 1:  # VERIFY
            elided.append(encode_frame(f["mode"], f["target"], f["gas_limit"], b""))
        else:
            elided.append(encode_frame(f["mode"], f["target"], f["gas_limit"], f["data"]))
    payload = build_payload(chain_id, nonce, sender, elided, max_priority_fee, max_fee)
    return keccak(b"\x06" + payload)

def rpc(url, method, params):
    req = urllib.request.Request(
        url,
        data=json.dumps({"jsonrpc": "2.0", "method": method, "params": params, "id": 1}).encode(),
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        return json.loads(resp.read())


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--rpc-url", required=True)
    parser.add_argument("--private-key", required=True)
    parser.add_argument("--sponsor", default="0xb4b46bdaa835f8e4b4d8e208b6559cd267851051")
    parser.add_argument("--recipient", default="0x0000000000000000000000000000000000C0FFEE")
    args = parser.parse_args()

    pk = eth_keys.PrivateKey(bytes.fromhex(args.private_key))
    sender_addr = bytes.fromhex(pk.public_key.to_checksum_address()[2:])
    sponsor_addr = bytes.fromhex(args.sponsor[2:])
    recipient_addr = bytes.fromhex(args.recipient[2:])

    chain_id = int(rpc(args.rpc_url, "eth_chainId", [])["result"], 16)
    nonce = int(rpc(args.rpc_url, "eth_getTransactionCount", ["0x" + sender_addr.hex(), "latest"])["result"], 16)
    gas_price = int(rpc(args.rpc_url, "eth_gasPrice", [])["result"], 16)

    sender_balance = int(rpc(args.rpc_url, "eth_getBalance", ["0x" + sender_addr.hex(), "latest"])["result"], 16)
    sponsor_balance = int(rpc(args.rpc_url, "eth_getBalance", [args.sponsor, "latest"])["result"], 16)

    print("=== Sponsored Frame Transaction Test ===")
    print(f"Chain ID:        {chain_id}")
    print(f"Sender:          0x{sender_addr.hex()}")
    print(f"Sender balance:  {sender_balance / 10**18:.4f} ETH")
    print(f"Sponsor:         {args.sponsor}")
    print(f"Sponsor balance: {sponsor_balance / 10**18:.4f} ETH")
    print(f"Recipient:       {args.recipient}")
    print(f"Nonce:           {nonce}")
    print()

    max_priority_fee = 1_000_000_000  # 1 gwei
    max_fee = max(gas_price * 2, 10_000_000_000)

    # ── Frame 0: VERIFY, scope=1 (sender only) ──
    # Target = sender (default EOA code verifies ECDSA)
    # mode = 1 (VERIFY) | (1 << 8) (scope=1: sender only)
    verify_sender_mode = 1 | (1 << 8)  # 0x101

    # ── Frame 1: VERIFY, scope=2 (payer only) ──
    # Target = GasSponsor contract
    # mode = 1 (VERIFY) | (2 << 8) (scope=2: payer only)
    # data = verify() selector = 0xfc735e99
    verify_payer_mode = 1 | (2 << 8)  # 0x201
    sponsor_calldata = bytes.fromhex("fc735e99")

    # ── Frame 2: SENDER ──
    # Target = sender (default EOA code dispatches subcalls)
    # data = RLP [[recipient, value, calldata]]
    sender_mode = 2
    transfer_value = 10**16  # 0.01 ETH
    sender_call = rlp_encode_list([
        rlp_encode_address(recipient_addr),
        rlp_encode_uint(transfer_value),
        rlp_encode_bytes(b""),
    ])
    sender_data = rlp_encode_list([sender_call])

    frames = [
        {"mode": verify_sender_mode, "exec_mode": 1, "target": sender_addr, "gas_limit": 100_000, "data": b""},
        {"mode": verify_payer_mode,  "exec_mode": 1, "target": sponsor_addr, "gas_limit": 200_000, "data": sponsor_calldata},
        {"mode": sender_mode,        "exec_mode": 2, "target": sender_addr, "gas_limit": 100_000, "data": sender_data},
    ]

    print("Frames:")
    print("  [0] VERIFY (scope=1) → sender EOA: verify signature, APPROVE as sender")
    print("  [1] VERIFY (scope=2) → GasSponsor: check token balance, APPROVE as payer")
    print("  [2] SENDER           → transfer 0.01 ETH to recipient")
    print()

    # Sign
    sig_hash = compute_sig_hash(chain_id, nonce, sender_addr, frames, max_priority_fee, max_fee)
    print(f"Sig hash: 0x{sig_hash.hex()}")

    signed = pk.sign_msg_hash(sig_hash)
    v = signed.v + 27
    r = signed.r.to_bytes(32, "big")
    s = signed.s.to_bytes(32, "big")

    # Fill VERIFY frame 0 data: [type=0x00, v, r, s]
    frames[0]["data"] = bytes([0x00, v]) + r + s

    # Build raw tx
    frames_rlp = [encode_frame(f["mode"], f["target"], f["gas_limit"], f["data"]) for f in frames]
    payload = build_payload(chain_id, nonce, sender_addr, frames_rlp, max_priority_fee, max_fee)
    raw_tx = "0x" + (b"\x06" + payload).hex()

    print(f"Tx size:  {len(raw_tx)//2} bytes")
    print()

    # Record balances before
    sender_before = sender_balance
    sponsor_before = sponsor_balance
    recipient_before = int(rpc(args.rpc_url, "eth_getBalance", [args.recipient, "latest"])["result"], 16)

    # Send
    print("Sending sponsored frame transaction...")
    result = rpc(args.rpc_url, "eth_sendRawTransaction", [raw_tx])
    if "error" in result:
        print(f"ERROR: {result['error']}")
        sys.exit(1)

    tx_hash = result["result"]
    print(f"TX HASH: {tx_hash}")
    print()

    # Wait for receipt
    print("Waiting for receipt...")
    for attempt in range(30):
        time.sleep(2)
        r = rpc(args.rpc_url, "eth_getTransactionReceipt", [tx_hash])
        if r.get("result"):
            receipt = r["result"]
            status = receipt.get("status")
            print()
            print("=== SPONSORED FRAME TX RECEIPT ===")
            print(f"Status:     {'SUCCESS' if status == '0x1' else 'FAILED (' + str(status) + ')'}")
            print(f"Block:      {int(receipt.get('blockNumber', '0x0'), 16)}")
            print(f"Gas used:   {int(receipt.get('gasUsed', '0x0'), 16)}")
            print(f"Payer:      {receipt.get('payer', 'N/A')}")

            frs = receipt.get("frameReceipts", [])
            labels = ["VERIFY(sender)", "VERIFY(payer) ", "SENDER        "]
            for i, fr in enumerate(frs):
                s = "OK" if fr.get("status") in (True, "0x1", 1) else "FAIL"
                g = fr.get("gasUsed", "?")
                if isinstance(g, str) and g.startswith("0x"):
                    g = int(g, 16)
                print(f"  Frame {i} [{labels[i] if i < len(labels) else '???'}]: {s}, gas={g}")

            # Check balances after
            sender_after = int(rpc(args.rpc_url, "eth_getBalance", ["0x" + sender_addr.hex(), "latest"])["result"], 16)
            sponsor_after = int(rpc(args.rpc_url, "eth_getBalance", [args.sponsor, "latest"])["result"], 16)
            recipient_after = int(rpc(args.rpc_url, "eth_getBalance", [args.recipient, "latest"])["result"], 16)

            print()
            print("=== BALANCE CHANGES ===")
            sender_diff = sender_after - sender_before
            sponsor_diff = sponsor_after - sponsor_before
            recipient_diff = recipient_after - recipient_before
            print(f"Sender:    {sender_diff/10**18:+.6f} ETH {'(paid gas!)' if sender_diff < -transfer_value else '(only transfer, no gas!)'}")
            print(f"Sponsor:   {sponsor_diff/10**18:+.6f} ETH {'(PAID GAS)' if sponsor_diff < 0 else ''}")
            print(f"Recipient: {recipient_diff/10**18:+.6f} ETH")

            is_sponsored = receipt.get("payer", "").lower() != ("0x" + sender_addr.hex()).lower()
            print()
            if is_sponsored and status == "0x1":
                print("*** SPONSORSHIP CONFIRMED: Gas paid by sponsor, not sender! ***")
            elif status == "0x1":
                print("Transaction succeeded but sender paid gas (self-sponsored)")
            else:
                print("Transaction FAILED")
            print("==================================")
            return

        if attempt % 5 == 4:
            print(f"  ... waiting ({attempt+1}/30)")

    print("Timed out")
    sys.exit(1)


if __name__ == "__main__":
    main()
