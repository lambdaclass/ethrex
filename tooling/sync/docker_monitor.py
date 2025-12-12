#!/usr/bin/env python3
"""
Monitor Docker Compose snapsync instances.

Reuses logic from server_runner.py to check sync status and block production.

Usage:
    python docker_monitor.py --compose-file docker-compose.snapsync-test.yaml
    python docker_monitor.py --ports 8545,8546 --names hoodi-1,hoodi-2
"""

import argparse
import requests
import time
import os
import json
import socket
import sys
import subprocess
from dataclasses import dataclass
from typing import Optional


CHECK_INTERVAL = 10  # seconds
BLOCK_CHECK_INTERVAL = 120  # seconds  
BLOCK_PRODUCTION_DURATION = 30 * 60  # 30 minutes
STATUS_PRINT_INTERVAL = 30  # seconds - print status every 30s


@dataclass
class Instance:
    name: str
    port: int
    container_name: str = ""  # Docker container name
    status: str = "waiting"  # waiting, syncing, synced, block_production, success, failed
    start_time: float = 0
    sync_time: float = 0
    last_block: int = 0
    block_check_start: float = 0
    error: str = ""

    @property
    def rpc_url(self) -> str:
        return f"http://localhost:{self.port}"


def format_elapsed_time(seconds: float) -> str:
    seconds = abs(seconds)
    hours = int(seconds // 3600)
    remaining = seconds % 3600
    minutes = int(remaining // 60)
    secs = int(remaining % 60)
    parts = []
    if hours > 0:
        parts.append(f"{hours}h")
    if minutes > 0:
        parts.append(f"{minutes}m")
    if secs > 0 or (hours == 0 and minutes == 0):
        parts.append(f"{secs}s")
    return " ".join(parts)


def get_git_commit() -> Optional[str]:
    try:
        return subprocess.check_output(["git", "rev-parse", "--short", "HEAD"]).decode().strip()
    except:
        return None


def get_container_start_time(container_name: str) -> Optional[float]:
    """Get the start time of a Docker container as Unix timestamp."""
    try:
        result = subprocess.check_output([
            "docker", "inspect", "-f", "{{.State.StartedAt}}", container_name
        ], stderr=subprocess.DEVNULL).decode().strip()
        # Parse ISO format: 2024-12-11T10:30:45.123456789Z
        from datetime import datetime
        # Handle nanoseconds by truncating to microseconds
        if '.' in result:
            base, frac = result.rsplit('.', 1)
            # Remove 'Z' and truncate to 6 digits for microseconds
            frac = frac.rstrip('Z')[:6]
            result = f"{base}.{frac}"
        else:
            result = result.rstrip('Z')
        dt = datetime.fromisoformat(result.replace('Z', '+00:00'))
        return dt.timestamp()
    except Exception:
        return None


def send_slack_message(header: str, message: str, success: bool = True):
    """Send Slack notification."""
    try:
        webhook_key = "SLACK_WEBHOOK_URL_SUCCESS" if success else "SLACK_WEBHOOK_URL_FAILED"
        webhook_url = os.environ.get(webhook_key)
        if not webhook_url:
            return

        payload = {
            "blocks": [
                {"type": "header", "text": {"type": "plain_text", "text": header}},
                {"type": "section", "text": {"type": "mrkdwn", "text": message}},
            ]
        }
        requests.post(webhook_url, json=payload, timeout=10)
    except Exception as e:
        print(f"Slack error: {e}", file=sys.stderr)


def check_syncing(rpc_url: str) -> tuple[bool, Optional[dict]]:
    """Check eth_syncing. Returns (is_synced, sync_info)."""
    try:
        resp = requests.post(rpc_url, json={
            "jsonrpc": "2.0", "method": "eth_syncing", "params": [], "id": 1
        }, timeout=5).json()
        result = resp.get("result")
        if result is False:
            return True, None
        return False, result
    except:
        return False, None


def get_block_number(rpc_url: str) -> Optional[int]:
    """Get current block number."""
    try:
        resp = requests.post(rpc_url, json={
            "jsonrpc": "2.0", "method": "eth_blockNumber", "params": [], "id": 1
        }, timeout=5).json()
        return int(resp.get("result", "0x0"), 16)
    except:
        return None


def parse_args():
    parser = argparse.ArgumentParser(description="Monitor Docker Compose snapsync instances")
    parser.add_argument("--ports", type=str, default="8545,8546",
                        help="Comma-separated RPC ports (default: 8545,8546)")
    parser.add_argument("--names", type=str, default="hoodi-1,hoodi-2",
                        help="Comma-separated instance names (default: hoodi-1,hoodi-2)")
    parser.add_argument("--containers", type=str, default="",
                        help="Comma-separated Docker container names (default: ethrex-<name>)")
    parser.add_argument("--timeout", type=int, default=180,
                        help="Sync timeout in minutes (default: 180)")
    parser.add_argument("--no-slack", action="store_true",
                        help="Disable Slack notifications")
    parser.add_argument("--exit-on-success", action="store_true",
                        help="Exit when all instances succeed")
    return parser.parse_args()


def print_status(instances: list[Instance]):
    """Print current status of all instances."""
    # Clear previous output for cleaner display
    print(f"\033[2J\033[H", end="")  # Clear screen and move cursor to top
    print(f"{'='*60}")
    print(f"Status at {time.strftime('%H:%M:%S')}")
    print(f"{'='*60}")
    for inst in instances:
        elapsed = time.time() - inst.start_time if inst.start_time else 0
        emoji = {
            "waiting": "⏳",
            "syncing": "🔄",
            "synced": "✅",
            "block_production": "📦",
            "success": "🎉",
            "failed": "❌"
        }.get(inst.status, "❓")
        
        extra = ""
        if inst.status == "waiting":
            extra = " (waiting for node...)"
        elif inst.status == "syncing":
            extra = f" ({format_elapsed_time(elapsed)} elapsed)"
        elif inst.status == "synced":
            extra = f" (synced in {format_elapsed_time(inst.sync_time)})"
        elif inst.status == "block_production":
            remaining = BLOCK_PRODUCTION_DURATION - (time.time() - inst.block_check_start)
            extra = f" (block {inst.last_block}, {format_elapsed_time(remaining)} remaining)"
        elif inst.status == "success":
            extra = f" ✓ synced in {format_elapsed_time(inst.sync_time)}"
        elif inst.status == "failed":
            extra = f" - {inst.error}"
            
        print(f"  {emoji} {inst.name} (:{inst.port}): {inst.status}{extra}")
    print()
    sys.stdout.flush()  # Force output


def monitor_instance(inst: Instance, timeout_minutes: int) -> bool:
    """Update instance status. Returns True if state changed."""
    now = time.time()
    
    if inst.status in ["success", "failed"]:
        return False
    
    # Check if reachable
    block = get_block_number(inst.rpc_url)
    if block is None:
        if inst.status == "waiting":
            return False  # Still waiting for container
        # Was running but stopped
        inst.status = "failed"
        inst.error = "Node stopped responding"
        return True
    
    # Node is reachable
    if inst.status == "waiting":
        inst.status = "syncing"
        inst.start_time = now
        return True
    
    # Check timeout
    if inst.status == "syncing" and (now - inst.start_time) > timeout_minutes * 60:
        inst.status = "failed"
        inst.error = f"Sync timeout ({timeout_minutes}m)"
        return True
    
    # Check sync status
    if inst.status == "syncing":
        is_synced, _ = check_syncing(inst.rpc_url)
        if is_synced:
            inst.status = "synced"
            inst.sync_time = now - inst.start_time
            inst.block_check_start = now
            inst.last_block = block or 0
            return True
        return False
    
    # Synced - start block production monitoring
    if inst.status == "synced":
        inst.status = "block_production"
        inst.block_check_start = now
        inst.last_block = block or 0
        return True
    
    # Block production phase
    if inst.status == "block_production":
        if (now - inst.block_check_start) > BLOCK_PRODUCTION_DURATION:
            inst.status = "success"
            return True
        
        if block is not None and block > inst.last_block:
            inst.last_block = block
        elif block is not None and block <= inst.last_block:
            # No new blocks - but only fail after several checks
            pass  # For now, just continue
        return False
    
    return False


def main():
    args = parse_args()
    hostname = socket.gethostname()
    commit = get_git_commit()
    
    ports = [int(p.strip()) for p in args.ports.split(",")]
    names = [n.strip() for n in args.names.split(",")]
    
    if len(ports) != len(names):
        print("Error: ports and names must have same length", file=sys.stderr)
        sys.exit(1)
    
    # Get container names (default: ethrex-<name>)
    if args.containers:
        containers = [c.strip() for c in args.containers.split(",")]
    else:
        containers = [f"ethrex-{n}" for n in names]
    
    instances = [Instance(name=n, port=p, container_name=c) for n, p, c in zip(names, ports, containers)]
    
    # Try to get container start times
    for inst in instances:
        start_time = get_container_start_time(inst.container_name)
        if start_time:
            inst.start_time = start_time
            inst.status = "syncing"  # Container is running, assume syncing
    
    print(f"🔍 Monitoring {len(instances)} instances...")
    print(f"   Timeout: {args.timeout} minutes")
    print(f"   Block production check: {BLOCK_PRODUCTION_DURATION // 60} minutes")
    sys.stdout.flush()
    
    last_print = 0
    
    try:
        while True:
            any_changed = False
            for inst in instances:
                if monitor_instance(inst, args.timeout):
                    any_changed = True
                    
                    # Send notifications on state changes
                    if not args.no_slack:
                        if inst.status == "success":
                            send_slack_message(
                                f"✅ {inst.name} snapsync complete",
                                f"*Server:* `{hostname}`\n*Synced in:* {format_elapsed_time(inst.sync_time)}\n*Commit:* `{commit}`",
                                success=True
                            )
                        elif inst.status == "failed":
                            send_slack_message(
                                f"❌ {inst.name} snapsync failed",
                                f"*Server:* `{hostname}`\n*Error:* {inst.error}\n*Commit:* `{commit}`",
                                success=False
                            )
            
            # Print status every STATUS_PRINT_INTERVAL seconds
            if any_changed or (time.time() - last_print) > STATUS_PRINT_INTERVAL:
                print_status(instances)
                last_print = time.time()
            
            # Check completion
            all_done = all(i.status in ["success", "failed"] for i in instances)
            all_success = all(i.status == "success" for i in instances)
            
            if all_done:
                print_status(instances)
                if all_success:
                    print("🎉 All instances synced successfully!")
                    if args.exit_on_success:
                        sys.exit(0)
                else:
                    print("⚠️ Some instances failed")
                    sys.exit(1)
            
            time.sleep(CHECK_INTERVAL)
            
    except KeyboardInterrupt:
        print("\n\n⚠️ Interrupted by user")
        print_status(instances)
        sys.exit(130)


if __name__ == "__main__":
    main()
