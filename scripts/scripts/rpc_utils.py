"""JSON-RPC utilities for interacting with ethrex node."""
from __future__ import annotations

import json
import requests
from eth_utils import keccak
import rlp


DEFAULT_RPC_URL = "http://localhost:8545"

# Dev account (pre-funded in genesis)
# Using a standard dev private key — this is NOT a real key, just for local testing
DEV_PRIVATE_KEY = bytes.fromhex(
    "b6b15c8cb491557369f3c7d2c287b053eb229daa9c22138887752191c9520659"
)
DEV_ADDRESS = bytes.fromhex("3f1Eae7D46d88F08fc2F8ed27FCb2AB183EB2d0E")


def rpc_call(method: str, params: list, rpc_url: str = DEFAULT_RPC_URL) -> dict:
    """Make a JSON-RPC call."""
    payload = {
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1,
    }
    resp = requests.post(rpc_url, json=payload, timeout=30)
    resp.raise_for_status()
    result = resp.json()
    if "error" in result:
        raise RuntimeError(f"RPC error: {result['error']}")
    return result.get("result")


def get_nonce(address: bytes, rpc_url: str = DEFAULT_RPC_URL) -> int:
    """Get the nonce for an address."""
    addr_hex = "0x" + address.hex()
    result = rpc_call("eth_getTransactionCount", [addr_hex, "latest"], rpc_url)
    return int(result, 16)


def get_balance(address: bytes, rpc_url: str = DEFAULT_RPC_URL) -> int:
    """Get the balance for an address."""
    addr_hex = "0x" + address.hex()
    result = rpc_call("eth_getBalance", [addr_hex, "latest"], rpc_url)
    return int(result, 16)


def get_chain_id(rpc_url: str = DEFAULT_RPC_URL) -> int:
    """Get the chain ID."""
    result = rpc_call("eth_chainId", [], rpc_url)
    return int(result, 16)


def get_base_fee(rpc_url: str = DEFAULT_RPC_URL) -> int:
    """Get the base fee from the latest block."""
    block = rpc_call("eth_getBlockByNumber", ["latest", False], rpc_url)
    return int(block["baseFeePerGas"], 16)


def get_tx_receipt(tx_hash: str, rpc_url: str = DEFAULT_RPC_URL) -> dict:
    """Get transaction receipt."""
    return rpc_call("eth_getTransactionReceipt", [tx_hash], rpc_url)


def send_raw_transaction(raw_tx: bytes, rpc_url: str = DEFAULT_RPC_URL) -> str:
    """Send a raw transaction. Returns tx hash."""
    return rpc_call("eth_sendRawTransaction", ["0x" + raw_tx.hex()], rpc_url)


def send_eip1559_tx(
    to: bytes | None,
    value: int,
    data: bytes,
    gas_limit: int,
    private_key: bytes = DEV_PRIVATE_KEY,
    rpc_url: str = DEFAULT_RPC_URL,
    nonce: int | None = None,
) -> str:
    """Send an EIP-1559 transaction signed with the dev account.

    Returns the transaction hash.
    """
    from eth_account import Account

    chain_id = get_chain_id(rpc_url)
    base_fee = get_base_fee(rpc_url)

    acct = Account.from_key(private_key)
    if nonce is None:
        nonce = get_nonce(acct.address if isinstance(acct.address, bytes)
                         else bytes.fromhex(acct.address[2:]), rpc_url)

    tx_dict = {
        "type": 2,  # EIP-1559
        "chainId": chain_id,
        "nonce": nonce,
        "maxPriorityFeePerGas": 1_000_000_000,  # 1 gwei
        "maxFeePerGas": base_fee * 2 + 1_000_000_000,
        "gas": gas_limit,
        "value": value,
        "data": data,
    }

    if to is not None:
        from eth_utils import to_checksum_address
        tx_dict["to"] = to_checksum_address("0x" + to.hex())

    signed = acct.sign_transaction(tx_dict)
    tx_hash = send_raw_transaction(signed.raw_transaction, rpc_url)
    return tx_hash


def wait_for_receipt(tx_hash: str, rpc_url: str = DEFAULT_RPC_URL,
                     timeout: int = 60) -> dict:
    """Wait for a transaction receipt."""
    import time
    start = time.time()
    while time.time() - start < timeout:
        receipt = get_tx_receipt(tx_hash, rpc_url)
        if receipt is not None:
            return receipt
        time.sleep(1)
    raise TimeoutError(f"Transaction {tx_hash} not mined within {timeout}s")


def deploy_contract(
    initcode: bytes,
    gas_limit: int = 500_000,
    private_key: bytes = DEV_PRIVATE_KEY,
    rpc_url: str = DEFAULT_RPC_URL,
) -> tuple[str, bytes]:
    """Deploy a contract. Returns (tx_hash, contract_address)."""
    tx_hash = send_eip1559_tx(
        to=None,
        value=0,
        data=initcode,
        gas_limit=gas_limit,
        private_key=private_key,
        rpc_url=rpc_url,
    )
    receipt = wait_for_receipt(tx_hash, rpc_url)
    contract_addr = bytes.fromhex(receipt["contractAddress"][2:])
    print(f"  Deployed at: 0x{contract_addr.hex()}")
    print(f"  Gas used: {int(receipt['gasUsed'], 16)}")
    return tx_hash, contract_addr


def compute_create2_address(deployer: bytes, salt: bytes, initcode: bytes) -> bytes:
    """Compute CREATE2 address: keccak256(0xff ++ deployer ++ salt ++ keccak256(initcode))[12:]"""
    code_hash = keccak(initcode)
    preimage = b'\xff' + deployer + salt + code_hash
    return keccak(preimage)[12:]


def deploy_via_create2(
    deployer_addr: bytes,
    salt: bytes,
    initcode: bytes,
    gas_limit: int = 500_000,
    rpc_url: str = DEFAULT_RPC_URL,
) -> tuple[str, bytes]:
    """Deploy a contract via AccountDeployer's deploy(salt, initcode).
    Returns (tx_hash, contract_address)."""
    calldata = (
        b'\x00\x00\x00\x01' +      # deploy selector
        salt +                       # salt (32 bytes)
        initcode                     # initcode (variable)
    )
    tx_hash = send_eip1559_tx(
        to=deployer_addr,
        value=0,
        data=calldata,
        gas_limit=gas_limit,
        rpc_url=rpc_url,
    )
    receipt = wait_for_receipt(tx_hash, rpc_url)
    status = int(receipt["status"], 16)
    gas_used = int(receipt["gasUsed"], 16)
    if status != 1:
        raise RuntimeError(f"CREATE2 deploy failed: status={status}, gas_used={gas_used}")

    # Extract created address from return data or compute it
    addr = compute_create2_address(deployer_addr, salt, initcode)
    print(f"  Deployed via CREATE2 at: 0x{addr.hex()}")
    print(f"  Gas used: {gas_used}")
    return tx_hash, addr


def fund_address(address: bytes, amount: int, rpc_url: str = DEFAULT_RPC_URL) -> str:
    """Send ETH to an address from the dev account.
    Uses higher gas limit to handle contract accounts with fallback functions."""
    tx_hash = send_eip1559_tx(
        to=address,
        value=amount,
        data=b'',
        gas_limit=100_000,
        rpc_url=rpc_url,
    )
    receipt = wait_for_receipt(tx_hash, rpc_url)
    return tx_hash
