#!/usr/bin/env python3
"""Monitor Docker Compose snapsync instances for sync completion."""

import argparse
import os
import socket
import subprocess
import sys
import time
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from typing import Any, Optional

import requests

# Load .env file if it exists
if os.path.exists('.env'):
    with open('.env') as f:
        for line in f:
            line = line.strip()
            if line and not line.startswith('#'):
                key, _, value = line.partition('=')
                os.environ[key.strip()] = value.strip()

CHECK_INTERVAL = 10
SYNC_TIMEOUT = 4 * 60  # 4 hours default sync timeout (in minutes)
BLOCK_PROCESSING_DURATION = 22 * 60 # Monitor block processing for 22 minutes
BLOCK_STALL_TIMEOUT = 10 * 60  # Fail if no new block for 10 minutes
STATUS_PRINT_INTERVAL = 30

# Network to port mapping (fixed in docker-compose.multisync.yaml)
NETWORK_PORTS = {
    "hoodi": 8545,
    "sepolia": 8546,
    "mainnet": 8547,
    "hoodi-2": 8548,
}

# Logging configuration
LOGS_DIR = Path("./multisync_logs")
RUN_LOG_FILE = LOGS_DIR / "run_history.log"  # Append-only text log

STATUS_EMOJI = {
    "waiting": "⏳", "syncing": "🔄", "synced": "✅",
    "block_processing": "📦", "success": "🎉", "failed": "❌"
}


@dataclass
class Instance:
    name: str
    port: int
    container: str = ""
    status: str = "waiting"
    start_time: float = 0
    sync_time: float = 0
    last_block: int = 0
    last_block_time: float = 0  # When we last saw a new block
    block_check_start: float = 0
    initial_block: int = 0  # Block when entering block_processing
    error: str = ""

    @property
    def rpc_url(self) -> str:
        return f"http://localhost:{self.port}"


def fmt_time(secs: float) -> str:
    secs = int(abs(secs))
    h, m, s = secs // 3600, (secs % 3600) // 60, secs % 60
    return " ".join(f"{v}{u}" for v, u in [(h, "h"), (m, "m"), (s, "s")] if v or (not h and not m))


def git_commit() -> str:
    try:
        return subprocess.check_output(["git", "rev-parse", "--short", "HEAD"], stderr=subprocess.DEVNULL).decode().strip()
    except Exception:
        return "unknown"


def git_branch() -> str:
    try:
        return subprocess.check_output(["git", "rev-parse", "--abbrev-ref", "HEAD"], stderr=subprocess.DEVNULL).decode().strip()
    except Exception:
        return "unknown"


def container_start_time(name: str) -> Optional[float]:
    try:
        out = subprocess.check_output(["docker", "inspect", "-f", "{{.State.StartedAt}}", name], stderr=subprocess.DEVNULL).decode().strip()
        if '.' in out:
            base, frac = out.rsplit('.', 1)
            out = f"{base}.{frac.rstrip('Z')[:6]}"
        return datetime.fromisoformat(out.replace('Z', '+00:00')).timestamp()
    except Exception:
        return None


def rpc_call(url: str, method: str) -> Optional[Any]:
    try:
        return requests.post(url, json={"jsonrpc": "2.0", "method": method, "params": [], "id": 1}, timeout=5).json().get("result")
    except Exception:
        return None


def slack_notify(run_id: str, run_count: int, instances: list, hostname: str, branch: str, commit: str):
    """Send a single summary Slack message for the run."""
    all_success = all(i.status == "success" for i in instances)
    url = os.environ.get("SLACK_WEBHOOK_URL_SUCCESS" if all_success else "SLACK_WEBHOOK_URL_FAILED")
    if not url:
        return
    status_icon = "✅" if all_success else "❌"
    header = f"{status_icon} Run #{run_count} (ID: {run_id})"
    run_start = datetime.strptime(run_id, "%Y%m%d_%H%M%S")
    elapsed_secs = (datetime.now() - run_start).total_seconds()
    elapsed_str = fmt_time(elapsed_secs)
    summary = f"*Host:* `{hostname}`\n*Branch:* `{branch}`\n*Commit:* <https://github.com/lambdaclass/ethrex/commit/{commit}|{commit}>\n*Elapsed:* `{elapsed_str}`\n*Logs:* `tooling/sync/multisync_logs/run_{run_id}`\n*Result:* {'SUCCESS' if all_success else 'FAILED'}"
    blocks = [
        {"type": "header", "text": {"type": "plain_text", "text": header}},
        {"type": "section", "text": {"type": "mrkdwn", "text": summary}},
        {"type": "divider"}
    ]
    for i in instances:
        icon = "✅" if i.status == "success" else "❌"
        line = f"{icon} *{i.name}*: `{i.status}`"
        if i.sync_time:
            line += f" (sync: {fmt_time(i.sync_time)})"
        if i.initial_block:
            line += f" post-sync block: {i.initial_block}"
        if i.initial_block and i.last_block > i.initial_block:
            blocks_processed = i.last_block - i.initial_block
            line += f" (processed +{blocks_processed} blocks in {BLOCK_PROCESSING_DURATION//60}m)"
        if i.error:
            line += f"\n       Error: {i.error}"
        blocks.append({"type": "section", "text": {"type": "mrkdwn", "text": line}})
    try:
        requests.post(url, json={"blocks": blocks}, timeout=10)
    except Exception:
        pass


def ensure_logs_dir():
    """Ensure the logs directory exists."""
    LOGS_DIR.mkdir(parents=True, exist_ok=True)


def save_container_logs(container: str, run_id: str, suffix: str = ""):
    """Save container logs to a file."""
    log_file = LOGS_DIR / f"run_{run_id}" / f"{container}{suffix}.log"
    log_file.parent.mkdir(parents=True, exist_ok=True)
    try:
        logs = subprocess.check_output(
            ["docker", "logs", container], 
            stderr=subprocess.STDOUT,
            timeout=60
        ).decode(errors='replace')
        log_file.write_text(logs)
        print(f"  📄 Saved logs: {log_file}")
        return True
    except subprocess.CalledProcessError as e:
        print(f"  ⚠️ Failed to get logs for {container}: {e}")
        return False
    except subprocess.TimeoutExpired:
        print(f"  ⚠️ Timeout getting logs for {container}")
        return False
    except Exception as e:
        print(f"  ⚠️ Error saving logs for {container}: {e}")
        return False


def save_all_logs(instances: list[Instance], run_id: str, compose_file: str):
    """Save logs for all containers (ethrex + consensus)."""
    print(f"\n📁 Saving logs for run {run_id}...")
    
    for inst in instances:
        # Save ethrex logs
        save_container_logs(inst.container, run_id)
        # Save consensus logs (convention: consensus-{network})
        consensus_container = inst.container.replace("ethrex-", "consensus-")
        save_container_logs(consensus_container, run_id)
    
    print(f"📁 Logs saved to {LOGS_DIR}/run_{run_id}/\n")


def log_run_result(run_id: str, run_count: int, instances: list[Instance], hostname: str, branch: str, commit: str):
    """Append run result to the persistent log file."""
    ensure_logs_dir()
    all_success = all(i.status == "success" for i in instances)
    status_icon = "✅" if all_success else "❌"
    run_start = datetime.strptime(run_id, "%Y%m%d_%H%M%S")
    elapsed_secs = (datetime.now() - run_start).total_seconds()
    elapsed_str = fmt_time(elapsed_secs)
    # Build log entry as plain text
    lines = [
        f"\n{'='*60}",
        f"{status_icon} Run #{run_count} (ID: {run_id})",
        f"{'='*60}",
        f"Host:   {hostname}",
        f"Branch: {branch}",
        f"Commit: {commit}",
        f"Elapsed: {elapsed_str}",
        f"Result: {'SUCCESS' if all_success else 'FAILED'}",
        "",
    ]
    for inst in instances:
        icon = "✅" if inst.status == "success" else "❌"
        line = f"  {icon} {inst.name}: {inst.status}"
        if inst.sync_time:
            line += f" (sync: {fmt_time(inst.sync_time)})"
        if inst.initial_block:
            line += f" post-sync block: {inst.initial_block}"
        if inst.initial_block and inst.last_block > inst.initial_block:
            blocks_processed = inst.last_block - inst.initial_block
            line += f" (processed +{blocks_processed} blocks in {BLOCK_PROCESSING_DURATION//60}m)"
        if inst.error:
            line += f"\n       Error: {inst.error}"
        lines.append(line)
    lines.append("")
    # Append to log file
    with open(RUN_LOG_FILE, "a") as f:
        f.write("\n".join(lines) + "\n")
    print(f"📝 Run logged to {RUN_LOG_FILE}")
    # Also write summary to the run folder
    summary_file = LOGS_DIR / f"run_{run_id}" / "summary.txt"
    summary_file.parent.mkdir(parents=True, exist_ok=True)
    summary_file.write_text("\n".join(lines))


def generate_run_id() -> str:
    """Generate a unique run ID based on timestamp."""
    return datetime.now().strftime("%Y%m%d_%H%M%S")


def restart_containers(compose_file: str, compose_dir: str):
    """Stop and restart docker compose containers, clearing volumes."""
    print("\n🔄 Restarting containers...\n", flush=True)
    try:
        subprocess.run(["docker", "compose", "-f", compose_file, "down", "-v"], cwd=compose_dir, check=True)
        time.sleep(5)
        subprocess.run(["docker", "compose", "-f", compose_file, "up", "-d"], cwd=compose_dir, check=True)
        print("✅ Containers restarted successfully\n", flush=True)
        return True
    except subprocess.CalledProcessError as e:
        print(f"❌ Failed to restart containers: {e}\n", flush=True)
        return False


def reset_instance(inst: Instance):
    """Reset instance state for a new sync cycle."""
    inst.status = "waiting"
    inst.start_time = 0
    inst.sync_time = 0
    inst.last_block = 0
    inst.last_block_time = 0
    inst.block_check_start = 0
    inst.initial_block = 0
    inst.error = ""


def print_status(instances: list[Instance]):
    print("\033[2J\033[H", end="")
    print(f"{'='*60}\nStatus at {time.strftime('%H:%M:%S')}\n{'='*60}")
    
    for i in instances:
        elapsed = time.time() - i.start_time if i.start_time else 0
        extra = {
            "waiting": " (waiting for node...)",
            "syncing": f" ({fmt_time(elapsed)} elapsed)",
            "synced": f" (synced in {fmt_time(i.sync_time)})",
            "block_processing": f" (block {i.last_block}, +{i.last_block - i.initial_block} blocks, {fmt_time(BLOCK_PROCESSING_DURATION - (time.time() - i.block_check_start))} left)",
            "success": f" ✓ synced in {fmt_time(i.sync_time)}, processed +{i.last_block - i.initial_block} blocks",
            "failed": f" - {i.error}"
        }.get(i.status, "")
        print(f"  {STATUS_EMOJI.get(i.status, '?')} {i.name} (:{i.port}): {i.status}{extra}")
    
    print(flush=True)


def update_instance(inst: Instance, timeout_min: int) -> bool:
    if inst.status in ("success", "failed"):
        return False
    
    now = time.time()
    block = rpc_call(inst.rpc_url, "eth_blockNumber")
    block = int(block, 16) if block else None
    
    if block is None:
        if inst.status != "waiting":
            inst.status, inst.error = "failed", "Node stopped responding"
            return True
        return False
    
    if inst.status == "waiting":
        inst.status, inst.start_time = "syncing", inst.start_time or now
        return True
    
    if inst.status == "syncing":
        if (now - inst.start_time) > timeout_min * 60:
            inst.status, inst.error = "failed", f"Sync timeout after {fmt_time(timeout_min * 60)}"
            return True
        if rpc_call(inst.rpc_url, "eth_syncing") is False:
            inst.status, inst.sync_time = "synced", now - inst.start_time
            inst.block_check_start, inst.last_block = now, block
            inst.initial_block, inst.last_block_time = block, now
            return True
    
    if inst.status == "synced":
        inst.status = "block_processing"
        inst.block_check_start, inst.last_block, inst.initial_block, inst.last_block_time = now, block, block, now
        return True
    
    if inst.status == "block_processing":
        # Check for stalled node (no new blocks for too long)
        if (now - inst.last_block_time) > BLOCK_STALL_TIMEOUT:
            inst.status, inst.error = "failed", f"Block processing stalled at {inst.last_block} for {fmt_time(BLOCK_STALL_TIMEOUT)}"
            return True
        # Update last block time if we see progress
        if block and block > inst.last_block:
            inst.last_block, inst.last_block_time = block, now
        # Success after duration, but only if we made progress
        if (now - inst.block_check_start) > BLOCK_PROCESSING_DURATION:
            if inst.last_block > inst.initial_block:
                inst.status = "success"
                return True
            else:
                inst.status, inst.error = "failed", "No block progress during monitoring"
                return True
    
    return False


def main():
    p = argparse.ArgumentParser(description="Monitor Docker snapsync instances")
    p.add_argument("--networks", default="hoodi,sepolia,mainnet")
    p.add_argument("--timeout", type=int, default=SYNC_TIMEOUT)
    p.add_argument("--no-slack", action="store_true")
    p.add_argument("--exit-on-success", action="store_true")
    p.add_argument("--compose-file", default="docker-compose.multisync.yaml", help="Docker compose file name")
    p.add_argument("--compose-dir", default=".", help="Directory containing docker compose file")
    args = p.parse_args()
    
    names = [n.strip() for n in args.networks.split(",")]
    ports = []
    for n in names:
        if n not in NETWORK_PORTS:
            sys.exit(f"Error: unknown network '{n}', known networks: {list(NETWORK_PORTS.keys())}")
        ports.append(NETWORK_PORTS[n])
    containers = [f"ethrex-{n}" for n in names]
    
    instances = [Instance(n, p, c) for n, p, c in zip(names, ports, containers)]
    
    # Detect state of already-running containers
    for inst in instances:
        if t := container_start_time(inst.container):
            inst.start_time = t
            # Check if already synced
            syncing = rpc_call(inst.rpc_url, "eth_syncing")
            if syncing is False:
                # Already synced - go straight to block_processing
                block = rpc_call(inst.rpc_url, "eth_blockNumber")
                block = int(block, 16) if block else 0
                inst.status = "block_processing"
                inst.sync_time = time.time() - t
                inst.block_check_start = time.time()
                inst.initial_block = block
                inst.last_block = block
                inst.last_block_time = time.time()
            elif syncing is not None:
                # Still syncing
                inst.status = "syncing"
            # else: node not responding yet, stay in "waiting"
    
    hostname = socket.gethostname()
    branch = git_branch()
    commit = git_commit()
    run_count = 1
    run_id = generate_run_id()
    
    # Ensure logs directory exists
    ensure_logs_dir()
    print(f"📁 Logs will be saved to {LOGS_DIR.absolute()}")
    print(f"📝 Run history: {RUN_LOG_FILE.absolute()}\n")
    
    try:
        while True:
            print(f"🔍 Run #{run_count} (ID: {run_id}): Monitoring {len(instances)} instances (timeout: {args.timeout}m)", flush=True)
            last_print = 0
            while True:
                changed = any(update_instance(i, args.timeout) for i in instances)
                if changed or (time.time() - last_print) > STATUS_PRINT_INTERVAL:
                    print_status(instances)
                    last_print = time.time()
                if all(i.status in ("success", "failed") for i in instances):
                    print_status(instances)
                    break
                time.sleep(CHECK_INTERVAL)
            # Log the run result and save container logs BEFORE any restart
            save_all_logs(instances, run_id, args.compose_file)
            log_run_result(run_id, run_count, instances, hostname, branch, commit)
            # Send a single Slack summary notification for the run
            if not args.no_slack:
                slack_notify(run_id, run_count, instances, hostname, branch, commit)
            # Check results
            if all(i.status == "success" for i in instances):
                print(f"🎉 Run #{run_count}: All instances synced successfully!")
                if args.exit_on_success:
                    sys.exit(0)
                # Restart for another run
                if restart_containers(args.compose_file, args.compose_dir):
                    for inst in instances:
                        reset_instance(inst)
                    run_count += 1
                    run_id = generate_run_id()  # New run ID for the new cycle
                    time.sleep(30)  # Wait for containers to start
                else:
                    sys.exit("❌ Failed to restart containers")
            else:
                # On failure: containers are NOT stopped, you can inspect the DB
                print("\n" + "="*60)
                print("⚠️  FAILURE - Containers are still running for inspection")
                print("="*60)
                print("\nYou can:")
                print("  - Inspect the database in the running containers")
                print("  - Check logs: docker logs <container-name>")
                print(f"  - View saved logs: {LOGS_DIR}/run_{run_id}/")
                print(f"  - View run history: {RUN_LOG_FILE}")
                print("\nTo restart manually: make multisync-restart")
                sys.exit(1)
    except KeyboardInterrupt:
        print("\n⚠️ Interrupted")
        print_status(instances)
        sys.exit(130)


if __name__ == "__main__":
    main()
