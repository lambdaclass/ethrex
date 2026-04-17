#!/usr/bin/env python3
"""Deploy EIP-8141 devnet contracts using web3.py.

Usage:
    python3 deploy-contracts.py --rpc-url http://127.0.0.1:32003 \
        --private-key <DEPLOYER_PRIVATE_KEY>

Requires: pip3 install web3 py-solc-x
"""
import argparse
import json
import subprocess
import sys
import tempfile
import urllib.request
from pathlib import Path

try:
    from web3 import Web3
    from eth_account import Account as EthAccount
except ImportError:
    print("ERROR: web3 not installed. Run: pip3 install web3")
    sys.exit(1)


CANONICAL_PAYMASTER_URL = (
    "https://raw.githubusercontent.com/ethereum/EIPs/master/"
    "assets/eip-8141/CanonicalPaymaster.sol"
)


def compile_contract(sol_path: str) -> tuple[str, list]:
    """Compile a Solidity contract using solc and return (bytecode, abi)."""
    result = subprocess.run(
        ["solc", "--combined-json", "abi,bin", sol_path],
        capture_output=True, text=True,
    )
    if result.returncode != 0:
        print(f"solc error:\n{result.stderr}")
        sys.exit(1)

    combined = json.loads(result.stdout)
    # Get the first contract from the combined output
    for key, contract in combined["contracts"].items():
        if "CanonicalPaymaster" in key:
            return contract["bin"], json.loads(contract["abi"])

    print("ERROR: CanonicalPaymaster not found in solc output")
    sys.exit(1)


def main():
    parser = argparse.ArgumentParser(description="Deploy EIP-8141 devnet contracts")
    parser.add_argument("--rpc-url", required=True, help="RPC endpoint")
    parser.add_argument("--private-key", required=True, help="Deployer private key (hex, no 0x prefix)")
    parser.add_argument("--fund-amount", default=100, type=int, help="ETH to fund paymaster with (default: 100)")
    args = parser.parse_args()

    w3 = Web3(Web3.HTTPProvider(args.rpc_url))
    if not w3.is_connected():
        print(f"ERROR: Cannot connect to {args.rpc_url}")
        sys.exit(1)

    account = EthAccount.from_key(args.private_key)
    deployer = account.address

    chain_id = w3.eth.chain_id
    print(f"Chain ID: {chain_id}")
    print(f"Block:    {w3.eth.block_number}")
    print(f"Deployer: {deployer}")
    print(f"Balance:  {w3.from_wei(w3.eth.get_balance(deployer), 'ether')} ETH")
    print()

    # Download CanonicalPaymaster source
    print("Downloading CanonicalPaymaster.sol...")
    with tempfile.NamedTemporaryFile(suffix=".sol", mode="w", delete=False) as f:
        sol_content = urllib.request.urlopen(CANONICAL_PAYMASTER_URL).read().decode()
        f.write(sol_content)
        sol_path = f.name
    print(f"Saved to {sol_path}")

    # Compile
    print("Compiling...")
    bytecode, abi = compile_contract(sol_path)
    print(f"Bytecode: {len(bytecode)//2} bytes, ABI: {len(abi)} entries")

    # Deploy with constructor arg (owner = deployer address)
    print(f"\nDeploying CanonicalPaymaster (owner={deployer})...")
    contract = w3.eth.contract(abi=abi, bytecode=bytecode)
    deploy_data = contract.constructor(deployer).build_transaction({
        "from": deployer,
        "nonce": w3.eth.get_transaction_count(deployer),
        "value": w3.to_wei(10, "ether"),
        "gas": 500_000,
        "maxFeePerGas": w3.eth.gas_price * 2 or w3.to_wei(10, "gwei"),
        "maxPriorityFeePerGas": w3.to_wei(1, "gwei"),
        "chainId": chain_id,
    })
    signed = account.sign_transaction(deploy_data)
    tx_hash = w3.eth.send_raw_transaction(signed.raw_transaction)
    print(f"Deploy tx: {tx_hash.hex()}")

    receipt = w3.eth.wait_for_transaction_receipt(tx_hash, timeout=120)
    paymaster_addr = receipt["contractAddress"]
    print(f"Deployed at: {paymaster_addr}")
    print(f"Gas used: {receipt['gasUsed']}")
    print(f"Status: {'SUCCESS' if receipt['status'] == 1 else 'FAILED'}")

    if receipt["status"] != 1:
        print("ERROR: Deployment transaction reverted!")
        sys.exit(1)

    # Verify code exists
    code = w3.eth.get_code(paymaster_addr)
    if len(code) <= 2:
        print("ERROR: No code at deployed address!")
        sys.exit(1)
    print(f"Contract code: {len(code)} bytes")

    # Fund with additional ETH
    if args.fund_amount > 10:
        extra = args.fund_amount - 10
        print(f"\nFunding paymaster with {extra} more ETH...")
        fund_tx = {
            "to": paymaster_addr,
            "value": w3.to_wei(extra, "ether"),
            "nonce": w3.eth.get_transaction_count(deployer),
            "gas": 21_000,
            "maxFeePerGas": w3.eth.gas_price * 2 or w3.to_wei(10, "gwei"),
            "maxPriorityFeePerGas": w3.to_wei(1, "gwei"),
            "chainId": chain_id,
        }
        signed = account.sign_transaction(fund_tx)
        tx_hash = w3.eth.send_raw_transaction(signed.raw_transaction)
        w3.eth.wait_for_transaction_receipt(tx_hash, timeout=60)

    balance = w3.from_wei(w3.eth.get_balance(paymaster_addr), "ether")
    print(f"Paymaster balance: {balance} ETH")

    print("\n=== Deployment Complete ===")
    print(f"CanonicalPaymaster: {paymaster_addr}")
    print(f"Owner/Signer:       {deployer}")
    print(f"Chain ID:           {chain_id}")


if __name__ == "__main__":
    main()
