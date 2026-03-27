#!/usr/bin/env python3
"""Verify the NativeRollup contract in a local Blockscout instance.

Usage:
    python3 scripts/blockscout_verify_native_rollup.py <contract_address> [blockscout_url]

The script flattens NativeRollup.sol + MPTProof.sol, submits them to Blockscout's
standard-input verification API, and polls until verification completes.
"""

import http.client
import json
import os
import sys
import time
import uuid

COMPILER_VERSION = "v0.8.31+commit.fd3a2265"
CONTRACT_NAME = "NativeRollup"
CONTRACTS_DIR = os.path.join(
    os.path.dirname(__file__), "..", "crates", "vm", "levm", "contracts"
)


def flatten_source():
    mpt_path = os.path.join(CONTRACTS_DIR, "MPTProof.sol")
    nr_path = os.path.join(CONTRACTS_DIR, "NativeRollup.sol")

    mpt_source = open(mpt_path).read()
    nr_lines = open(nr_path).readlines()

    # Remove import, duplicate SPDX and pragma from NativeRollup
    nr_clean = []
    for line in nr_lines:
        stripped = line.strip()
        if stripped.startswith("import") or stripped.startswith("// SPDX") or stripped.startswith("pragma solidity"):
            continue
        nr_clean.append(line)

    return mpt_source + "\n" + "".join(nr_clean)


def build_standard_input(source):
    return json.dumps({
        "language": "Solidity",
        "sources": {"NativeRollup.sol": {"content": source}},
        "settings": {
            "optimizer": {"enabled": True, "runs": 999999},
            "viaIR": True,
            "metadata": {"bytecodeHash": "none", "appendCBOR": False},
            "outputSelection": {"*": {"*": ["abi", "evm.bytecode", "evm.deployedBytecode"]}},
        },
    })


def submit_verification(host, port, address, standard_input):
    boundary = uuid.uuid4().hex

    body = ""
    for name, value in [
        ("compiler_version", COMPILER_VERSION),
        ("contract_name", CONTRACT_NAME),
    ]:
        body += (
            f"--{boundary}\r\n"
            f'Content-Disposition: form-data; name="{name}"\r\n\r\n'
            f"{value}\r\n"
        )
    body += (
        f"--{boundary}\r\n"
        f'Content-Disposition: form-data; name="files[0]"; filename="input.json"\r\n'
        f"Content-Type: application/json\r\n\r\n"
        f"{standard_input}\r\n"
    )
    body += f"--{boundary}--\r\n"

    conn = http.client.HTTPConnection(host, port)
    conn.request(
        "POST",
        f"/api/v2/smart-contracts/{address}/verification/via/standard-input",
        body.encode(),
        {"Content-Type": f"multipart/form-data; boundary={boundary}"},
    )
    resp = conn.getresponse()
    result = json.loads(resp.read())
    conn.close()

    if resp.status != 200:
        print(f"Error {resp.status}: {result}", file=sys.stderr)
        sys.exit(1)

    return result


def check_verified(host, port, address):
    conn = http.client.HTTPConnection(host, port)
    conn.request("GET", f"/api?module=contract&action=getabi&address={address}")
    resp = conn.getresponse()
    result = json.loads(resp.read())
    conn.close()
    return result.get("message") == "OK"


def main():
    if len(sys.argv) < 2:
        print(f"Usage: {sys.argv[0]} <contract_address> [blockscout_url]", file=sys.stderr)
        sys.exit(1)

    address = sys.argv[1]
    blockscout_url = sys.argv[2] if len(sys.argv) > 2 else "http://localhost"

    # Parse host:port from URL
    url = blockscout_url.replace("http://", "").replace("https://", "")
    if ":" in url:
        host, port = url.split(":")
        port = int(port)
    else:
        host = url
        port = 80

    print(f"Flattening NativeRollup.sol + MPTProof.sol...")
    source = flatten_source()

    print(f"Submitting verification for {address} to {blockscout_url}...")
    standard_input = build_standard_input(source)
    result = submit_verification(host, port, address, standard_input)
    print(f"  {result.get('message', result)}")

    # Poll for completion
    for i in range(30):
        time.sleep(1)
        if check_verified(host, port, address):
            print("Contract verified successfully!")
            return

    print("Verification did not complete within 30 seconds. Check Blockscout logs.", file=sys.stderr)
    sys.exit(1)


if __name__ == "__main__":
    main()
