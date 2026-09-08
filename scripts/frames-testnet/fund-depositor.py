#!/usr/bin/env python3
"""Fund a depositor with the stake for the validator slots it was granted.

A deposit slot (a gating token, see INSTALL.md section 11) lets an address deposit; it
does not give it the 32 ETH per validator that the deposit itself moves, and the faucet's
drip is far below that. This sends the stake from one of the deployment's rich accounts.

Runs where the keys live, inside the faucet image so no extra tooling is installed:

    set -a; . ~/frames-testnet-keys.env; set +a
    docker run --rm --network host -e K="$RICH_01_KEY" -e TO=<depositor> -e AMOUNT_ETH=97 \
      -v $PWD/fund-depositor.py:/fund-depositor.py:ro --entrypoint python \
      frames-faucet:latest /fund-depositor.py

Environment: K (sender private key), TO (recipient), AMOUNT_ETH (default 97 = 3 x 32 + gas),
RPC_URL (default http://127.0.0.1:36003, the host's first execution node).
"""
import json
import os
import time
import urllib.request

from eth_account import Account

RPC = os.environ.get("RPC_URL", "http://127.0.0.1:36003")


def call(method, params):
    req = urllib.request.Request(
        RPC,
        data=json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode(),
        headers={"content-type": "application/json"},
    )
    reply = json.load(urllib.request.urlopen(req, timeout=20))
    if "error" in reply:
        raise SystemExit(f"{method}: {reply['error']}")
    return reply["result"]


def main():
    acct = Account.from_key(os.environ["K"])
    to = os.environ["TO"]
    amount_eth = int(os.environ.get("AMOUNT_ETH", "97"))
    chain_id = int(call("eth_chainId", []), 16)
    nonce = int(call("eth_getTransactionCount", [acct.address, "pending"]), 16)
    base_fee = int(call("eth_getBlockByNumber", ["latest", False])["baseFeePerGas"], 16)
    tx = {
        "type": 2,
        "chainId": chain_id,
        "nonce": nonce,
        "to": to,
        "value": amount_eth * 10**18,
        "gas": 21_000,
        "maxFeePerGas": max(2 * base_fee, 10**9) + 10**9,
        "maxPriorityFeePerGas": 10**9,
    }
    tx_hash = call("eth_sendRawTransaction", ["0x" + acct.sign_transaction(tx).raw_transaction.hex()])
    print(f"sent {tx_hash} from {acct.address} amount {amount_eth} ETH")
    for _ in range(20):
        receipt = call("eth_getTransactionReceipt", [tx_hash])
        if receipt:
            print(f"mined in block {int(receipt['blockNumber'], 16)} status {receipt['status']}")
            break
        time.sleep(3)
    else:
        print("not mined within the wait; check the hash above")
    print("recipient balance:", int(call("eth_getBalance", [to, "latest"]), 16) / 1e18, "ETH")


if __name__ == "__main__":
    main()
