"""Demo 1: Deploy SimpleP256Account via CREATE2 and send frame transactions.

This script demonstrates:
1. Deploying the AccountDeployer (one-time setup)
2. Deploying a P256 smart contract account via CREATE2 (deterministic address)
3. Funding the account with ETH
4. Sending a frame transaction (VERIFY + SENDER) for an ETH transfer
5. Verifying the receipt

Usage:
    python demo1_simple_account.py [--rpc-url http://localhost:8545]
"""
from __future__ import annotations

import sys
import argparse

# Add scripts dir to path
sys.path.insert(0, '.')

from p256_utils import generate_keypair, sign_hash, pubkey_to_bytes
from frame_tx import (
    FrameTransaction, Frame, build_transfer_frame,
    FRAME_MODE_VERIFY, FRAME_MODE_SENDER,
)
from contracts import SIMPLE_P256_ACCOUNT_INITCODE, ACCOUNT_DEPLOYER_INITCODE
from rpc_utils import (
    deploy_contract, deploy_via_create2, compute_create2_address,
    fund_address, send_raw_transaction, wait_for_receipt,
    get_nonce, get_balance, get_chain_id, get_base_fee,
    DEFAULT_RPC_URL,
)
from eth_utils import keccak


def main():
    parser = argparse.ArgumentParser(description='Demo 1: Simple P256 Account')
    parser.add_argument('--rpc-url', default=DEFAULT_RPC_URL)
    args = parser.parse_args()
    rpc = args.rpc_url

    print("=" * 60)
    print("Demo 1: SimpleP256Account Frame Transaction (CREATE2)")
    print("=" * 60)

    # Step 1: Generate P256 keypair
    print("\n[1] Generating P256 keypair...")
    private_key, pub_x, pub_y = generate_keypair()
    x_bytes, y_bytes = pubkey_to_bytes(pub_x, pub_y)
    print(f"  Public key X: 0x{pub_x:064x}")
    print(f"  Public key Y: 0x{pub_y:064x}")

    # Step 2: Deploy AccountDeployer
    print("\n[2] Deploying AccountDeployer...")
    _, deployer_addr = deploy_contract(ACCOUNT_DEPLOYER_INITCODE, gas_limit=500_000, rpc_url=rpc)
    print(f"  Deployer address: 0x{deployer_addr.hex()}")

    # Step 3: Deploy SimpleP256Account via CREATE2
    print("\n[3] Deploying SimpleP256Account via CREATE2...")
    # Salt = keccak256(pubkey_x || pubkey_y) — deterministic from the public key
    salt = keccak(x_bytes + y_bytes)
    # Full initcode includes constructor + runtime + constructor args
    full_initcode = SIMPLE_P256_ACCOUNT_INITCODE + x_bytes + y_bytes

    # Predict the address before deployment
    predicted_addr = compute_create2_address(deployer_addr, salt, full_initcode)
    print(f"  Predicted address: 0x{predicted_addr.hex()}")

    _, account_addr = deploy_via_create2(
        deployer_addr, salt, full_initcode,
        gas_limit=500_000, rpc_url=rpc,
    )
    assert account_addr == predicted_addr, "CREATE2 address mismatch!"
    print(f"  Address matches prediction: yes")

    # Step 4: Fund the account with ETH
    print("\n[4] Funding account with 1 ETH...")
    fund_address(account_addr, 10**18, rpc_url=rpc)
    balance = get_balance(account_addr, rpc_url=rpc)
    print(f"  Account balance: {balance / 10**18:.4f} ETH")

    # Step 5: Build frame transaction
    print("\n[5] Building frame transaction (ETH transfer)...")
    chain_id = get_chain_id(rpc)
    base_fee = get_base_fee(rpc)
    account_nonce = get_nonce(account_addr, rpc_url=rpc)
    recipient = bytes.fromhex("00" * 19 + "01")  # address 0x01
    print(f"  Account nonce: {account_nonce}")

    # Build the frame tx structure first (without VERIFY data for sig_hash)
    tx = FrameTransaction(
        chain_id=chain_id,
        nonce=account_nonce,
        sender=account_addr,
        frames=[
            # VERIFY frame — data will be filled after signing
            Frame(
                mode=FRAME_MODE_VERIFY,
                target=account_addr,
                gas_limit=200_000,
                data=b'',  # placeholder
            ),
            # SENDER frame — transfer 0.01 ETH
            build_transfer_frame(
                target=account_addr,
                gas_limit=100_000,
                dest=recipient,
                amount=10**16,  # 0.01 ETH
            ),
        ],
        max_priority_fee_per_gas=1_000_000_000,  # 1 gwei
        max_fee_per_gas=base_fee * 2 + 1_000_000_000,
    )

    # Step 6: Compute sig_hash and sign with P256
    print("\n[6] Signing with P256...")
    sig_hash = tx.compute_sig_hash()
    print(f"  Sig hash: 0x{sig_hash.hex()}")

    r, s = sign_hash(private_key, sig_hash)
    print(f"  Signature r: 0x{r:064x}")
    print(f"  Signature s: 0x{s:064x}")

    # Step 7: Fill VERIFY frame data with signature
    # Use selector 0x00000003 = verifyAndPay() for combined sender+payer (self-paying)
    verify_data = (
        b'\x00\x00\x00\x03' +      # selector for verifyAndPay()
        r.to_bytes(32, 'big') +      # r
        s.to_bytes(32, 'big')        # s
    )
    tx.frames[0].data = verify_data

    # Step 8: Send frame transaction
    print("\n[7] Sending frame transaction...")
    raw_tx = tx.encode_canonical()
    print(f"  Raw tx size: {len(raw_tx)} bytes")
    print(f"  Tx hash: 0x{tx.tx_hash().hex()}")

    tx_hash_hex = send_raw_transaction(raw_tx, rpc)
    print(f"  Submitted tx hash: {tx_hash_hex}")

    # Step 9: Wait for receipt
    print("\n[8] Waiting for receipt...")
    receipt = wait_for_receipt(tx_hash_hex, rpc)
    status = int(receipt["status"], 16)
    gas_used = int(receipt["gasUsed"], 16)
    print(f"  Status: {'SUCCESS' if status == 1 else 'FAILED'}")
    print(f"  Gas used: {gas_used}")

    # Step 10: Verify recipient balance
    print("\n[9] Verifying result...")
    recipient_balance = get_balance(recipient, rpc)
    account_balance = get_balance(account_addr, rpc)
    print(f"  Recipient balance: {recipient_balance / 10**18:.4f} ETH")
    print(f"  Account balance: {account_balance / 10**18:.4f} ETH")

    # Step 11: Verify receipt fields (Task 55)
    print("\n[10] Verifying receipt fields...")
    errors = []

    # Payer should be the account itself (scope=2, self-paying)
    payer = receipt.get("payer")
    if payer:
        print(f"  Payer: {payer}")
        if payer.lower() != ("0x" + account_addr.hex()).lower():
            errors.append(f"Payer mismatch: expected 0x{account_addr.hex()}, got {payer}")
    else:
        errors.append("Payer field missing from receipt")

    # Frame receipts should have 2 entries (VERIFY + SENDER)
    frame_receipts = receipt.get("frameReceipts")
    if frame_receipts is not None:
        print(f"  Frame receipts: {len(frame_receipts)} frames")
        if len(frame_receipts) != 2:
            errors.append(f"Expected 2 frame receipts, got {len(frame_receipts)}")
        for i, fr in enumerate(frame_receipts):
            fr_status = int(fr["status"], 16)
            fr_gas = int(fr["gasUsed"], 16)
            mode = "VERIFY" if i == 0 else "SENDER"
            print(f"    Frame {i} ({mode}): status={'OK' if fr_status else 'FAIL'}, gas_used={fr_gas}")
            if not fr_status:
                errors.append(f"Frame {i} failed")
        # Gas accounting: sum of frame gas should equal total gas used
        frame_gas_sum = sum(int(fr["gasUsed"], 16) for fr in frame_receipts)
        print(f"  Frame gas sum: {frame_gas_sum}, total gas: {gas_used}")
    else:
        errors.append("frameReceipts field missing from receipt")

    if recipient_balance >= 10**16 and not errors:
        print("\n  SUCCESS: ETH transfer + receipt verification passed!")
    elif errors:
        print(f"\n  RECEIPT ISSUES: {'; '.join(errors)}")
    else:
        print("\n  FAILED: Recipient did not receive ETH")

    print("\n" + "=" * 60)
    print("Demo 1 complete!")
    print("=" * 60)


if __name__ == '__main__':
    main()
