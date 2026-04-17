#!/usr/bin/env python3
"""Send a self-verified frame transaction on the EIP-8141 devnet.

This constructs and sends a minimal frame tx:
  Frame 0 (VERIFY): Default EOA code verifies secp256k1 signature, APPROVE(scope=3)
  Frame 1 (SENDER): Transfer 0.01 ETH to a recipient

Usage:
    python3 test-frame-tx.py --rpc-url http://127.0.0.1:32003 \
        --private-key <FUNDED_PRIVATE_KEY>
"""
import argparse
import json
import sys
import urllib.request

from eth_account import Account
from eth_keys import keys as eth_keys
from eth_hash.auto import keccak

# ---------------------------------------------------------------------------
# RLP encoding helpers (minimal, no external dep)
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
    """Address is always 20 bytes, encode as bytes."""
    assert len(addr) == 20
    return bytes([0x80 + 20]) + addr


def rlp_encode_list(items: list[bytes]) -> bytes:
    """Encode a list of already-RLP-encoded items."""
    payload = b"".join(items)
    if len(payload) <= 55:
        return bytes([0xC0 + len(payload)]) + payload
    len_bytes = len(payload).to_bytes((len(payload).bit_length() + 7) // 8, "big")
    return bytes([0xF7 + len(len_bytes)]) + len_bytes + payload


def rlp_encode_u256(value: int) -> bytes:
    """Encode a U256 as RLP bytes (big-endian, minimal encoding)."""
    return rlp_encode_uint(value)


# ---------------------------------------------------------------------------
# Frame TX construction
# ---------------------------------------------------------------------------

def encode_frame(mode: int, flags: int, target: bytes, gas_limit: int, data: bytes) -> bytes:
    """RLP-encode a single frame: [mode, flags, target, gas_limit, data] (post-spec-update)."""
    return rlp_encode_list([
        rlp_encode_uint(mode),
        rlp_encode_uint(flags),
        rlp_encode_address(target),
        rlp_encode_uint(gas_limit),
        rlp_encode_bytes(data),
    ])


def build_frame_tx_payload(
    chain_id: int,
    nonce: int,
    sender: bytes,
    frames_rlp: list[bytes],
    max_priority_fee: int,
    max_fee: int,
) -> bytes:
    """Build the RLP payload (without 0x06 prefix) for a frame tx."""
    return rlp_encode_list([
        rlp_encode_uint(chain_id),
        rlp_encode_uint(nonce),
        rlp_encode_address(sender),
        rlp_encode_list(frames_rlp),       # frames array
        rlp_encode_uint(max_priority_fee),
        rlp_encode_uint(max_fee),
        rlp_encode_uint(0),                 # max_fee_per_blob_gas
        rlp_encode_list([]),                # blob_versioned_hashes (empty)
    ])


def compute_sig_hash(
    chain_id: int,
    nonce: int,
    sender: bytes,
    frames: list[dict],
    max_priority_fee: int,
    max_fee: int,
) -> bytes:
    """Compute the EIP-8141 sig_hash: keccak(0x06 || rlp(...)) with VERIFY data elided."""
    elided_frames = []
    for f in frames:
        if f["execution_mode"] == 1:  # VERIFY
            elided_frames.append(encode_frame(f["mode"], f["flags"], f["target"], f["gas_limit"], b""))
        else:
            elided_frames.append(encode_frame(f["mode"], f["flags"], f["target"], f["gas_limit"], f["data"]))

    payload = build_frame_tx_payload(chain_id, nonce, sender, elided_frames, max_priority_fee, max_fee)
    return keccak(b"\x06" + payload)


def rpc_call(url: str, method: str, params: list) -> dict:
    """Make a JSON-RPC call."""
    req = urllib.request.Request(
        url,
        data=json.dumps({"jsonrpc": "2.0", "method": method, "params": params, "id": 1}).encode(),
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        return json.loads(resp.read())


def main():
    parser = argparse.ArgumentParser(description="Send a test EIP-8141 frame transaction")
    parser.add_argument("--rpc-url", required=True)
    parser.add_argument("--private-key", required=True)
    parser.add_argument("--recipient", default="0x0000000000000000000000000000000000C0FFEE")
    args = parser.parse_args()

    # Derive sender
    account = Account.from_key(args.private_key)
    sender_addr = bytes.fromhex(account.address[2:])
    recipient_addr = bytes.fromhex(args.recipient[2:])

    # Get chain info
    chain_resp = rpc_call(args.rpc_url, "eth_chainId", [])
    chain_id = int(chain_resp["result"], 16)

    nonce_resp = rpc_call(args.rpc_url, "eth_getTransactionCount", [account.address, "latest"])
    nonce = int(nonce_resp["result"], 16)

    gas_resp = rpc_call(args.rpc_url, "eth_gasPrice", [])
    gas_price = int(gas_resp["result"], 16)

    balance_resp = rpc_call(args.rpc_url, "eth_getBalance", [account.address, "latest"])
    balance = int(balance_resp["result"], 16)

    print(f"Chain ID:  {chain_id}")
    print(f"Sender:    {account.address}")
    print(f"Nonce:     {nonce}")
    print(f"Gas price: {gas_price}")
    print(f"Balance:   {balance / 10**18:.4f} ETH")
    print(f"Recipient: {args.recipient}")
    print()

    # Frame layout (post-spec-update: separate mode/flags u8 fields):
    #   Frame 0: VERIFY, target=sender, mode=1, flags=0x03 (scope=3: sender+payer)
    #            Data = [0x00 (secp256k1), v(1), r(32), s(32)] = 66 bytes (filled after signing)
    #   Frame 1: SENDER, target=sender, mode=2, flags=0x00
    #            Data = RLP [[recipient, value, calldata]]

    verify_mode, verify_flags = 1, 0x03  # VERIFY + scope=3 (combined sender+payer)
    sender_mode, sender_flags = 2, 0x00  # SENDER, no flags

    # Build SENDER frame data: RLP-encoded list of SenderCall = [target, value, data]
    transfer_value = 10**16  # 0.01 ETH
    sender_call = rlp_encode_list([
        rlp_encode_address(recipient_addr),     # target
        rlp_encode_uint(transfer_value),         # value (0.01 ETH)
        rlp_encode_bytes(b""),                   # calldata (empty = plain transfer)
    ])
    sender_data = rlp_encode_list([sender_call])  # wrap in outer list: [[target, value, data]]

    # Define frames (VERIFY data will be filled with signature)
    frames = [
        {"mode": verify_mode, "flags": verify_flags, "execution_mode": 1, "target": sender_addr, "gas_limit": 100_000, "data": b""},
        {"mode": sender_mode, "flags": sender_flags, "execution_mode": 2, "target": sender_addr, "gas_limit": 100_000, "data": sender_data},
    ]

    max_priority_fee = 1_000_000_000  # 1 gwei
    max_fee = max(gas_price * 2, 10_000_000_000)  # 2x gas price or 10 gwei

    # Step 1: Compute sig_hash (VERIFY data elided)
    sig_hash = compute_sig_hash(chain_id, nonce, sender_addr, frames, max_priority_fee, max_fee)
    print(f"Sig hash:  0x{sig_hash.hex()}")

    # Step 2: Sign the sig_hash with secp256k1
    pk = eth_keys.PrivateKey(bytes.fromhex(args.private_key))
    signed = pk.sign_msg_hash(sig_hash)
    v = signed.v + 27  # eth_keys returns 0/1, ecrecover needs 27/28
    r = signed.r.to_bytes(32, "big")
    s = signed.s.to_bytes(32, "big")
    print(f"Signature: v={v}, r=0x{r.hex()[:8]}..., s=0x{s.hex()[:8]}...")

    # Step 3: Build VERIFY frame data: [type=0x00, v, r, s] = 66 bytes
    verify_data = bytes([0x00, v]) + r + s
    assert len(verify_data) == 66, f"Expected 66 bytes, got {len(verify_data)}"
    frames[0]["data"] = verify_data

    # Step 4: RLP-encode the full transaction
    frames_rlp = [
        encode_frame(f["mode"], f["flags"], f["target"], f["gas_limit"], f["data"])
        for f in frames
    ]
    tx_payload = build_frame_tx_payload(chain_id, nonce, sender_addr, frames_rlp, max_priority_fee, max_fee)
    raw_tx = b"\x06" + tx_payload
    raw_tx_hex = "0x" + raw_tx.hex()

    print(f"Raw tx:    {raw_tx_hex[:40]}...{raw_tx_hex[-20:]}")
    print(f"Tx size:   {len(raw_tx)} bytes")
    print()

    # Step 5: Send
    print("Sending frame transaction...")
    result = rpc_call(args.rpc_url, "eth_sendRawTransaction", [raw_tx_hex])

    if "error" in result:
        print(f"ERROR: {result['error']}")
        sys.exit(1)

    tx_hash = result["result"]
    print(f"TX HASH:   {tx_hash}")
    print()

    # Step 6: Wait for receipt
    print("Waiting for receipt...")
    import time
    for attempt in range(30):
        time.sleep(2)
        receipt_result = rpc_call(args.rpc_url, "eth_getTransactionReceipt", [tx_hash])
        if receipt_result.get("result") is not None:
            receipt = receipt_result["result"]
            print()
            print("=== FRAME TRANSACTION RECEIPT ===")
            print(f"Status:          {'SUCCESS' if receipt.get('status') == '0x1' else 'FAILED (' + receipt.get('status', '?') + ')'}")
            print(f"Block:           {int(receipt.get('blockNumber', '0x0'), 16)}")
            print(f"Gas used:        {int(receipt.get('gasUsed', '0x0'), 16)}")
            print(f"Payer:           {receipt.get('payer', 'N/A')}")
            frame_receipts = receipt.get("frameReceipts", [])
            if frame_receipts:
                print(f"Frame receipts:  {len(frame_receipts)} frames")
                for i, fr in enumerate(frame_receipts):
                    status = "OK" if fr.get("status") in (True, "0x1", 1) else "FAIL"
                    gas = fr.get("gasUsed", "?")
                    if isinstance(gas, str) and gas.startswith("0x"):
                        gas = int(gas, 16)
                    logs = fr.get("logs", [])
                    print(f"  Frame {i}: {status}, gas={gas}, logs={len(logs)}")
            print("=================================")
            return

        if attempt % 5 == 4:
            print(f"  ... still waiting (attempt {attempt+1}/30)")

    print("Timed out waiting for receipt")
    sys.exit(1)


if __name__ == "__main__":
    main()
