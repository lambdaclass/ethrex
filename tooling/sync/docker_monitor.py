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
from typing import Optional

import requests

CHECK_INTERVAL = 10
SYNC_TIMEOUT = 4 * 60  # 4 hours default sync timeout (in minutes)
BLOCK_PROCESSING_DURATION = 22 * 60 # Monitor block processing for 20 minutes
BLOCK_STALL_TIMEOUT = 10 * 60  # Fail if no new block for 10 minutes
STATUS_PRINT_INTERVAL = 30

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


def container_start_time(name: str) -> Optional[float]:
    try:
        out = subprocess.check_output(["docker", "inspect", "-f", "{{.State.StartedAt}}", name], stderr=subprocess.DEVNULL).decode().strip()
        if '.' in out:
            base, frac = out.rsplit('.', 1)
            out = f"{base}.{frac.rstrip('Z')[:6]}"
        return datetime.fromisoformat(out.replace('Z', '+00:00')).timestamp()
    except Exception:
        return None


def rpc_call(url: str, method: str) -> Optional[any]:
    try:
        return requests.post(url, json={"jsonrpc": "2.0", "method": method, "params": [], "id": 1}, timeout=5).json().get("result")
    except Exception:
        return None


def slack_notify(header: str, msg: str, success: bool = True):
    url = os.environ.get("SLACK_WEBHOOK_URL_SUCCESS" if success else "SLACK_WEBHOOK_URL_FAILED")
    if url:
        try:
            requests.post(url, json={"blocks": [
                {"type": "header", "text": {"type": "plain_text", "text": header}},
                {"type": "section", "text": {"type": "mrkdwn", "text": msg}}
            ]}, timeout=10)
        except Exception:
            pass


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
            inst.status, inst.error = "failed", f"Sync timeout ({timeout_min}m)"
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
    p.add_argument("--ports", default="8545,8546,8547")
    p.add_argument("--names", default="hoodi,sepolia,mainnet")
    p.add_argument("--containers", default="")
    p.add_argument("--timeout", type=int, default=SYNC_TIMEOUT)
    p.add_argument("--no-slack", action="store_true")
    p.add_argument("--exit-on-success", action="store_true")
    p.add_argument("--compose-file", default="docker-compose.snapsync.yaml", help="Docker compose file name")
    p.add_argument("--compose-dir", default=".", help="Directory containing docker compose file")
    args = p.parse_args()
    
    ports = [int(x) for x in args.ports.split(",")]
    names = args.names.split(",")
    containers = args.containers.split(",") if args.containers else [f"ethrex-{n}" for n in names]
    
    if len(ports) != len(names):
        sys.exit("Error: ports and names must match")
    
    instances = [Instance(n.strip(), p, c.strip()) for n, p, c in zip(names, ports, containers)]
    
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
    
    hostname, commit = socket.gethostname(), git_commit()
    run_count = 1
    
    try:
        while True:
            print(f"🔍 Run #{run_count}: Monitoring {len(instances)} instances (timeout: {args.timeout}m)", flush=True)
            last_print = 0
            
            while True:
                changed = any(update_instance(i, args.timeout) for i in instances)
                
                if not args.no_slack:
                    for i in instances:
                        if i.status == "success":
                            slack_notify(f"✅ {i.name} snapsync complete (run #{run_count})", f"*Server:* `{hostname}`\n*Synced in:* {fmt_time(i.sync_time)}\n*Commit:* `{commit}`")
                        elif i.status == "failed":
                            slack_notify(f"❌ {i.name} snapsync failed (run #{run_count})", f"*Server:* `{hostname}`\n*Error:* {i.error}\n*Commit:* `{commit}`", False)
                
                if changed or (time.time() - last_print) > STATUS_PRINT_INTERVAL:
                    print_status(instances)
                    last_print = time.time()
                
                if all(i.status in ("success", "failed") for i in instances):
                    print_status(instances)
                    break
                
                time.sleep(CHECK_INTERVAL)
            
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
                    time.sleep(30)  # Wait for containers to start
                else:
                    sys.exit("❌ Failed to restart containers")
            else:
                sys.exit("⚠️ Some instances failed")
    except KeyboardInterrupt:
        print("\n⚠️ Interrupted")
        print_status(instances)
        sys.exit(130)


if __name__ == "__main__":
    main()
