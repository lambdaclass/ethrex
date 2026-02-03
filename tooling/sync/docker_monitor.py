#!/usr/bin/env python3
"""Monitor Docker Compose snapsync instances for sync completion."""

import argparse
import os
import re
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

CHECK_INTERVAL = int(os.environ.get("CHECK_INTERVAL", 10))
SYNC_TIMEOUT = int(os.environ.get("SYNC_TIMEOUT", 8 * 60))  # default 8 hours (in minutes)
BLOCK_PROCESSING_DURATION = int(os.environ.get("BLOCK_PROCESSING_DURATION", 22 * 60))  # default 22 minutes (in seconds)
BLOCK_STALL_TIMEOUT = int(os.environ.get("BLOCK_STALL_TIMEOUT", 10 * 60))  # default 10 minutes (in seconds)
NODE_UNRESPONSIVE_TIMEOUT = int(os.environ.get("NODE_UNRESPONSIVE_TIMEOUT", 5 * 60))  # default 5 minutes (in seconds)
STATUS_PRINT_INTERVAL = int(os.environ.get("STATUS_PRINT_INTERVAL", 30))

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
    first_failure_time: float = 0
    validation_status: str = ""  # Current validation progress (if any)

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


def git_pull_latest(branch: str, ethrex_dir: str) -> tuple[bool, str]:
    """Fetch and pull latest changes from the specified branch.

    Returns (success, new_commit_hash).
    """
    try:
        print(f"📥 Pulling latest from branch '{branch}'...")
        # Fetch all remotes
        subprocess.run(["git", "fetch", "--all"], cwd=ethrex_dir, check=True, capture_output=True)
        # Checkout the branch
        subprocess.run(["git", "checkout", branch], cwd=ethrex_dir, check=True, capture_output=True)
        # Pull latest from origin explicitly
        subprocess.run(["git", "pull", "origin", branch], cwd=ethrex_dir, check=True, capture_output=True)
        # Get new commit hash
        new_commit = subprocess.check_output(
            ["git", "rev-parse", "--short", "HEAD"],
            cwd=ethrex_dir,
            stderr=subprocess.DEVNULL
        ).decode().strip()
        print(f"✅ Updated to commit {new_commit}")
        return True, new_commit
    except subprocess.CalledProcessError as e:
        error_details = ""
        if e.stderr:
            error_details = e.stderr.decode(errors='replace').strip()
        print(f"❌ Failed to pull latest: {e}")
        if error_details:
            print(f"   {error_details}")
        return False, ""


def build_docker_image(profile: str, image_tag: str, ethrex_dir: str) -> bool:
    """Build the Docker image with the specified profile.

    Args:
        profile: Cargo build profile (e.g., 'release-with-debug-assertions')
        image_tag: Docker image tag (e.g., 'ethrex-local:validate')
        ethrex_dir: Path to ethrex repository root
    """
    print(f"🔨 Building Docker image with profile '{profile}'...")
    print(f"   Image tag: {image_tag}")
    try:
        subprocess.run(
            [
                "docker", "build",
                "--build-arg", f"PROFILE={profile}",
                "-t", image_tag,
                "-f", f"{ethrex_dir}/Dockerfile",
                ethrex_dir
            ],
            check=True
        )
        print("✅ Docker image built successfully")
        return True
    except subprocess.CalledProcessError as e:
        print(f"❌ Failed to build Docker image: {e}")
        return False


def container_start_time(name: str) -> Optional[float]:
    try:
        out = subprocess.check_output(["docker", "inspect", "-f", "{{.State.StartedAt}}", name], stderr=subprocess.DEVNULL).decode().strip()
        if '.' in out:
            base, frac = out.rsplit('.', 1)
            out = f"{base}.{frac.rstrip('Z')[:6]}"
        return datetime.fromisoformat(out.replace('Z', '+00:00')).timestamp()
    except Exception:
        return None


def container_exit_info(name: str) -> tuple[Optional[bool], Optional[int]]:
    """Check if container has exited and get exit code.

    Returns:
        (is_running, exit_code) - is_running is True if running, False if exited, None on error.
        exit_code is the exit code if exited, None otherwise.
    """
    try:
        # Get container state
        status = subprocess.check_output(
            ["docker", "inspect", "-f", "{{.State.Status}}", name],
            stderr=subprocess.DEVNULL
        ).decode().strip()

        if status == "running":
            return True, None
        elif status == "exited":
            exit_code = subprocess.check_output(
                ["docker", "inspect", "-f", "{{.State.ExitCode}}", name],
                stderr=subprocess.DEVNULL
            ).decode().strip()
            return False, int(exit_code)
        else:
            return None, None
    except Exception:
        return None, None


# Validation error patterns to look for in container logs
VALIDATION_ERROR_PATTERNS = [
    "We have failed the validation of the state tree",
    "validate_storage_root",
    "Missing code hash",
    "Node count mismatch",
    "TrieError::Verify",
]

# Validation progress patterns - indicate validation is running
VALIDATION_PROGRESS_PATTERNS = [
    ("Starting validate_state_root", "validating state root"),
    ("Starting validate_storage_root", "validating storage roots"),
    ("Starting validate_bytecodes", "validating bytecodes"),
    ("Finished validate_storage_root", "storage validation complete"),
    ("Succesfully validated tree", "state validation complete"),
]


def check_validation_failure(container: str) -> Optional[str]:
    """Check container logs for validation failure messages.

    Returns a validation error message if found, None otherwise.
    """
    try:
        # Get last 100 lines of logs (validation errors should be near the end)
        logs = subprocess.check_output(
            ["docker", "logs", "--tail", "100", container],
            stderr=subprocess.STDOUT,
            timeout=10
        ).decode(errors='replace')

        for pattern in VALIDATION_ERROR_PATTERNS:
            if pattern in logs:
                return f"State trie validation failed: found '{pattern}' in logs"
        return None
    except Exception:
        return None


def check_validation_progress(container: str) -> Optional[str]:
    """Check container logs for validation progress.

    Returns the latest validation status if validation is in progress, None otherwise.
    """
    try:
        # Get last 200 lines of logs to catch validation messages
        logs = subprocess.check_output(
            ["docker", "logs", "--tail", "200", container],
            stderr=subprocess.STDOUT,
            timeout=10
        ).decode(errors='replace')

        # Find the most recent validation progress message
        latest_status = None
        latest_pos = -1
        for pattern, status in VALIDATION_PROGRESS_PATTERNS:
            pos = logs.rfind(pattern)
            if pos > latest_pos:
                latest_pos = pos
                latest_status = status

        return latest_status
    except Exception:
        return None


def rpc_call(url: str, method: str) -> Optional[Any]:
    try:
        return requests.post(url, json={"jsonrpc": "2.0", "method": method, "params": [], "id": 1}, timeout=5).json().get("result")
    except Exception:
        return None


def slack_notify(run_id: str, run_count: int, instances: list, hostname: str, branch: str, commit: str, build_profile: str = ""):
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

    # Validation is enabled when using debug-assertions profile
    validation_enabled = "debug-assertions" in build_profile
    validation_str = "enabled ✓" if validation_enabled else "disabled"

    # Check for validation failures
    validation_failures = [i for i in instances if i.error and "validation" in i.error.lower()]

    summary = (
        f"*Host:* `{hostname}`\n"
        f"*Branch:* `{branch}`\n"
        f"*Commit:* <https://github.com/lambdaclass/ethrex/commit/{commit}|{commit}>\n"
        f"*Elapsed:* `{elapsed_str}`\n"
        f"*Validation:* `{validation_str}`\n"
        f"*Logs:* `tooling/sync/multisync_logs/run_{run_id}`\n"
        f"*Result:* {'SUCCESS' if all_success else 'FAILED'}"
    )
    blocks = [
        {"type": "header", "text": {"type": "plain_text", "text": header}},
        {"type": "section", "text": {"type": "mrkdwn", "text": summary}},
        {"type": "divider"}
    ]

    # Add validation failure warning if any
    if validation_failures:
        blocks.append({
            "type": "section",
            "text": {"type": "mrkdwn", "text": "⚠️ *State trie validation failed!* Check logs for details."}
        })

    for i in instances:
        icon = "✅" if i.status == "success" else "❌"
        line = f"{icon} *{i.name}*: `{i.status}`"
        if i.sync_time:
            line += f" (sync: {fmt_time(i.sync_time)})"
        # Show validation status if it was tracked
        if i.validation_status:
            line += f" [validation: {i.validation_status}]"
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
        # Slack notification failures are non-critical; ignore them so they
        # do not interfere with the main monitoring workflow.
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


def log_run_result(run_id: str, run_count: int, instances: list[Instance], hostname: str, branch: str, commit: str, build_profile: str = ""):
    """Append run result to the persistent log file."""
    ensure_logs_dir()
    all_success = all(i.status == "success" for i in instances)
    status_icon = "✅" if all_success else "❌"
    run_start = datetime.strptime(run_id, "%Y%m%d_%H%M%S")
    elapsed_secs = (datetime.now() - run_start).total_seconds()
    elapsed_str = fmt_time(elapsed_secs)

    # Validation is enabled when using debug-assertions profile
    validation_enabled = "debug-assertions" in build_profile
    validation_str = "enabled" if validation_enabled else "disabled"
    validation_failures = [i for i in instances if i.error and "validation" in i.error.lower()]

    # Build log entry as plain text
    lines = [
        f"\n{'='*60}",
        f"{status_icon} Run #{run_count} (ID: {run_id})",
        f"{'='*60}",
        f"Host:       {hostname}",
        f"Branch:     {branch}",
        f"Commit:     {commit}",
        f"Elapsed:    {elapsed_str}",
        f"Validation: {validation_str}",
        f"Result:     {'SUCCESS' if all_success else 'FAILED'}",
        "",
    ]

    if validation_failures:
        lines.append("⚠️  VALIDATION FAILED - State trie validation errors detected!")
        lines.append("")

    for inst in instances:
        icon = "✅" if inst.status == "success" else "❌"
        line = f"  {icon} {inst.name}: {inst.status}"
        if inst.sync_time:
            line += f" (sync: {fmt_time(inst.sync_time)})"
        if inst.validation_status:
            line += f" [validation: {inst.validation_status}]"
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


def get_next_run_count() -> int:
    """Get the next run count by parsing the run history log.

    This provides persistence across restarts - if run #3 failed and we restart,
    the next run will be #4 instead of starting from #1 again.
    """
    if not RUN_LOG_FILE.exists():
        return 1

    try:
        content = RUN_LOG_FILE.read_text()
        # Find all "Run #N" patterns and get the highest number
        matches = re.findall(r'Run #(\d+)', content)
        if matches:
            max_run = max(int(m) for m in matches)
            return max_run + 1
        return 1
    except Exception:
        return 1


def restart_containers(compose_file: str, compose_dir: str, networks: list[str] = None, image_tag: str = ""):
    """Stop and restart docker compose containers, clearing volumes.

    Args:
        compose_file: Docker compose file name
        compose_dir: Directory containing docker compose file
        networks: Optional list of network names (for selective restart)
        image_tag: Optional image tag override (sets ETHREX_IMAGE env var)
    """
    print("\n🔄 Restarting containers...\n", flush=True)
    try:
        subprocess.run(["docker", "compose", "-f", compose_file, "down", "-v"], cwd=compose_dir, check=True)
        time.sleep(5)

        env = os.environ.copy()
        if image_tag:
            env["ETHREX_IMAGE"] = image_tag
            env["ETHREX_PULL_POLICY"] = "never"

        # Build service list if networks specified
        cmd = ["docker", "compose", "-f", compose_file, "up", "-d"]
        if networks:
            for n in networks:
                cmd.extend([f"setup-jwt-{n}", f"ethrex-{n}", f"consensus-{n}"])

        subprocess.run(cmd, cwd=compose_dir, check=True, env=env)
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
    inst.first_failure_time = 0
    inst.validation_status = ""


def print_status(instances: list[Instance]):
    print("\033[2J\033[H", end="")
    print(f"{'='*60}\nStatus at {time.strftime('%H:%M:%S')}\n{'='*60}")

    for i in instances:
        elapsed = time.time() - i.start_time if i.start_time else 0
        # Build status-specific extra info
        if i.status == "syncing":
            validation_info = f" [{i.validation_status}]" if i.validation_status else ""
            extra = f" ({fmt_time(elapsed)} elapsed){validation_info}"
        elif i.status == "waiting":
            extra = " (waiting for node...)"
        elif i.status == "synced":
            extra = f" (synced in {fmt_time(i.sync_time)})"
        elif i.status == "block_processing":
            extra = f" (block {i.last_block}, +{i.last_block - i.initial_block} blocks, {fmt_time(BLOCK_PROCESSING_DURATION - (time.time() - i.block_check_start))} left)"
        elif i.status == "success":
            extra = f" ✓ synced in {fmt_time(i.sync_time)}, processed +{i.last_block - i.initial_block} blocks"
        elif i.status == "failed":
            extra = f" - {i.error}"
        else:
            extra = ""
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
            if inst.first_failure_time == 0:
                inst.first_failure_time = now
            elif (now - inst.first_failure_time) >= NODE_UNRESPONSIVE_TIMEOUT:
                first_fail_str = datetime.fromtimestamp(inst.first_failure_time).strftime("%H:%M:%S")

                # Check if container exited (possibly due to validation failure)
                is_running, exit_code = container_exit_info(inst.container)
                if is_running is False and exit_code is not None:
                    # Container exited - check for validation failure
                    validation_error = check_validation_failure(inst.container)
                    if validation_error:
                        inst.status, inst.error = "failed", validation_error
                    else:
                        inst.status, inst.error = "failed", f"Container exited with code {exit_code} (first failure at {first_fail_str})"
                else:
                    inst.status, inst.error = "failed", f"Node stopped responding (first failure at {first_fail_str}, down for {fmt_time(now - inst.first_failure_time)})"
                return True
        return False

    inst.first_failure_time = 0
    
    if inst.status == "waiting":
        inst.status, inst.start_time = "syncing", inst.start_time or now
        return True
    
    if inst.status == "syncing":
        if (now - inst.start_time) > timeout_min * 60:
            inst.status, inst.error = "failed", f"Sync timeout after {fmt_time(timeout_min * 60)}"
            return True
        # Check for validation progress (shows when validation is running)
        validation_progress = check_validation_progress(inst.container)
        if validation_progress and validation_progress != inst.validation_status:
            inst.validation_status = validation_progress
            return True  # Status changed, refresh display
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
                # If validation was running, mark it as complete
                if inst.validation_status:
                    inst.validation_status = "complete"
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
    # Auto-update and build options
    p.add_argument("--auto-update", action="store_true",
                   help="Pull latest code and rebuild Docker image before each run")
    p.add_argument("--branch", default=os.environ.get("MULTISYNC_BRANCH", ""),
                   help="Git branch to track (default: from MULTISYNC_BRANCH env or current branch)")
    p.add_argument("--build-profile", default=os.environ.get("MULTISYNC_BUILD_PROFILE", "release-with-debug-assertions"),
                   help="Cargo build profile for Docker image")
    p.add_argument("--image-tag", default=os.environ.get("MULTISYNC_LOCAL_IMAGE", "ethrex-local:multisync"),
                   help="Docker image tag to build")
    p.add_argument("--ethrex-dir", default=os.environ.get("ETHREX_DIR", "../.."),
                   help="Path to ethrex repository root")
    args = p.parse_args()

    # Resolve ethrex directory to absolute path
    ethrex_dir = os.path.abspath(args.ethrex_dir)
    
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
    branch = args.branch if args.branch else git_branch()
    commit = git_commit()

    # Ensure logs directory exists first (needed for run count)
    ensure_logs_dir()

    # Get run count from existing logs (persists across restarts)
    run_count = get_next_run_count()
    run_id = generate_run_id()

    print(f"📁 Logs will be saved to {LOGS_DIR.absolute()}")
    print(f"📝 Run history: {RUN_LOG_FILE.absolute()}")
    if args.auto_update:
        print(f"🔄 Auto-update enabled: tracking branch '{branch}'")
        print(f"   Build profile: {args.build_profile}")
        print(f"   Image tag: {args.image_tag}")
    print()
    
    try:
        while True:
            # Auto-update: pull latest and rebuild before each run
            if args.auto_update:
                print(f"\n{'='*60}")
                print(f"🔄 Auto-update: Preparing run #{run_count}")
                print(f"{'='*60}")
                success, new_commit = git_pull_latest(branch, ethrex_dir)
                if not success:
                    print("❌ Failed to pull latest code, aborting")
                    sys.exit(1)
                commit = new_commit  # Update commit for logging
                if not build_docker_image(args.build_profile, args.image_tag, ethrex_dir):
                    print("❌ Failed to build Docker image, aborting")
                    sys.exit(1)

                # Start/restart containers with the new image
                if not restart_containers(args.compose_file, args.compose_dir, names, args.image_tag):
                    print("❌ Failed to start containers", file=sys.stderr)
                    sys.exit(1)
                # Reset instances since we restarted
                for inst in instances:
                    reset_instance(inst)
                time.sleep(30)  # Wait for containers to start
                print(f"{'='*60}\n")

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
            log_run_result(run_id, run_count, instances, hostname, branch, commit, args.build_profile)
            # Send a single Slack summary notification for the run
            if not args.no_slack:
                slack_notify(run_id, run_count, instances, hostname, branch, commit, args.build_profile)
            # Check results
            if all(i.status == "success" for i in instances):
                print(f"🎉 Run #{run_count}: All instances synced successfully!")
                if args.exit_on_success:
                    sys.exit(0)
                # Prepare for another run
                run_count += 1
                run_id = generate_run_id()  # New run ID for the new cycle

                # If auto-update is enabled, the loop will pull/build/restart
                # Otherwise, just restart containers now
                if not args.auto_update:
                    image_tag = args.image_tag if args.image_tag != "ethrex-local:multisync" else ""
                    if restart_containers(args.compose_file, args.compose_dir, names, image_tag):
                        for inst in instances:
                            reset_instance(inst)
                        time.sleep(30)  # Wait for containers to start
                    else:
                        print("❌ Failed to restart containers", file=sys.stderr)
                        sys.exit(1)
                else:
                    # Reset instances - containers will be restarted after pull/build
                    for inst in instances:
                        reset_instance(inst)
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
