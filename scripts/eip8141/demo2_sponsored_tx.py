"""Demo 2: True sponsored transaction via ERC20Sponsor.

This script demonstrates the full EIP-8141 sponsored flow:
1. Deploy AccountDeployer, SimpleP256Account (CREATE2), MockERC20, ERC20Sponsor
2. Configure sponsor with token address and rate
3. Mint tokens to the account (sender)
4. Fund the sponsor with ETH (payer pays for gas)
5. Send a 3-frame transaction:
   - Frame 0 (VERIFY): sender account verifies P256 signature → APPROVE(scope=0)
   - Frame 1 (VERIFY): sponsor verifies token balance → APPROVE(scope=1)
   - Frame 2 (SENDER): sender account executes ETH transfer

The sender never pays ETH for gas — the sponsor does!

Usage:
    python demo2_sponsored_tx.py [--rpc-url http://localhost:8545]
"""
from __future__ import annotations

import sys
import argparse

sys.path.insert(0, '.')

from p256_utils import generate_keypair, sign_hash, pubkey_to_bytes
from frame_tx import (
    FrameTransaction, Frame, FRAME_MODE_VERIFY, FRAME_MODE_SENDER,
)
from contracts import (
    SIMPLE_P256_ACCOUNT_INITCODE, ACCOUNT_DEPLOYER_INITCODE,
    MOCK_ERC20_INITCODE, ERC20_SPONSOR_INITCODE,
)
from rpc_utils import (
    deploy_contract, deploy_via_create2, compute_create2_address,
    fund_address, send_raw_transaction, send_eip1559_tx,
    wait_for_receipt, get_nonce, get_balance, get_chain_id, get_base_fee,
    DEFAULT_RPC_URL,
)
from eth_utils import keccak


def main():
    parser = argparse.ArgumentParser(description='Demo 2: Sponsored Transaction')
    parser.add_argument('--rpc-url', default=DEFAULT_RPC_URL)
    args = parser.parse_args()
    rpc = args.rpc_url

    print("=" * 60)
    print("Demo 2: True Sponsored Frame Transaction")
    print("=" * 60)

    chain_id = get_chain_id(rpc)
    base_fee = get_base_fee(rpc)

    # ── Step 1: Deploy infrastructure ──────────────────────────
    print("\n[1] Generating P256 keypair...")
    private_key, pub_x, pub_y = generate_keypair()
    x_bytes, y_bytes = pubkey_to_bytes(pub_x, pub_y)
    print(f"  Public key X: 0x{pub_x:064x}")

    print("\n[2] Deploying AccountDeployer...")
    _, deployer_addr = deploy_contract(ACCOUNT_DEPLOYER_INITCODE, gas_limit=500_000, rpc_url=rpc)

    print("\n[3] Deploying SimpleP256Account via CREATE2...")
    salt = keccak(x_bytes + y_bytes)
    full_initcode = SIMPLE_P256_ACCOUNT_INITCODE + x_bytes + y_bytes
    predicted_addr = compute_create2_address(deployer_addr, salt, full_initcode)
    print(f"  Predicted address: 0x{predicted_addr.hex()}")
    _, account_addr = deploy_via_create2(
        deployer_addr, salt, full_initcode,
        gas_limit=500_000, rpc_url=rpc,
    )

    print("\n[4] Deploying MockERC20...")
    _, token_addr = deploy_contract(MOCK_ERC20_INITCODE, gas_limit=500_000, rpc_url=rpc)

    print("\n[5] Deploying ERC20Sponsor...")
    _, sponsor_addr = deploy_contract(ERC20_SPONSOR_INITCODE, gas_limit=500_000, rpc_url=rpc)

    # ── Step 2: Configure ─────────────────────────────────────
    print("\n[6] Configuring ERC20Sponsor (token + rate)...")
    setconfig_data = (
        b'\x00\x00\x00\x01' +                    # selector for setConfig
        token_addr.rjust(32, b'\x00') +           # token_address
        (1).to_bytes(32, 'big')                   # rate = 1 token per gas
    )
    tx_hash = send_eip1559_tx(
        to=sponsor_addr, value=0, data=setconfig_data,
        gas_limit=100_000, rpc_url=rpc,
    )
    wait_for_receipt(tx_hash, rpc)
    print("  Sponsor configured.")

    # ── Step 3: Fund actors ────────────────────────────────────
    print("\n[7] Minting 1000 tokens to sender account...")
    mint_data = (
        bytes.fromhex("40c10f19") +                            # mint selector
        account_addr.rjust(32, b'\x00') +                      # to
        (1000 * 10**18).to_bytes(32, 'big')                    # amount
    )
    tx_hash = send_eip1559_tx(
        to=token_addr, value=0, data=mint_data,
        gas_limit=100_000, rpc_url=rpc,
    )
    wait_for_receipt(tx_hash, rpc)
    print("  Tokens minted.")

    print("\n[8] Funding sponsor with 10 ETH (payer)...")
    fund_address(sponsor_addr, 10 * 10**18, rpc_url=rpc)
    sponsor_bal = get_balance(sponsor_addr, rpc)
    print(f"  Sponsor balance: {sponsor_bal / 10**18:.4f} ETH")

    # Note: the sender account does NOT need ETH for gas —
    # the sponsor pays. But we fund a small amount for the transfer value.
    print("\n[9] Funding sender account with 0.1 ETH (for transfer value only)...")
    fund_address(account_addr, 10**17, rpc_url=rpc)
    account_bal = get_balance(account_addr, rpc)
    print(f"  Account balance: {account_bal / 10**18:.4f} ETH")

    # ── Step 4: Build 3-frame sponsored transaction ────────────
    print("\n[10] Building 3-frame sponsored transaction...")
    account_nonce = get_nonce(account_addr, rpc_url=rpc)
    recipient = bytes.fromhex("00" * 19 + "02")
    print(f"  Account nonce: {account_nonce}")
    print(f"  Sender: 0x{account_addr.hex()}")
    print(f"  Payer (sponsor): 0x{sponsor_addr.hex()}")
    print(f"  Recipient: 0x{recipient.hex()}")

    tx = FrameTransaction(
        chain_id=chain_id,
        nonce=account_nonce,
        sender=account_addr,
        frames=[
            # Frame 0 (VERIFY): sender approval — P256 sig check → APPROVE(scope=0)
            Frame(
                mode=FRAME_MODE_VERIFY,
                target=account_addr,
                gas_limit=200_000,
                data=b'',  # placeholder, filled after signing
            ),
            # Frame 1 (VERIFY): payer approval — sponsor checks token balance → APPROVE(scope=1)
            Frame(
                mode=FRAME_MODE_VERIFY,
                target=sponsor_addr,
                gas_limit=100_000,
                data=b'\x00\x00\x00\x00',  # selector for verify()
            ),
            # Frame 2 (SENDER): actual operation — transfer 0.01 ETH
            Frame(
                mode=FRAME_MODE_SENDER,
                target=account_addr,
                gas_limit=100_000,
                data=(
                    b'\x00\x00\x00\x02' +                  # transfer selector
                    recipient.rjust(32, b'\x00') +          # dest
                    (10**16).to_bytes(32, 'big')            # 0.01 ETH
                ),
            ),
        ],
        max_priority_fee_per_gas=1_000_000_000,
        max_fee_per_gas=base_fee * 2 + 1_000_000_000,
    )

    # ── Step 5: Sign ───────────────────────────────────────────
    print("\n[11] Signing with P256...")
    sig_hash = tx.compute_sig_hash()
    r, s = sign_hash(private_key, sig_hash)
    print(f"  Sig hash: 0x{sig_hash.hex()}")

    # Fill Frame 0 VERIFY data: selector(4) + r(32) + s(32)
    # selector 0x00000000 = verify() → APPROVE(scope=0, sender only)
    tx.frames[0].data = (
        b'\x00\x00\x00\x00' +
        r.to_bytes(32, 'big') +
        s.to_bytes(32, 'big')
    )

    # ── Step 6: Send ──────────────────────────────────────────
    print("\n[12] Sending 3-frame sponsored transaction...")
    raw_tx = tx.encode_canonical()
    print(f"  Raw tx size: {len(raw_tx)} bytes")
    print(f"  Frames: {len(tx.frames)} (VERIFY-sender + VERIFY-payer + SENDER)")

    tx_hash_hex = send_raw_transaction(raw_tx, rpc)
    print(f"  Tx hash: {tx_hash_hex}")

    receipt = wait_for_receipt(tx_hash_hex, rpc)
    status = int(receipt["status"], 16)
    gas_used = int(receipt["gasUsed"], 16)
    print(f"  Status: {'SUCCESS' if status == 1 else 'FAILED'}")
    print(f"  Gas used: {gas_used}")

    # ── Step 7: Verify results ────────────────────────────────
    print("\n[13] Verifying results...")
    recipient_balance = get_balance(recipient, rpc)
    account_balance_after = get_balance(account_addr, rpc)
    sponsor_balance_after = get_balance(sponsor_addr, rpc)

    print(f"  Recipient balance: {recipient_balance / 10**18:.6f} ETH")
    print(f"  Sender account balance: {account_balance_after / 10**18:.4f} ETH")
    print(f"  Sponsor balance: {sponsor_balance_after / 10**18:.4f} ETH")

    # The key insight: the sender's balance should only decrease by the
    # transfer amount (0.01 ETH), NOT by gas. Gas is paid by the sponsor.
    sender_spent = (account_bal - account_balance_after)
    sponsor_spent = (sponsor_bal - sponsor_balance_after)
    print(f"\n  Sender spent: {sender_spent / 10**18:.6f} ETH (should be ~0.01, transfer only)")
    print(f"  Sponsor spent: {sponsor_spent / 10**18:.6f} ETH (gas cost)")

    # ── Step 8: Verify receipt fields ────────────────────────
    print("\n[14] Verifying receipt fields...")
    errors = []

    # Payer should be the sponsor (scope=1)
    payer = receipt.get("payer")
    if payer:
        print(f"  Payer: {payer}")
        if payer.lower() != ("0x" + sponsor_addr.hex()).lower():
            errors.append(f"Payer mismatch: expected sponsor 0x{sponsor_addr.hex()}, got {payer}")
        else:
            print("  Payer matches sponsor address")
    else:
        errors.append("Payer field missing from receipt")

    # Frame receipts should have 3 entries
    frame_receipts = receipt.get("frameReceipts")
    if frame_receipts is not None:
        print(f"  Frame receipts: {len(frame_receipts)} frames")
        mode_names = ["VERIFY(sender)", "VERIFY(payer)", "SENDER(transfer)"]
        if len(frame_receipts) != 3:
            errors.append(f"Expected 3 frame receipts, got {len(frame_receipts)}")
        for i, fr in enumerate(frame_receipts):
            fr_status = int(fr["status"], 16)
            fr_gas = int(fr["gasUsed"], 16)
            mode = mode_names[i] if i < len(mode_names) else "?"
            print(f"    Frame {i} ({mode}): status={'OK' if fr_status else 'FAIL'}, gas_used={fr_gas}")
            if not fr_status:
                errors.append(f"Frame {i} failed")
        frame_gas_sum = sum(int(fr["gasUsed"], 16) for fr in frame_receipts)
        print(f"  Frame gas sum: {frame_gas_sum}, total gas: {gas_used}")
    else:
        errors.append("frameReceipts field missing from receipt")

    if recipient_balance >= 10**16 and not errors:
        print("\n  SUCCESS: Sponsored ETH transfer + receipt verification passed!")
        print("  The sender paid ONLY the transfer value.")
        print("  The sponsor paid for ALL gas costs.")
    elif errors:
        print(f"\n  RECEIPT ISSUES: {'; '.join(errors)}")
    else:
        print("\n  FAILED: Recipient did not receive ETH")

    print("\n" + "=" * 60)
    print("Demo 2 complete!")
    print("=" * 60)


if __name__ == '__main__':
    main()
