"""Demo 1b: Deploy account via frame transaction and transfer ETH.

This script demonstrates deploying a smart contract account AS PART of
a frame transaction — the account doesn't exist yet when the tx is sent:
1. Deploy AccountDeployer (standard EIP-1559 tx, one-time setup)
2. Predict the CREATE2 address for the new account
3. Pre-fund the predicted address with ETH
4. Send a 3-frame transaction:
   - Frame 0 (DEFAULT): deployer.deploy(salt, initcode) → CREATE2 the account
   - Frame 1 (VERIFY):  account.verifyAndPay() → P256 sig check → APPROVE(scope=2)
   - Frame 2 (SENDER):  account.transfer(recipient, 0.01 ETH)

The account is deployed AND used in the same transaction!

Usage:
    python demo1b_deploy_account.py [--rpc-url http://localhost:8545]
"""
from __future__ import annotations

import sys
import argparse

sys.path.insert(0, '.')

from p256_utils import generate_keypair, sign_hash, pubkey_to_bytes
from frame_tx import (
    FrameTransaction, Frame, build_transfer_frame,
    FRAME_MODE_DEFAULT, FRAME_MODE_VERIFY, FRAME_MODE_SENDER,
)
from contracts import SIMPLE_P256_ACCOUNT_INITCODE, ACCOUNT_DEPLOYER_INITCODE
from rpc_utils import (
    deploy_contract, compute_create2_address,
    fund_address, send_raw_transaction, wait_for_receipt,
    get_nonce, get_balance, get_chain_id, get_base_fee,
    DEFAULT_RPC_URL,
)
from eth_utils import keccak


def main():
    parser = argparse.ArgumentParser(description='Demo 1b: Deploy Account via Frame Tx')
    parser.add_argument('--rpc-url', default=DEFAULT_RPC_URL)
    args = parser.parse_args()
    rpc = args.rpc_url

    print("=" * 60)
    print("Demo 1b: Account Deployment via Frame Transaction")
    print("=" * 60)

    chain_id = get_chain_id(rpc)
    base_fee = get_base_fee(rpc)

    # Step 1: Generate P256 keypair
    print("\n[1] Generating P256 keypair...")
    private_key, pub_x, pub_y = generate_keypair()
    x_bytes, y_bytes = pubkey_to_bytes(pub_x, pub_y)
    print(f"  Public key X: 0x{pub_x:064x}")

    # Step 2: Deploy AccountDeployer (one-time infrastructure)
    print("\n[2] Deploying AccountDeployer...")
    _, deployer_addr = deploy_contract(ACCOUNT_DEPLOYER_INITCODE, gas_limit=500_000, rpc_url=rpc)

    # Step 3: Predict the CREATE2 address
    print("\n[3] Predicting CREATE2 address...")
    salt = keccak(x_bytes + y_bytes)
    full_initcode = SIMPLE_P256_ACCOUNT_INITCODE + x_bytes + y_bytes
    predicted_addr = compute_create2_address(deployer_addr, salt, full_initcode)
    print(f"  Predicted address: 0x{predicted_addr.hex()}")

    # Step 4: Pre-fund the predicted address (account doesn't exist yet!)
    print("\n[4] Pre-funding predicted address with 1 ETH...")
    fund_address(predicted_addr, 10**18, rpc_url=rpc)
    balance = get_balance(predicted_addr, rpc_url=rpc)
    print(f"  Balance at predicted address: {balance / 10**18:.4f} ETH")

    # Step 5: Build 3-frame transaction
    # The sender IS the predicted address — it will be deployed in Frame 0
    print("\n[5] Building 3-frame transaction...")
    sender_nonce = get_nonce(predicted_addr, rpc_url=rpc)
    recipient = bytes.fromhex("00" * 19 + "03")  # address 0x03
    print(f"  Sender nonce: {sender_nonce} (account not yet deployed)")
    print(f"  Sender (predicted): 0x{predicted_addr.hex()}")
    print(f"  Recipient: 0x{recipient.hex()}")

    # Frame 0: DEFAULT — deploy the account via CREATE2
    deploy_calldata = (
        b'\x00\x00\x00\x01' +      # deploy selector
        salt +                       # salt (32 bytes)
        full_initcode               # initcode (variable)
    )

    tx = FrameTransaction(
        chain_id=chain_id,
        nonce=sender_nonce,
        sender=predicted_addr,
        frames=[
            # Frame 0 (DEFAULT): deploy account via CREATE2
            Frame(
                mode=FRAME_MODE_DEFAULT,
                target=deployer_addr,
                gas_limit=500_000,
                data=deploy_calldata,
            ),
            # Frame 1 (VERIFY): P256 sig check → APPROVE(scope=2)
            Frame(
                mode=FRAME_MODE_VERIFY,
                target=predicted_addr,
                gas_limit=200_000,
                data=b'',  # placeholder, filled after signing
            ),
            # Frame 2 (SENDER): transfer 0.01 ETH
            build_transfer_frame(
                target=predicted_addr,
                gas_limit=100_000,
                dest=recipient,
                amount=10**16,  # 0.01 ETH
            ),
        ],
        max_priority_fee_per_gas=1_000_000_000,
        max_fee_per_gas=base_fee * 2 + 1_000_000_000,
    )

    # Step 6: Sign with P256
    print("\n[6] Signing with P256...")
    sig_hash = tx.compute_sig_hash()
    r, s = sign_hash(private_key, sig_hash)
    print(f"  Sig hash: 0x{sig_hash.hex()}")

    # Fill Frame 1 VERIFY data: selector(4) + r(32) + s(32)
    # Selector 0x00000003 = verifyAndPay() → APPROVE(scope=2)
    tx.frames[1].data = (
        b'\x00\x00\x00\x03' +
        r.to_bytes(32, 'big') +
        s.to_bytes(32, 'big')
    )

    # Step 7: Send
    print("\n[7] Sending 3-frame transaction (deploy + verify + transfer)...")
    raw_tx = tx.encode_canonical()
    print(f"  Raw tx size: {len(raw_tx)} bytes")
    print(f"  Frames: DEFAULT(deploy) + VERIFY(P256) + SENDER(transfer)")

    tx_hash_hex = send_raw_transaction(raw_tx, rpc)
    print(f"  Tx hash: {tx_hash_hex}")

    receipt = wait_for_receipt(tx_hash_hex, rpc)
    status = int(receipt["status"], 16)
    gas_used = int(receipt["gasUsed"], 16)
    print(f"  Status: {'SUCCESS' if status == 1 else 'FAILED'}")
    print(f"  Gas used: {gas_used}")

    # Step 8: Verify results
    print("\n[8] Verifying results...")

    # Check account was deployed (has code)
    from rpc_utils import rpc_call
    code = rpc_call("eth_getCode", ["0x" + predicted_addr.hex(), "latest"], rpc)
    has_code = code is not None and code != "0x" and len(code) > 2
    print(f"  Account has code: {has_code} ({len(code)//2 - 1} bytes)" if has_code else f"  Account has code: {has_code}")

    # Check recipient balance
    recipient_balance = get_balance(recipient, rpc)
    account_balance = get_balance(predicted_addr, rpc)
    print(f"  Recipient balance: {recipient_balance / 10**18:.4f} ETH")
    print(f"  Account balance: {account_balance / 10**18:.4f} ETH")

    # Check receipt frame fields
    print("\n[9] Checking receipt fields...")
    if "payer" in receipt and receipt["payer"]:
        print(f"  Payer: {receipt['payer']}")
    else:
        print("  Payer: not set (self-paying, scope=2)")

    if "frameReceipts" in receipt and receipt["frameReceipts"]:
        print(f"  Frame receipts: {len(receipt['frameReceipts'])} frames")
        for i, fr in enumerate(receipt["frameReceipts"]):
            fr_status = int(fr["status"], 16)
            fr_gas = int(fr["gasUsed"], 16)
            mode_names = {0: "DEFAULT", 1: "VERIFY", 2: "SENDER"}
            mode = mode_names.get(tx.frames[i].mode, "?")
            print(f"    Frame {i} ({mode}): status={'OK' if fr_status else 'FAIL'}, gas_used={fr_gas}")

    if has_code and recipient_balance >= 10**16:
        print("\n  SUCCESS: Account deployed AND ETH transferred in one frame tx!")
    else:
        print("\n  FAILED: Check results above")

    print("\n" + "=" * 60)
    print("Demo 1b complete!")
    print("=" * 60)


if __name__ == '__main__':
    main()
