#!/usr/bin/env python3
"""Drive an ethrex node along the Daisugi chain without a consensus client.

Reads the head and finalized block hashes from a Daisugi RPC endpoint and sends
them to ethrex as `engine_forkchoiceUpdatedV3` calls over the authenticated engine
API. ethrex treats an unknown head as a sync target and fetches the missing blocks
from its devp2p peers, validating every one, so this is enough to sync to the tip
and keep following it. No payload attributes are sent: the node never builds.

usage: follow_head.py --jwt-secret PATH [--engine URL] [--local-rpc URL] [--rpc URL]
                      [--interval SECONDS] [--target BLOCK_NUMBER]
"""

import argparse
import base64
import hashlib
import hmac
import http.client
import json
import sys
import time
import urllib.request


def b64url(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()


def engine_token(secret: bytes) -> str:
    """An HS256 JWT with a fresh `iat`, as the engine API requires."""
    header = b64url(json.dumps({"alg": "HS256", "typ": "JWT"}).encode())
    claims = b64url(json.dumps({"iat": int(time.time())}).encode())
    signing_input = f"{header}.{claims}".encode()
    signature = b64url(hmac.new(secret, signing_input, hashlib.sha256).digest())
    return f"{header}.{claims}.{signature}"


def rpc(url: str, method: str, params: list, token: str | None = None):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    headers = {"content-type": "application/json"}
    if token:
        headers["authorization"] = f"Bearer {token}"
    request = urllib.request.Request(url, data=body, headers=headers)
    # Every call is a small read or a forkchoice update. One still pending after
    # 10 seconds has hung, and waiting longer only stops the node's head moving.
    with urllib.request.urlopen(request, timeout=10) as response:
        reply = json.loads(response.read())
    if "error" in reply:
        raise RuntimeError(f"{method}: {reply['error']}")
    return reply["result"]


def block(url: str, tag: str) -> dict:
    result = rpc(url, "eth_getBlockByNumber", [tag, False])
    if result is None:
        raise RuntimeError(f"no {tag} block")
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--jwt-secret", required=True, help="hex JWT secret file ethrex uses")
    parser.add_argument("--engine", default="http://127.0.0.1:8551")
    parser.add_argument("--local-rpc", default="http://127.0.0.1:8545", help="ethrex's own JSON-RPC")
    parser.add_argument("--rpc", default="https://daisugi.fyi/rpc")
    parser.add_argument("--interval", type=float, default=2.0)
    parser.add_argument(
        "--target",
        type=int,
        help="sync to this block number and stay there, instead of following the head",
    )
    args = parser.parse_args()

    secret = bytes.fromhex(open(args.jwt_secret).read().strip().removeprefix("0x"))
    last_status = None
    while True:
        try:
            if args.target is not None:
                head = finalized = block(args.rpc, hex(args.target))
            else:
                head = block(args.rpc, "latest")
                finalized = block(args.rpc, "finalized")
            state = {
                "headBlockHash": head["hash"],
                "safeBlockHash": finalized["hash"],
                "finalizedBlockHash": finalized["hash"],
            }
            result = rpc(
                args.engine,
                "engine_forkchoiceUpdatedV3",
                [state, None],
                token=engine_token(secret),
            )
            status = result["payloadStatus"]["status"]
            local = int(rpc(args.local_rpc, "eth_blockNumber", []), 16)
            line = (
                f"head {int(head['number'], 16)} finalized {int(finalized['number'], 16)} "
                f"status {status} local {local}"
            )
            if status != last_status or status == "VALID":
                print(time.strftime("%H:%M:%S"), line, flush=True)
            last_status = status
            if status == "INVALID":
                print(json.dumps(result, indent=1), flush=True)
        # OSError covers URLError, timeouts, and refused or dropped connections,
        # such as a node hanging up while it restarts; HTTPException covers a
        # response cut short. Retry instead of leaving the node without a head.
        except (OSError, http.client.HTTPException, RuntimeError, KeyError, ValueError) as error:
            print(time.strftime("%H:%M:%S"), f"error: {error}", flush=True)
        time.sleep(args.interval)


if __name__ == "__main__":
    sys.exit(main())
