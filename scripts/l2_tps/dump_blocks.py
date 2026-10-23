#!/usr/bin/env python3
"""Write blocks.csv (number, timestamp, tx count, gas used, gas limit, base fee)
for every L2 block of a run. Usage: dump_blocks.py <run_dir> [<l2_rpc_url>]"""
import json
import sys
import urllib.request

run_dir = sys.argv[1]
url = sys.argv[2] if len(sys.argv) > 2 else "http://localhost:1729"


def rpc(method, params):
    body = json.dumps({"jsonrpc": "2.0", "method": method, "params": params, "id": 1}).encode()
    req = urllib.request.Request(url, data=body, headers={"content-type": "application/json"})
    return json.load(urllib.request.urlopen(req, timeout=10))["result"]


latest = int(rpc("eth_blockNumber", []), 16)
with open(f"{run_dir}/blocks.csv", "w") as out:
    out.write("number,timestamp,tx_count,gas_used,gas_limit,base_fee\n")
    for number in range(1, latest + 1):
        block = rpc("eth_getBlockByNumber", [hex(number), False])
        fields = [
            number,
            int(block["timestamp"], 16),
            len(block["transactions"]),
            int(block["gasUsed"], 16),
            int(block["gasLimit"], 16),
            int(block["baseFeePerGas"], 16),
        ]
        out.write(",".join(str(f) for f in fields) + "\n")
print(f"blocks dumped: {latest}")
