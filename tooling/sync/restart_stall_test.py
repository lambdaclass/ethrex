#!/usr/bin/env python3
"""Test for snap sync restart stall bug using eth-docker.

Reproduces the issue where ethrex stalls downloading headers after a restart.
Assumes eth-docker is already cloned and configured with ethrex as the EL client.

Flow:
  Phase 1: Fresh snap sync (terminate + start, wait for completion)
  Phase 2: Stop only execution client, restart it, monitor for stall

Prerequisites:
  - eth-docker cloned (default: ~/eth-docker)
  - .env configured with ethrex (COMPOSE_FILE=prysm.yml:ethrex.yml:el-shared.yml, NETWORK=hoodi, etc.)
  - Slack webhooks in eth-docker's .env or exported as env vars (optional)

Usage:
  python3 restart_stall_test.py --eth-docker-dir ~/eth-docker
  python3 restart_stall_test.py --eth-docker-dir ~/eth-docker --restart-count 5
  python3 restart_stall_test.py --eth-docker-dir ~/eth-docker --skip-phase1
  python3 restart_stall_test.py --eth-docker-dir ~/eth-docker --no-slack
"""

import argparse
import os
import socket
import subprocess
import sys
import time
from datetime import datetime
from pathlib import Path

import requests

# Timeouts (all in seconds), configurable via env vars
SYNC_TIMEOUT = int(os.environ.get("SYNC_TIMEOUT", 8 * 60 * 60))  # default 8h
BLOCK_PROCESSING_DURATION = int(os.environ.get("BLOCK_PROCESSING_DURATION", 22 * 60))  # default 22m
BLOCK_STALL_TIMEOUT = int(os.environ.get("BLOCK_STALL_TIMEOUT", 10 * 60))  # default 10m
RESTART_STALL_TIMEOUT = int(os.environ.get("RESTART_STALL_TIMEOUT", 15 * 60))  # default 15m
NODE_STARTUP_TIMEOUT = int(os.environ.get("NODE_STARTUP_TIMEOUT", 5 * 60))  # default 5m
CHECK_INTERVAL = int(os.environ.get("CHECK_INTERVAL", 10))

SCRIPT_DIR = Path(__file__).resolve().parent
LOGS_DIR = SCRIPT_DIR / "restart_stall_logs"


def configure_eth_docker(eth_docker_dir: str, network: str, fee_recipient: str = "", slack_success: str = "", slack_failed: str = ""):
    """Write eth-docker .env configured for ethrex + Prysm.

    Copies default.env as base and overrides the key settings.
    """
    default_env = os.path.join(eth_docker_dir, "default.env")
    env_file = os.path.join(eth_docker_dir, ".env")

    if not os.path.isfile(default_env):
        print(f"Error: default.env not found at {default_env}")
        sys.exit(1)

    # Read default.env as base
    with open(default_env) as f:
        lines = f.readlines()

    # Map network to checkpoint sync URL
    checkpoint_urls = {
        "mainnet": "https://mainnet.checkpoint.sigp.io",
        "hoodi": "https://hoodi.checkpoint.sigp.io",
        "holesky": "https://holesky.checkpoint.sigp.io",
        "sepolia": "https://sepolia.checkpoint.sigp.io",
    }
    checkpoint_url = checkpoint_urls.get(network, "")

    # Settings to override
    overrides = {
        "COMPOSE_FILE": "prysm.yml:ethrex.yml:el-shared.yml",
        "NETWORK": network,
        "ETHREX_DOCKERFILE": "Dockerfile.binary",
        "ETHREX_DOCKER_REPO": "ghcr.io/lambdaclass/ethrex",
        "ETHREX_DOCKER_TAG": "latest",
    }
    if checkpoint_url:
        overrides["CHECKPOINT_SYNC_URL"] = checkpoint_url
    if fee_recipient:
        overrides["FEE_RECIPIENT"] = fee_recipient
    if slack_success:
        overrides["SLACK_WEBHOOK_URL_SUCCESS"] = slack_success
    if slack_failed:
        overrides["SLACK_WEBHOOK_URL_FAILED"] = slack_failed

    applied = set()
    new_lines = []
    for line in lines:
        stripped = line.strip()
        # Match lines like KEY=value or #KEY=value
        for key, value in overrides.items():
            if stripped.startswith(f"{key}=") or stripped.startswith(f"#{key}="):
                line = f"{key}={value}\n"
                applied.add(key)
                break
        new_lines.append(line)

    # Append any overrides that weren't found in default.env
    for key, value in overrides.items():
        if key not in applied:
            new_lines.append(f"{key}={value}\n")

    with open(env_file, "w") as f:
        f.writelines(new_lines)

    print(f"  Wrote {env_file}")
    print(f"    COMPOSE_FILE=prysm.yml:ethrex.yml:el-shared.yml")
    print(f"    NETWORK={network}")
    if checkpoint_url:
        print(f"    CHECKPOINT_SYNC_URL={checkpoint_url}")
    if fee_recipient:
        print(f"    FEE_RECIPIENT={fee_recipient}")
    print(f"    ETHREX_DOCKER_REPO=ghcr.io/lambdaclass/ethrex")
    print(f"    ETHREX_DOCKER_TAG=latest")


def load_env_file(env_path: str):
    """Load variables from an .env file into os.environ (without overriding existing)."""
    if not os.path.exists(env_path):
        return
    with open(env_path) as f:
        for line in f:
            line = line.strip()
            if line and not line.startswith("#"):
                key, _, value = line.partition("=")
                key, value = key.strip(), value.strip()
                if key and key not in os.environ:
                    os.environ[key] = value


def fmt_time(secs: float) -> str:
    secs = int(abs(secs))
    h, m, s = secs // 3600, (secs % 3600) // 60, secs % 60
    return " ".join(f"{v}{u}" for v, u in [(h, "h"), (m, "m"), (s, "s")] if v or (not h and not m))


def git_info(cwd: str = None) -> tuple[str, str]:
    try:
        commit = subprocess.check_output(
            ["git", "rev-parse", "--short", "HEAD"], stderr=subprocess.DEVNULL, cwd=cwd
        ).decode().strip()
    except Exception:
        commit = "unknown"
    try:
        branch = subprocess.check_output(
            ["git", "rev-parse", "--abbrev-ref", "HEAD"], stderr=subprocess.DEVNULL, cwd=cwd
        ).decode().strip()
    except Exception:
        branch = "unknown"
    return branch, commit


def rpc_call(url: str, method: str, params=None):
    try:
        payload = {"jsonrpc": "2.0", "method": method, "params": params or [], "id": 1}
        resp = requests.post(url, json=payload, timeout=5)
        return resp.json().get("result")
    except Exception:
        return None


def rpc_block_number(url: str):
    result = rpc_call(url, "eth_blockNumber")
    if result:
        return int(result, 16)
    return None


def docker_compose_in_ethd(eth_docker_dir: str, *args, check: bool = False) -> subprocess.CompletedProcess:
    """Run a docker compose command in the eth-docker directory.

    Uses eth-docker's .env for COMPOSE_FILE so the right yml files are picked up.
    If check=True, raises on non-zero exit code.
    """
    cmd = ["docker", "compose"] + list(args)
    print(f"  $ {' '.join(cmd)}")
    result = subprocess.run(cmd, cwd=eth_docker_dir, capture_output=True, text=True)
    if check and result.returncode != 0:
        print(f"  FAILED (exit code {result.returncode})")
        if result.stderr:
            print(f"  stderr: {result.stderr.strip()}")
        raise RuntimeError(f"docker compose {' '.join(args)} failed with exit code {result.returncode}")
    return result


def wipe_data_volumes(eth_docker_dir: str):
    """Remove and recreate containers and their data volumes for a fresh start.

    Stops and removes the execution and consensus containers, then removes all
    data volumes (EL, consensus, validator) while preserving the JWT secret.
    """
    docker_compose_in_ethd(eth_docker_dir, "rm", "-f", "-s", "execution", "consensus", check=True)

    project = os.path.basename(eth_docker_dir)
    volumes = [
        f"{project}_ethrex-el-data",
        f"{project}_prysmconsensus-data",
        f"{project}_prysmvalidator-data",
    ]
    for volume in volumes:
        print(f"  Removing volume {volume}...")
        result = subprocess.run(
            ["docker", "volume", "rm", volume],
            capture_output=True, text=True,
        )
        if result.returncode != 0:
            # Volume may not exist yet on first run; warn but don't fail
            print(f"  Warning: could not remove {volume}: {result.stderr.strip()}")


def slack_notify(message: str, success: bool, details: str = "", ethrex_dir: str = None):
    """Send a Slack notification using the configured webhooks."""
    url = os.environ.get("SLACK_WEBHOOK_URL_SUCCESS" if success else "SLACK_WEBHOOK_URL_FAILED")
    if not url:
        print("  [no slack webhook configured, skipping notification]")
        return

    branch, commit = git_info(cwd=ethrex_dir)
    hostname = socket.gethostname()

    blocks = [
        {"type": "header", "text": {"type": "plain_text", "text": message}},
        {
            "type": "section",
            "text": {
                "type": "mrkdwn",
                "text": (
                    f"*Host:* `{hostname}`\n"
                    f"*Branch:* `{branch}`\n"
                    f"*Commit:* <https://github.com/lambdaclass/ethrex/commit/{commit}|{commit}>\n"
                    f"*Test:* Restart stall reproduction (eth-docker)"
                ),
            },
        },
    ]
    if details:
        blocks.append({"type": "section", "text": {"type": "mrkdwn", "text": details}})

    try:
        requests.post(url, json={"blocks": blocks}, timeout=10)
    except Exception as e:
        print(f"  Failed to send Slack notification: {e}")


def save_ethd_logs(eth_docker_dir: str, run_dir: Path, suffix: str = ""):
    """Save execution and consensus logs from eth-docker."""
    for service in ["execution", "consensus"]:
        log_file = run_dir / f"{service}{suffix}.log"
        try:
            result = docker_compose_in_ethd(eth_docker_dir, "logs", "--no-color", service)
            log_file.write_text(result.stdout + result.stderr)
            print(f"  Saved logs: {log_file}")
        except Exception as e:
            print(f"  Failed to save {service} logs: {e}")


def wait_for_node(rpc_url: str, timeout: int) -> bool:
    """Wait for the node to respond to RPC calls."""
    print(f"  Waiting for node to respond at {rpc_url}...")
    start = time.time()
    while time.time() - start < timeout:
        if rpc_call(rpc_url, "eth_blockNumber") is not None:
            print(f"  Node is up ({fmt_time(time.time() - start)} elapsed)")
            return True
        time.sleep(CHECK_INTERVAL)
    print(f"  Node did not respond within {fmt_time(timeout)}")
    return False


def wait_for_sync(rpc_url: str, timeout: int) -> tuple[bool, float]:
    """Wait for snap sync to complete.

    Returns (success, sync_time_seconds).
    """
    print(f"  Waiting for sync to complete (timeout: {fmt_time(timeout)})...")
    start = time.time()
    last_status_print = 0

    while time.time() - start < timeout:
        syncing = rpc_call(rpc_url, "eth_syncing")
        elapsed = time.time() - start

        if syncing is False:
            print(f"  Sync completed in {fmt_time(elapsed)}")
            return True, elapsed

        if syncing is None:
            if time.time() - last_status_print > 60:
                print(f"  [{fmt_time(elapsed)}] Node not responding...")
                last_status_print = time.time()
        else:
            if time.time() - last_status_print > 60:
                block = rpc_block_number(rpc_url)
                block_str = f"block {block}" if block else "unknown block"
                print(f"  [{fmt_time(elapsed)}] Still syncing... {block_str}")
                last_status_print = time.time()

        time.sleep(CHECK_INTERVAL)

    return False, time.time() - start


def wait_for_block_progress(rpc_url: str, duration: int, stall_timeout: int) -> tuple[bool, int]:
    """Wait for block progress after sync.

    Returns (success, blocks_processed).
    """
    print(f"  Monitoring block progress for {fmt_time(duration)} (stall timeout: {fmt_time(stall_timeout)})...")
    start = time.time()

    # Retry to get a valid initial block number
    initial_block = rpc_block_number(rpc_url)
    retry_start = time.time()
    while initial_block is None and time.time() - retry_start < stall_timeout:
        time.sleep(CHECK_INTERVAL)
        initial_block = rpc_block_number(rpc_url)
    if initial_block is None:
        print(f"  Failed to fetch initial block number; node not responding for {fmt_time(stall_timeout)}")
        return False, 0

    last_block = initial_block
    last_block_time = time.time()
    last_status_print = 0

    while time.time() - start < duration:
        block = rpc_block_number(rpc_url)

        if block is None:
            if time.time() - last_block_time > stall_timeout:
                print(f"  Node stopped responding for {fmt_time(stall_timeout)}")
                return False, last_block - initial_block
        elif block > last_block:
            last_block = block
            last_block_time = time.time()
        elif time.time() - last_block_time > stall_timeout:
            print(f"  Block stalled at {last_block} for {fmt_time(stall_timeout)}")
            return False, last_block - initial_block

        if time.time() - last_status_print > 60:
            elapsed = time.time() - start
            blocks_done = last_block - initial_block
            print(f"  [{fmt_time(elapsed)}] Block {last_block} (+{blocks_done} since sync)")
            last_status_print = time.time()

        time.sleep(CHECK_INTERVAL)

    blocks_processed = last_block - initial_block
    if blocks_processed > 0:
        return True, blocks_processed
    return False, 0


def monitor_restart_for_stall(rpc_url: str, timeout: int, stall_timeout: int) -> tuple[str, str]:
    """Monitor a restarted node for header download stall.

    Args:
        rpc_url: RPC endpoint to poll.
        timeout: Overall time limit for monitoring.
        stall_timeout: Time without block progress to declare a stall.

    Returns (result, details) where result is one of:
      - "ok": Node synced/caught up within timeout
      - "stall": Node appears stalled (no block progress for stall_timeout)
      - "timeout": Overall timeout reached but node was still making progress
      - "unresponsive": Node never came back up
    """
    print(f"\n  Monitoring restart for stall (timeout: {fmt_time(timeout)}, stall: {fmt_time(stall_timeout)})...")
    start = time.time()

    # Wait for node to come back up
    if not wait_for_node(rpc_url, NODE_STARTUP_TIMEOUT):
        elapsed = time.time() - start
        return "unresponsive", f"Node never responded after {fmt_time(elapsed)}"

    # Monitor: is it syncing? Is it making progress?
    last_block = rpc_block_number(rpc_url) or 0
    last_progress_time = time.time()
    last_status_print = 0
    syncing_reported = False

    while time.time() - start < timeout:
        syncing = rpc_call(rpc_url, "eth_syncing")
        block = rpc_block_number(rpc_url)
        elapsed = time.time() - start

        if syncing is False:
            print(f"  Node caught up in {fmt_time(elapsed)} (block {block})")
            return "ok", f"Caught up in {fmt_time(elapsed)}, block {block}"

        if syncing is not None and not syncing_reported:
            print(f"  Node is syncing (expected after restart)")
            syncing_reported = True

        if block is not None:
            if block > last_block:
                last_block = block
                last_progress_time = time.time()
            elif time.time() - last_progress_time > stall_timeout:
                stall_duration = fmt_time(time.time() - last_progress_time)
                return "stall", f"Stalled at block {last_block} for {stall_duration}"
        elif block is None and syncing is None:
            if time.time() - last_progress_time > NODE_STARTUP_TIMEOUT:
                return "unresponsive", f"Node stopped responding after {fmt_time(elapsed)}"

        if time.time() - last_status_print > 60:
            stall_elapsed = fmt_time(time.time() - last_progress_time)
            print(f"  [{fmt_time(elapsed)}] Block {last_block}, last progress {stall_elapsed} ago, syncing={syncing is not False}")
            last_status_print = time.time()

        time.sleep(CHECK_INTERVAL)

    # Distinguish timeout-while-progressing from true stall
    stall_elapsed = time.time() - last_progress_time
    if stall_elapsed <= CHECK_INTERVAL * 2:
        return "timeout", f"Timed out after {fmt_time(timeout)} while still making progress (block {last_block})"
    return "stall", f"No progress for {fmt_time(stall_elapsed)} (stuck at block {last_block})"


def phase1_fresh_sync(eth_docker_dir: str, rpc_url: str) -> bool:
    """Phase 1: Clean start via eth-docker and wait for sync completion."""
    print(f"\n{'='*60}")
    print(f"PHASE 1: Fresh snap sync")
    print(f"{'='*60}\n")

    # Terminate (removes volumes) and start fresh
    # Use docker compose directly to avoid interactive prompts from ./ethd
    print("Stopping and removing containers + volumes...")
    docker_compose_in_ethd(eth_docker_dir, "down", "-v")
    time.sleep(5)

    print("Starting eth-docker...")
    docker_compose_in_ethd(eth_docker_dir, "up", "-d", check=True)
    time.sleep(30)

    # Wait for node to come up
    if not wait_for_node(rpc_url, NODE_STARTUP_TIMEOUT):
        print("FAILED: Node never came up")
        return False

    # Wait for sync
    synced, sync_time = wait_for_sync(rpc_url, SYNC_TIMEOUT)
    if not synced:
        print(f"FAILED: Sync timed out after {fmt_time(sync_time)}")
        return False

    # Verify block progress
    print(f"\n  Sync complete. Verifying block progress...")
    progress_ok, blocks = wait_for_block_progress(rpc_url, BLOCK_PROCESSING_DURATION, BLOCK_STALL_TIMEOUT)
    if not progress_ok:
        print(f"FAILED: No block progress after sync (processed {blocks} blocks)")
        return False

    print(f"\n  Phase 1 SUCCESS: synced in {fmt_time(sync_time)}, processed +{blocks} blocks")
    return True


def phase2_restart_test(eth_docker_dir: str, rpc_url: str, restart_num: int, wipe_data: bool = False) -> tuple[str, str]:
    """Phase 2: Stop execution client, restart it, monitor for stall.

    When wipe_data=True, removes containers and all data volumes (EL +
    consensus + validator) before restarting, forcing a full snap sync from
    scratch.
    """
    mode = "wipe + fresh sync" if wipe_data else "restart"
    print(f"\n{'='*60}")
    print(f"PHASE 2: Restart test #{restart_num} ({mode})")
    print(f"{'='*60}\n")

    if wipe_data:
        print("Wiping data volumes (EL + consensus + validator)...")
        wipe_data_volumes(eth_docker_dir)
        time.sleep(5)

        print("Starting containers with fresh volumes...")
        docker_compose_in_ethd(eth_docker_dir, "up", "-d", "execution", "consensus", check=True)
        time.sleep(30)

        # Wait for node to come up
        if not wait_for_node(rpc_url, NODE_STARTUP_TIMEOUT):
            print(f"\n  Restart #{restart_num} result: NODE UNRESPONSIVE")
            return "unresponsive", "Node never responded after wipe"

        # Wait for full snap sync
        synced, sync_time = wait_for_sync(rpc_url, SYNC_TIMEOUT)
        if not synced:
            details = f"Sync timed out after {fmt_time(sync_time)}"
            print(f"\n  Restart #{restart_num} result: TIMEOUT - {details}")
            return "timeout", details

        # Verify block progress
        print(f"\n  Sync complete. Verifying block progress...")
        progress_ok, blocks = wait_for_block_progress(rpc_url, BLOCK_PROCESSING_DURATION, BLOCK_STALL_TIMEOUT)
        if not progress_ok:
            details = f"No block progress after sync (processed {blocks} blocks, sync took {fmt_time(sync_time)})"
            print(f"\n  Restart #{restart_num} result: FAIL - {details}")
            return "stall", details

        details = f"Synced in {fmt_time(sync_time)}, processed +{blocks} blocks"
        print(f"\n  Restart #{restart_num} result: PASS - {details}")
        return "ok", details
    else:
        # Original behavior: stop/start, monitor for stall
        print("Stopping execution client (keeping consensus + volumes)...")
        docker_compose_in_ethd(eth_docker_dir, "stop", "execution", check=True)
        time.sleep(10)

        print("Restarting execution client...")
        docker_compose_in_ethd(eth_docker_dir, "start", "execution", check=True)
        time.sleep(5)

        result, details = monitor_restart_for_stall(
            rpc_url,
            timeout=RESTART_STALL_TIMEOUT * 2,
            stall_timeout=RESTART_STALL_TIMEOUT,
        )

        status_str = {
            "ok": "PASS",
            "stall": "STALL DETECTED",
            "timeout": "TIMEOUT (still progressing)",
            "unresponsive": "NODE UNRESPONSIVE",
        }.get(result, result.upper())

        print(f"\n  Restart #{restart_num} result: {status_str} - {details}")
        return result, details


def main():
    parser = argparse.ArgumentParser(description="Test snap sync restart stall bug (eth-docker)")
    parser.add_argument("--eth-docker-dir", default=os.path.expanduser("~/eth-docker"),
                        help="Path to eth-docker clone (default: ~/eth-docker)")
    parser.add_argument("--network", default="hoodi",
                        help="Ethereum network (default: hoodi)")
    parser.add_argument("--configure", action="store_true",
                        help="Write eth-docker .env for ethrex+Prysm before starting")
    parser.add_argument("--fee-recipient", default="",
                        help="Ethereum address for EL rewards (FEE_RECIPIENT in eth-docker)")
    parser.add_argument("--rpc-port", type=int, default=8545,
                        help="RPC port for ethrex (default: 8545)")
    parser.add_argument("--restart-count", type=int, default=3,
                        help="Number of restart cycles to test (default: 3)")
    parser.add_argument("--no-slack", action="store_true",
                        help="Disable Slack notifications")
    parser.add_argument("--skip-phase1", action="store_true",
                        help="Skip fresh sync (assume node is already synced)")
    parser.add_argument("--wipe-el-data", action="store_true",
                        help="Wipe all data volumes on each restart (forces fresh snap sync)")
    parser.add_argument("--ethrex-dir", default=None,
                        help="Path to ethrex repo (for git info in Slack). Auto-detected if not set.")
    args = parser.parse_args()

    eth_docker_dir = os.path.abspath(args.eth_docker_dir)
    rpc_url = f"http://localhost:{args.rpc_port}"
    run_id = datetime.now().strftime("%Y%m%d_%H%M%S")

    # Validate eth-docker directory
    if not os.path.isfile(os.path.join(eth_docker_dir, "ethd")):
        print(f"Error: eth-docker not found at {eth_docker_dir}")
        print("Clone it with: git clone https://github.com/ethstaker/eth-docker.git ~/eth-docker")
        sys.exit(1)

    # Load our local .env first (for Slack webhooks)
    load_env_file(str(SCRIPT_DIR / ".env"))

    # Configure eth-docker .env if requested
    if args.configure:
        print("Configuring eth-docker for ethrex + Prysm...")
        configure_eth_docker(
            eth_docker_dir,
            network=args.network,
            fee_recipient=args.fee_recipient,
            slack_success=os.environ.get("SLACK_WEBHOOK_URL_SUCCESS", ""),
            slack_failed=os.environ.get("SLACK_WEBHOOK_URL_FAILED", ""),
        )

    env_file = os.path.join(eth_docker_dir, ".env")
    if not os.path.isfile(env_file):
        print(f"Error: .env not found at {env_file}")
        print("Run with --configure, or configure manually: cd ~/eth-docker && ./ethd config")
        sys.exit(1)

    # Load eth-docker .env for network info and any extra vars
    load_env_file(env_file)

    network = os.environ.get("NETWORK", args.network)
    ethrex_dir = args.ethrex_dir or os.environ.get("ETHREX_DIR")
    branch, commit = git_info(cwd=ethrex_dir)

    # Create logs directory
    run_dir = LOGS_DIR / f"run_{run_id}"
    run_dir.mkdir(parents=True, exist_ok=True)

    print(f"Restart Stall Test (eth-docker)")
    print(f"  eth-docker: {eth_docker_dir}")
    print(f"  Network:    {network}")
    print(f"  RPC:        {rpc_url}")
    print(f"  Branch:     {branch}")
    print(f"  Commit:     {commit}")
    print(f"  Restarts:   {args.restart_count}")
    print(f"  Wipe data:  {args.wipe_el_data}")
    print(f"  Logs:       {run_dir}")
    print()

    # Phase 1: Fresh sync
    if not args.skip_phase1:
        sync_ok = phase1_fresh_sync(eth_docker_dir, rpc_url)
        save_ethd_logs(eth_docker_dir, run_dir, suffix="_phase1")

        if not sync_ok:
            if not args.no_slack:
                slack_notify(
                    "Restart Stall Test - Phase 1 FAILED",
                    success=False,
                    details=f"*Network:* `{network}`\nFresh sync failed. Cannot proceed to restart test.",
                    ethrex_dir=ethrex_dir,
                )
            sys.exit(1)

        if not args.no_slack:
            slack_notify(
                "Restart Stall Test - Phase 1 Complete",
                success=True,
                details=f"*Network:* `{network}`\nFresh sync completed. Starting restart tests...",
                ethrex_dir=ethrex_dir,
            )

    # Phase 2: Restart cycles
    results = []
    for i in range(1, args.restart_count + 1):
        result, details = phase2_restart_test(eth_docker_dir, rpc_url, i, wipe_data=args.wipe_el_data)
        results.append((i, result, details))

        save_ethd_logs(eth_docker_dir, run_dir, suffix=f"_restart{i}")

        if result not in ("ok", "timeout") and not args.no_slack:
            slack_notify(
                f"Restart Stall Test - STALL on restart #{i}",
                success=False,
                details=(
                    f"*Network:* `{network}`\n"
                    f"*Restart:* #{i} of {args.restart_count}\n"
                    f"*Result:* {details}\n"
                    f"*Logs:* `{run_dir}`\n\n"
                    "Containers are still running for inspection."
                ),
                ethrex_dir=ethrex_dir,
            )

    # Final summary
    stalls = [(i, r, d) for i, r, d in results if r not in ("ok", "timeout")]
    all_ok = len(stalls) == 0

    print(f"\n{'='*60}")
    print(f"FINAL RESULTS")
    print(f"{'='*60}")
    for i, result, details in results:
        status = "PASS" if result == "ok" else ("TIMEOUT" if result == "timeout" else "FAIL")
        print(f"  Restart #{i}: {status} - {details}")
    print(f"\n  Overall: {'ALL PASSED' if all_ok else f'{len(stalls)}/{len(results)} STALLED'}")

    # Save summary
    summary_lines = [
        f"Restart Stall Test - {run_id}",
        f"Network: {network}",
        f"Branch: {branch}",
        f"Commit: {commit}",
        f"Host: {socket.gethostname()}",
        f"eth-docker: {eth_docker_dir}",
        "",
    ]
    for i, result, details in results:
        summary_lines.append(f"Restart #{i}: {result} - {details}")
    summary_lines.append(f"\nOverall: {'ALL PASSED' if all_ok else f'{len(stalls)}/{len(results)} STALLED'}")
    (run_dir / "summary.txt").write_text("\n".join(summary_lines))

    # Final Slack notification
    if not args.no_slack:
        result_lines = "\n".join(
            f"{'PASS' if r == 'ok' else ('TIMEOUT' if r == 'timeout' else 'FAIL')} Restart #{i}: {d}" for i, r, d in results
        )
        slack_notify(
            f"Restart Stall Test - {'ALL PASSED' if all_ok else 'STALL DETECTED'}",
            success=all_ok,
            details=(
                f"*Network:* `{network}`\n"
                f"*Restarts:* {args.restart_count}\n"
                f"*Stalls:* {len(stalls)}/{len(results)}\n\n"
                f"```\n{result_lines}\n```\n"
                f"*Logs:* `{run_dir}`"
            ),
            ethrex_dir=ethrex_dir,
        )

    sys.exit(0 if all_ok else 1)


if __name__ == "__main__":
    main()
