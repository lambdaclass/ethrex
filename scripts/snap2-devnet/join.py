#!/usr/bin/env python3
"""Join a snap/2 devnet with a fresh ethrex node and assert it syncs via snap/2.

Every kurtosis participant starts at genesis, so none of them ever snap syncs.
This starts a node afterwards, against a chain that is already deep, which is
the only way to reach the code path.

There is no consensus client for the joiner. ethrex begins a sync when a
forkchoice update names a head it does not have, so this mirrors a seeder's head
over the Engine API directly. That is enough to drive the sync and avoids
standing up a second beacon node with checkpoint sync.

The assertions are the point. A snap/2 run that quietly fell back to snap/1
still ends with a synced node, so "it synced" proves nothing: this checks which
path executed, and fails if the node ever entered trie healing or never applied
an access list.

Usage:
    ./join.py                       # join enclave "snap2", run to completion
    ./join.py --enclave my-devnet
    ./join.py --source-el el-3-geth-lighthouse   # sync from geth instead
    ./join.py --keep                # leave the joiner running afterwards
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import hmac
import json
import os
import shutil
import subprocess
import sys
import time
import urllib.error
import urllib.request

ZERO_HASH = "0x" + "00" * 32

# The joiner's own knobs. These are what make a devnet-sized chain reach the
# interesting code; see fixtures/networks/snap2-devnet.yaml and the
# TEST-ONLY OVERRIDES block in crates/networking/p2p/snap/constants.rs.
#
# MIN_FULL_BLOCKS: below this ethrex full syncs instead, so at the stock 10_000
# a 6s devnet would need ~17 hours before snap sync even engages.
#
# SNAP_LIMIT: the pivot's lifetime in blocks. At the stock 128 the pivot lives
# ~25 minutes and a small devnet finishes downloading before it ever moves,
# which skips the access-list catch-up entirely — the one part of the sync with
# no other coverage.
#
# SECONDS_PER_BLOCK: must match the devnet's slot time. update_pivot estimates
# how far the chain moved by dividing elapsed time by it, so leaving it at the
# mainnet 12 while the devnet runs 6s slots makes every new pivot land short of
# the head, and it can be stale the moment it arrives.
DEFAULT_OVERRIDES = {
    "MIN_FULL_BLOCKS": "128",
    "SNAP_LIMIT": "16",
    "SECONDS_PER_BLOCK": "6",
}


def run(*args: str, check: bool = True) -> str:
    proc = subprocess.run(args, capture_output=True, text=True)
    if check and proc.returncode != 0:
        raise SystemExit(f"command failed: {' '.join(args)}\n{proc.stderr.strip()}")
    return proc.stdout.strip()


def rpc(url: str, method: str, params: list | None = None, token: str | None = None) -> dict:
    body = json.dumps(
        {"jsonrpc": "2.0", "id": 1, "method": method, "params": params or []}
    ).encode()
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    request = urllib.request.Request(url, data=body, headers=headers)
    with urllib.request.urlopen(request, timeout=20) as response:
        payload = json.load(response)
    if "error" in payload:
        raise RuntimeError(f"{method}: {payload['error']}")
    return payload["result"]


def engine_token(secret_hex: str) -> str:
    """An HS256 JWT for the Engine API. `iat` is all the spec requires."""
    secret = bytes.fromhex(secret_hex.removeprefix("0x").strip())

    def segment(value: dict) -> bytes:
        raw = json.dumps(value, separators=(",", ":")).encode()
        return base64.urlsafe_b64encode(raw).rstrip(b"=")

    signing_input = segment({"alg": "HS256", "typ": "JWT"}) + b"." + segment({"iat": int(time.time())})
    signature = hmac.new(secret, signing_input, hashlib.sha256).digest()
    return (signing_input + b"." + base64.urlsafe_b64encode(signature).rstrip(b"=")).decode()


def enclave_services(enclave: str) -> dict[str, str]:
    """Service name -> container name, for the running enclave."""
    raw = run("docker", "ps", "--format", "{{.Names}}")
    services = {}
    for container in raw.splitlines():
        # Kurtosis names containers "<service>--<uuid>".
        if "--" in container:
            services[container.rsplit("--", 1)[0]] = container
    if not services:
        raise SystemExit(f"no running containers found; is enclave {enclave!r} up?")
    return services


def container_ip(container: str) -> str:
    return run(
        "docker", "inspect", "-f",
        "{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}", container,
    )


def container_network(container: str) -> str:
    return run(
        "docker", "inspect", "-f",
        "{{range $k, $v := .NetworkSettings.Networks}}{{$k}}{{end}}", container,
    )


def pick_source(services: dict[str, str], requested: str | None) -> str:
    if requested:
        if requested not in services:
            raise SystemExit(
                f"service {requested!r} not found. Available:\n  "
                + "\n  ".join(sorted(services))
            )
        return requested
    candidates = sorted(
        name for name in services
        if name.startswith("el-") and ("ethrex" in name or "geth" in name)
    )
    if not candidates:
        raise SystemExit("no execution-layer service found in the enclave")
    return candidates[0]


def wait_for_depth(url: str, minimum: int, timeout: int) -> int:
    """Block until the seeder's chain is deep enough for snap sync to engage."""
    deadline = time.time() + timeout
    last = -1
    while time.time() < deadline:
        head = int(rpc(url, "eth_blockNumber"), 16)
        if head != last:
            print(f"  seeder head: {head}/{minimum}")
            last = head
        if head >= minimum:
            return head
        time.sleep(10)
    raise SystemExit(
        f"seeder only reached block {last} in {timeout}s; it must pass "
        f"MIN_FULL_BLOCKS ({minimum}) or the joiner will full sync instead"
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--enclave", default="snap2")
    parser.add_argument("--source-el", default=None, help="EL service to sync from")
    parser.add_argument("--image", default="ethrex:sync-test")
    parser.add_argument("--name", default="snap2-joiner")
    parser.add_argument("--timeout", type=int, default=3600, help="seconds to allow for the sync")
    parser.add_argument("--depth-timeout", type=int, default=3600, help="seconds to wait for chain depth")
    parser.add_argument("--keep", action="store_true", help="leave the joiner running")
    args = parser.parse_args()

    if not shutil.which("docker"):
        raise SystemExit("docker not found")

    services = enclave_services(args.enclave)
    source = pick_source(services, args.source_el)
    source_container = services[source]
    source_ip = container_ip(source_container)
    network = container_network(source_container)
    source_rpc = f"http://{source_ip}:8545"
    print(f"seeding from {source} ({source_ip}) on network {network}")

    overrides = {**DEFAULT_OVERRIDES, **{k: v for k, v in os.environ.items() if k in DEFAULT_OVERRIDES}}
    minimum = int(overrides["MIN_FULL_BLOCKS"])

    # The joiner has to start behind a chain that is already deep, or ethrex
    # picks full sync and never touches the snap path.
    head = wait_for_depth(source_rpc, minimum + 32, args.depth_timeout)
    enode = rpc(source_rpc, "admin_nodeInfo")["enode"].replace("127.0.0.1", source_ip)

    # Reuse the seeder's genesis and JWT rather than reconstructing them.
    workdir = f"/tmp/{args.name}"
    shutil.rmtree(workdir, ignore_errors=True)
    os.makedirs(workdir, exist_ok=True)
    for remote, local in (("/jwt/jwtsecret", "jwtsecret"), ("/network-configs/genesis.json", "genesis.json")):
        for candidate in (source_container, *services.values()):
            try:
                run("docker", "cp", f"{candidate}:{remote}", f"{workdir}/{local}")
                break
            except SystemExit:
                continue
        else:
            raise SystemExit(f"could not find {remote} in any enclave container")
    secret = open(f"{workdir}/jwtsecret").read().strip()

    run("docker", "rm", "-f", args.name, check=False)
    print(f"starting joiner {args.name} from {args.image}")
    run(
        "docker", "run", "-d", "--name", args.name, "--network", network,
        *[arg for key, value in overrides.items() for arg in ("-e", f"{key}={value}")],
        "-v", f"{workdir}:/joiner",
        args.image,
        "--network", "/joiner/genesis.json",
        "--datadir", "/joiner/data",
        "--syncmode", "snap",
        "--bootnodes", enode,
        "--http.addr", "0.0.0.0",
        "--http.api", "eth,net,web3,debug,admin",
        "--authrpc.addr", "0.0.0.0",
        "--authrpc.jwtsecret", "/joiner/jwtsecret",
        "--log.level", "info",
    )
    joiner_ip = container_ip(args.name)
    joiner_rpc = f"http://{joiner_ip}:8545"
    joiner_engine = f"http://{joiner_ip}:8551"
    print(f"joiner at {joiner_ip}; head to sync to: {head}")

    # Mirror the seeder's head so the joiner has something to sync towards, and
    # watch which path it takes.
    deadline = time.time() + args.timeout
    phases: list[str] = []
    healed = False
    last_report = 0.0
    try:
        while time.time() < deadline:
            try:
                block = rpc(source_rpc, "eth_getBlockByNumber", ["latest", False])
                # safe and finalized are left zero, as a consensus client does
                # before it knows finality. A non-zero hash the joiner has never
                # seen is looked up and order-checked against the head, and that
                # fails the forkchoice update outright instead of reporting
                # SYNCING — which is the answer that starts the sync.
                rpc(
                    joiner_engine, "engine_forkchoiceUpdatedV4",
                    [{"headBlockHash": block["hash"], "safeBlockHash": ZERO_HASH,
                      "finalizedBlockHash": ZERO_HASH}, None],
                    token=engine_token(secret),
                )
            except (urllib.error.URLError, RuntimeError, ConnectionError) as err:
                print(f"  (forkchoice not accepted yet: {err})")
                time.sleep(5)
                continue

            try:
                diag = rpc(joiner_rpc, "admin_syncStatus")
            except (urllib.error.URLError, RuntimeError, ConnectionError):
                time.sleep(5)
                continue

            phase = diag.get("current_phase", "?")
            if not phases or phases[-1] != phase:
                phases.append(phase)
                print(f"  phase: {phase}")
            if phase == "healing":
                healed = True

            if time.time() - last_report > 30:
                print(
                    f"  replayed={diag.get('snap2_blocks_replayed', 0)} "
                    f"bal_requests={diag.get('snap2_bal_requests_sent', 0)} "
                    f"unavailable={diag.get('snap2_bals_unavailable', 0)} "
                    f"validation_failures={diag.get('snap2_validation_failures', 0)}"
                )
                last_report = time.time()

            syncing = rpc(joiner_rpc, "eth_syncing")
            if syncing is False and int(rpc(joiner_rpc, "eth_blockNumber"), 16) >= head:
                break
            time.sleep(5)
        else:
            print(f"\nFAIL: joiner did not sync within {args.timeout}s", file=sys.stderr)
            return 1

        diag = rpc(joiner_rpc, "admin_syncStatus")
        joiner_head = int(rpc(joiner_rpc, "eth_blockNumber"), 16)
        print("\n--- result ---")
        print(f"  head:            {joiner_head} (target {head})")
        print(f"  phases:          {' -> '.join(phases)}")
        print(f"  blocks replayed: {diag.get('snap2_blocks_replayed', 0)}")
        print(f"  validation fail: {diag.get('snap2_validation_failures', 0)}")

        failures = []
        # The trap this whole exercise exists to avoid: a snap/1 fallback ends
        # with a synced node too, so it would otherwise read as a pass.
        if healed:
            failures.append("entered trie healing, so it fell back to snap/1")
        if not any(p.startswith("snap2") for p in phases):
            failures.append("never entered a snap/2 phase")
        # Without a pivot move the access-list catch-up never runs, which is
        # most of what is untested. Lower SNAP_LIMIT if this trips.
        if diag.get("snap2_blocks_replayed", 0) == 0:
            failures.append("no access lists applied: the pivot never moved")
        if diag.get("snap2_validation_failures", 0):
            failures.append("access-list validation failures")

        if failures:
            print("\nFAIL:", file=sys.stderr)
            for failure in failures:
                print(f"  - {failure}", file=sys.stderr)
            return 1
        print("\nPASS: synced via snap/2 with access-list catch-up")
        return 0
    finally:
        if not args.keep:
            run("docker", "rm", "-f", args.name, check=False)
        else:
            print(f"\njoiner left running as {args.name}; logs: docker logs -f {args.name}")


if __name__ == "__main__":
    sys.exit(main())
