#!/usr/bin/env python3
"""Verify EIP-8141 demo contracts on Blockscout.

Uses the v1 API (Etherscan-compatible) with standard-json-input format,
which correctly passes viaIR to the solc compiler. The v2 API's
standard-json-input route is unavailable in some Blockscout versions.

Called by verify-contracts.sh with:
    python3 verify-contracts.py <blockscout_url> <compiler_version> <contracts_dir>
"""
import json
import os
import sys
import time
import urllib.parse
import urllib.request


def read_file(path: str) -> str:
    with open(path) as f:
        return f.read()


def verify_contract(
    blockscout_url: str,
    compiler_version: str,
    address: str,
    contract_name: str,
    std_input: dict,
) -> bool:
    """Submit verification and poll for result."""
    print(f"\n=== Verifying {contract_name} at {address} ===")

    # Check if already verified
    try:
        check_url = f"{blockscout_url}/api/v2/smart-contracts/{address}"
        check_resp = urllib.request.urlopen(check_url)
        check_data = json.loads(check_resp.read().decode())
        if check_data.get("name") == contract_name:
            print(f"  Already verified.")
            return True
    except Exception:
        pass  # Contract not in smart_contracts table yet — proceed with verification

    params = {
        "module": "contract",
        "action": "verifysourcecode",
        "codeformat": "solidity-standard-json-input",
        "addressHash": address,
        "contractaddress": address,
        "compilerversion": compiler_version,
        "sourceCode": json.dumps(std_input),
        "contractname": f"{list(std_input['sources'].keys())[0]}:{contract_name}",
        "constructorArguements": "",
    }

    data = urllib.parse.urlencode(params).encode()
    req = urllib.request.Request(
        f"{blockscout_url}/api", data=data, method="POST"
    )
    req.add_header("Content-Type", "application/x-www-form-urlencoded")

    try:
        resp = urllib.request.urlopen(req)
        result = json.loads(resp.read().decode())
    except urllib.error.HTTPError as e:
        print(f"  FAILED: HTTP {e.code}")
        print(f"  {e.read().decode()[:200]}")
        return False

    if result.get("status") == "0":
        msg = result.get("message", "")
        if "already verified" in msg.lower():
            print(f"  Already verified.")
            return True
        print(f"  FAILED: {msg}")
        return False

    guid = result.get("result", "")
    print(f"  Submitted (guid: {guid})")

    # Poll for verification result
    for attempt in range(12):
        time.sleep(5)
        check_url = (
            f"{blockscout_url}/api?"
            f"module=contract&action=checkverifystatus&guid={guid}"
        )
        try:
            check_resp = urllib.request.urlopen(check_url)
            check_result = json.loads(check_resp.read().decode())
            status_msg = check_result.get("result", "")
            if "pass" in status_msg.lower() or "verified" in status_msg.lower():
                print(f"  OK — {contract_name} verified.")
                return True
            if "fail" in status_msg.lower():
                print(f"  FAILED: {status_msg}")
                return False
            print(f"  Waiting... ({status_msg})")
        except Exception as e:
            print(f"  Poll error: {e}")

    print("  TIMEOUT — verification did not complete in 60s")
    return False


def main():
    if len(sys.argv) != 4:
        print(f"Usage: {sys.argv[0]} <blockscout_url> <compiler_version> <contracts_dir>")
        sys.exit(1)

    blockscout_url = sys.argv[1]
    compiler_version = sys.argv[2]
    contracts_dir = sys.argv[3]

    results = {}

    # ── MockERC20 ──────────────────────────────────────────────────
    std_input = {
        "language": "Solidity",
        "sources": {
            "contracts/src/MockERC20.sol": {
                "content": read_file(os.path.join(contracts_dir, "src/MockERC20.sol"))
            }
        },
        "settings": {
            "optimizer": {"enabled": True, "runs": 200},
            "viaIR": True,
            "outputSelection": {"*": {"*": ["evm.bytecode", "evm.deployedBytecode", "abi"]}},
        },
    }
    results["MockERC20"] = verify_contract(
        blockscout_url,
        compiler_version,
        "0x1000000000000000000000000000000000000002",
        "MockERC20",
        std_input,
    )

    # ── WebAuthnVerifier ───────────────────────────────────────────
    sources = {}
    for name, rel_path in [
        ("contracts/src/WebAuthnVerifier.sol", "src/WebAuthnVerifier.sol"),
        ("contracts/lib/ECDSA.sol", "lib/ECDSA.sol"),
        ("contracts/lib/WebAuthnP256.sol", "lib/WebAuthnP256.sol"),
        ("contracts/lib/P256.sol", "lib/P256.sol"),
        ("contracts/deps/solady/src/utils/Base64.sol", "deps/solady/src/utils/Base64.sol"),
    ]:
        sources[name] = {"content": read_file(os.path.join(contracts_dir, rel_path))}

    std_input = {
        "language": "Solidity",
        "sources": sources,
        "settings": {
            "remappings": ["@solady/=contracts/deps/solady/"],
            "optimizer": {"enabled": True, "runs": 200},
            "viaIR": True,
            "outputSelection": {"*": {"*": ["evm.bytecode", "evm.deployedBytecode", "abi"]}},
        },
    }
    results["WebAuthnVerifier"] = verify_contract(
        blockscout_url,
        compiler_version,
        "0x1000000000000000000000000000000000000004",
        "WebAuthnVerifier",
        std_input,
    )

    # ── Summary ────────────────────────────────────────────────────
    print("\n" + "─" * 56)
    for name, ok in results.items():
        status = "VERIFIED" if ok else "FAILED"
        print(f"  {name}: {status}")
    print()
    print("NOT verifiable (Yul with custom opcodes):")
    print("  GasSponsor          (0x1000000000000000000000000000000000000001)")
    print("  WebAuthnP256Account (0x1000000000000000000000000000000000000003)")
    print("─" * 56)

    if not all(results.values()):
        sys.exit(1)


if __name__ == "__main__":
    main()
