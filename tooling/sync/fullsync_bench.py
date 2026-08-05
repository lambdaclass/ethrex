#!/usr/bin/env python3
"""Full-sync throughput regression watch — see issue #7111.

Keeps a node permanently a fixed distance behind head and, once per cycle per network:

    restore base -> run M blocks   (MEASURE, resulting state discarded)
    restore base -> advance to head-GAP  -> becomes the new base

The base creeps forward at chain rate, so the node never ages out of relevance and no
anchor or reference commit needs manual upkeep. The measurement window M is fixed and
independent of how fast the base moves; consecutive measurements therefore overlap
heavily, which is what keeps day-over-day workload variation small.

`run_leg` is deliberately pure — it never decides what happens to the resulting state,
never assumes a particular ref, and never rotates anything. That is what lets the manual
A/B tool (issue #7112) reuse it unchanged.
"""

import argparse
import json
import os
import shutil
import socket
import subprocess
import sys
import time
from datetime import datetime, timezone

import requests

from fullsync_metrics import parse_run_file

HERE = os.path.dirname(os.path.abspath(__file__))
COMPOSE_FILES = ["docker-compose.multisync.yaml", "docker-compose.fullsync-bench.yaml"]
COMPOSE_PROJECT = "fullsync-bench"

# One lock for the whole box: two legs running at once contend for CPU and disk and both
# results are junk. This also excludes the A/B tool (#7112), which must take the same lock.
LOCK_PATH = "/tmp/ethrex-fullsync-bench.lock"

# Blocks are ~12s on all three networks, so a nominal day is ~7200 blocks. `measure_blocks`
# is quantised to whole days so every sample spans the same diurnal composition, and must
# stay FIXED once a series starts — changing it breaks comparability of the history.
BLOCKS_PER_DAY = 7200

NETWORKS = {
    "mainnet": {"rpc_port": 8547, "cl_port": 5054,
                "measure_blocks": 3 * BLOCKS_PER_DAY, "gap_blocks": 6 * BLOCKS_PER_DAY},
    "sepolia": {"rpc_port": 8546, "cl_port": 5053,
                "measure_blocks": 3 * BLOCKS_PER_DAY, "gap_blocks": 6 * BLOCKS_PER_DAY},
    "hoodi":   {"rpc_port": 8545, "cl_port": 5052,
                "measure_blocks": 3 * BLOCKS_PER_DAY, "gap_blocks": 6 * BLOCKS_PER_DAY},
}

# Retained hardlink generations of each base, for rollback when a cycle ends badly.
# Hardlinks pin SST files that compaction would otherwise free, so this is capped.
KEEP_GENERATIONS = 5

STOP_TIMEOUT_SECONDS = 300  # graceful; SIGKILL corrupts the resume state (see #7111)
POLL_SECONDS = 20


def log(msg):
    print(f"[{datetime.now(timezone.utc).isoformat(timespec='seconds')}] {msg}", flush=True)


def run(cmd, check=True, capture=False):
    log(f"$ {' '.join(cmd)}")
    return subprocess.run(cmd, check=check, text=True,
                          stdout=subprocess.PIPE if capture else None)


def compose(args, check=True):
    cmd = ["docker", "compose", "-p", COMPOSE_PROJECT]
    for f in COMPOSE_FILES:
        cmd += ["-f", os.path.join(HERE, f)]
    return run(cmd + args, check=check)


# --------------------------------------------------------------------------- paths


def data_dir(net):
    """Host path of the node's data volume."""
    return f"/var/lib/docker/volumes/{COMPOSE_PROJECT}_ethrex-{net}/_data"


def base_dir(state_root, net, generation=0):
    return os.path.join(state_root, net, f"base.{generation}")


# ------------------------------------------------------------------- node control


def rpc_block_number(port, timeout=8):
    """Current canonical head.

    Deliberately `eth_blockNumber` and not `eth_syncing.currentBlock`: the latter goes
    stale during catch-up, which once let a benchmark leg run far past its stop target.
    """
    body = {"jsonrpc": "2.0", "method": "eth_blockNumber", "params": [], "id": 1}
    resp = requests.post(f"http://localhost:{port}", json=body, timeout=timeout).json()
    return int(resp["result"], 16)


def cl_head(net, timeout=8):
    """Consensus head, i.e. the tip the node would sync toward."""
    url = f"http://localhost:{NETWORKS[net]['cl_port']}/eth/v2/beacon/blocks/head"
    resp = requests.get(url, timeout=timeout).json()
    return int(resp["data"]["message"]["body"]["execution_payload"]["block_number"])


def start_node(net):
    compose(["up", "-d", "--no-deps", f"ethrex-{net}"])


def stop_node(net):
    """Graceful stop. Never SIGKILL: an abrupt stop can leave the canonical head ahead of
    any durably-flushed state, after which the node cannot regenerate and needs a re-sync."""
    run(["docker", "stop", "-t", str(STOP_TIMEOUT_SECONDS), f"ethrex-{net}"], check=False)


def exited_cleanly(net):
    out = run(["docker", "inspect", f"ethrex-{net}", "--format", "{{.State.ExitCode}}"],
              check=False, capture=True)
    return (out.stdout or "").strip() == "0"


def drop_page_cache():
    """Cold-start parity between runs."""
    subprocess.run(["sync"], check=False)
    try:
        with open("/proc/sys/vm/drop_caches", "w") as fh:
            fh.write("3")
    except OSError as exc:  # non-Linux dev box, or not root
        log(f"could not drop page cache ({exc}); continuing")


# ------------------------------------------------------------------ snapshot / base


def restore(net, src):
    """Reset the live data dir to `src`."""
    run(["rsync", "-a", "--delete", f"{src}/", f"{data_dir(net)}/"])


def snapshot(net, dest, link_dest=None):
    """Hardlink snapshot of the live data dir.

    RocksDB SST files are immutable and uniquely named, so `--link-dest` against the
    previous generation makes a new base cost only its delta rather than a full copy.
    """
    cmd = ["rsync", "-a", "--delete"]
    if link_dest and os.path.isdir(link_dest):
        cmd += [f"--link-dest={link_dest}"]
    run(cmd + [f"{data_dir(net)}/", f"{dest}/"])


def rotate_generations(state_root, net):
    """Shift base.N -> base.N+1, dropping the oldest."""
    oldest = base_dir(state_root, net, KEEP_GENERATIONS - 1)
    if os.path.isdir(oldest):
        shutil.rmtree(oldest)
    for gen in range(KEEP_GENERATIONS - 2, -1, -1):
        src = base_dir(state_root, net, gen)
        if os.path.isdir(src):
            os.rename(src, base_dir(state_root, net, gen + 1))


# --------------------------------------------------------------------- the primitive


def run_leg(net, target_block, log_path, timeout_seconds):
    """Sync from the currently-restored state up to `target_block`, then stop.

    Pure: the caller decides what to do with the resulting state. Returns a LegResult
    dict; `status` is `ok` only for a run that reached its target and exited cleanly.
    """
    cfg = NETWORKS[net]
    drop_page_cache()
    started = time.time()
    start_node(net)

    logger = subprocess.Popen(["docker", "logs", "-f", f"ethrex-{net}"],
                              stdout=open(log_path, "w"), stderr=subprocess.STDOUT)
    status, head = "ok", None
    try:
        deadline = started + timeout_seconds
        while True:
            try:
                head = rpc_block_number(cfg["rpc_port"])
            except Exception:
                pass  # RPC not up yet, or a transient blip; keep the last known head
            if head is not None and head >= target_block:
                break
            if time.time() > deadline:
                status = "timeout"
                break
            time.sleep(POLL_SECONDS)
    finally:
        stop_node(net)
        logger.terminate()

    if status == "ok" and not exited_cleanly(net):
        # An OOM-kill or crash leaves the DB in a state we must not promote to a base.
        status = "unclean_exit"

    metrics = parse_run_file(log_path)
    if status == "ok" and not metrics["batches"]:
        status = "no_batches"

    return {
        "network": net,
        "target_block": target_block,
        "reached_block": head,
        "wall_seconds": round(time.time() - started, 1),
        "status": status,
        "log": log_path,
        **metrics,
    }


# ------------------------------------------------------------------------- the cycle


def cycle(net, state_root, results_dir):
    """One measure-and-advance cycle for a single network."""
    cfg = NETWORKS[net]
    base = base_dir(state_root, net, 0)
    if not os.path.isdir(base):
        raise SystemExit(f"no base for {net} at {base}; bootstrap it first (see --help)")

    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    os.makedirs(os.path.join(results_dir, net), exist_ok=True)

    # --- measurement leg: fixed M blocks, resulting state thrown away -----------
    restore(net, base)
    base_head = read_base_head(net, base, cfg)
    measure_target = base_head + cfg["measure_blocks"]
    measure_log = os.path.join(results_dir, net, f"{stamp}.measure.log")
    result = run_leg(net, measure_target, measure_log,
                     timeout_seconds=_leg_timeout(cfg["measure_blocks"]))
    result["kind"] = "measure"
    result["base_block"] = base_head
    result["commit"] = git_commit()
    result["host"] = socket.gethostname()
    result["timestamp"] = stamp
    _write_result(results_dir, net, stamp, result)

    # --- advance leg: gap-maintaining, becomes the next base --------------------
    # Target head-GAP rather than a fixed block count, so the advance self-calibrates to
    # the network's real block production and absorbs missed slots.
    try:
        advance_target = cl_head(net) - cfg["gap_blocks"]
    except Exception as exc:
        log(f"{net}: could not read consensus head ({exc}); skipping advance this cycle")
        return result
    if advance_target <= base_head:
        log(f"{net}: gap has not opened yet (base {base_head} >= target {advance_target}); "
            "skipping advance")
        return result

    restore(net, base)
    advance_log = os.path.join(results_dir, net, f"{stamp}.advance.log")
    advanced = run_leg(net, advance_target, advance_log,
                       timeout_seconds=_leg_timeout(advance_target - base_head))

    if advanced["status"] != "ok":
        log(f"{net}: advance leg ended '{advanced['status']}'; keeping the previous base")
        return result

    # --- health-gate the rotation ----------------------------------------------
    # Promote the new state only once the node demonstrably restarts on it with state
    # available; otherwise a silently-degraded base would poison every later cycle.
    if not base_is_healthy(net, advanced["reached_block"]):
        log(f"{net}: new base failed its health check; keeping the previous base")
        return result

    rotate_generations(state_root, net)
    snapshot(net, base_dir(state_root, net, 0), link_dest=base_dir(state_root, net, 1))
    write_base_head(base_dir(state_root, net, 0), advanced["reached_block"])
    log(f"{net}: base advanced to {advanced['reached_block']}")
    return result


BASE_META = "bench-base.json"


def read_base_head(net, base, cfg):
    """Block number the base sits at.

    Recorded alongside the base when it is created, so a cycle does not need an extra
    node start/stop just to ask — each start pays a state-regeneration cost we would
    otherwise incur twice per cycle for nothing.
    """
    meta_path = os.path.join(base, BASE_META)
    if os.path.isfile(meta_path):
        with open(meta_path) as fh:
            return json.load(fh)["head"]

    # Bootstrap only: an externally-created base has no metadata yet.
    log(f"{net}: base has no {BASE_META}; reading head once and recording it")
    start_node(net)
    try:
        for _ in range(60):
            try:
                head = rpc_block_number(cfg["rpc_port"])
                write_base_head(base, head)
                return head
            except Exception:
                time.sleep(2)
        raise SystemExit(f"{net}: RPC never came up after restore")
    finally:
        stop_node(net)


def write_base_head(base, head):
    with open(os.path.join(base, BASE_META), "w") as fh:
        json.dump({"head": head}, fh)


def base_is_healthy(net, expected_head):
    """Start briefly and confirm the node reports the expected head with state available."""
    cfg = NETWORKS[net]
    start_node(net)
    try:
        for _ in range(60):
            try:
                head = rpc_block_number(cfg["rpc_port"])
            except Exception:
                time.sleep(2)
                continue
            if head < expected_head:
                return False
            body = {"jsonrpc": "2.0", "method": "eth_getBalance",
                    "params": ["0x0000000000000000000000000000000000000000", hex(head)], "id": 1}
            resp = requests.post(f"http://localhost:{cfg['rpc_port']}", json=body, timeout=8).json()
            # A node whose head state is missing answers with an error, not a balance.
            return "result" in resp
        return False
    except Exception as exc:
        log(f"{net}: health check errored ({exc})")
        return False
    finally:
        stop_node(net)


def _leg_timeout(blocks):
    """Generous ceiling: legs are expected around 7 blocks/s, so allow well under 1/s."""
    return max(3600, int(blocks / 1.0))


def git_commit():
    try:
        return subprocess.check_output(["git", "rev-parse", "--short", "HEAD"],
                                       text=True).strip()
    except subprocess.CalledProcessError:
        return None


def _write_result(results_dir, net, stamp, result):
    path = os.path.join(results_dir, net, f"{stamp}.json")
    with open(path, "w") as fh:
        json.dump(result, fh, indent=2)
    log(f"{net}: wrote {path}")


# ---------------------------------------------------------------------- reporting


def load_history(results_dir, net, limit=30):
    folder = os.path.join(results_dir, net)
    if not os.path.isdir(folder):
        return []
    out = []
    for name in sorted(os.listdir(folder))[-limit:]:
        if not name.endswith(".json"):
            continue
        with open(os.path.join(folder, name)) as fh:
            entry = json.load(fh)
        if entry.get("kind") == "measure" and entry.get("status") == "ok":
            out.append(entry)
    return out


def summarise(result, history):
    """Human-readable line plus the trailing-median delta, when there is enough history.

    Deliberately reports against the trailing median rather than the previous run: a
    regression is a step, and a pairwise day-to-day difference carries far more noise.
    """
    mean = result["throughput_ggas_s"]["mean"]
    if mean is None:
        return f"*{result['network']}*: run invalid (`{result['status']}`)"

    line = (f"*{result['network']}*: {mean:.4f} Ggas/s "
            f"over {result['batches']} batches (base {result.get('base_block')})")
    previous = [h["throughput_ggas_s"]["mean"] for h in history
                if h.get("timestamp") != result.get("timestamp")]
    if len(previous) >= 3:
        ordered = sorted(previous)
        median = ordered[len(ordered) // 2]
        line += f" — {100 * (mean - median) / median:+.1f}% vs {len(previous)}-run median"
    else:
        line += " — baseline still building"
    return line


def post_slack(lines):
    url = os.environ.get("SLACK_WEBHOOK_URL_SUCCESS")
    if not url:
        log("SLACK_WEBHOOK_URL_SUCCESS unset; skipping Slack")
        return
    # Observe-only by design for now: thresholds and step detection land once the
    # bootstrap period has produced enough data to derive them (see #7111).
    header = "Full-sync throughput (observe-only)"
    message = {"blocks": [
        {"type": "header", "text": {"type": "plain_text", "text": header}},
        {"type": "section", "text": {"type": "mrkdwn", "text": "\n".join(lines)}},
    ]}
    try:
        resp = requests.post(url, data=json.dumps(message),
                             headers={"Content-Type": "application/json"}, timeout=15)
        if resp.status_code != 200:
            log(f"Slack returned {resp.status_code}")
    except Exception as exc:
        log(f"Slack post failed: {exc}")


# --------------------------------------------------------------------------- main


def take_lock():
    try:
        fd = os.open(LOCK_PATH, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
    except FileExistsError:
        raise SystemExit(f"another bench run holds {LOCK_PATH}; refusing to run concurrently")
    os.write(fd, str(os.getpid()).encode())
    os.close(fd)


def release_lock():
    try:
        os.unlink(LOCK_PATH)
    except FileNotFoundError:
        pass


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--networks", default="mainnet",
                        help="comma-separated (mainnet,sepolia,hoodi). Run serially.")
    parser.add_argument("--state-root", default="/root/fullsync-bench/state",
                        help="where per-network base generations live")
    parser.add_argument("--results-dir", default="/root/fullsync-bench/results")
    parser.add_argument("--once", action="store_true",
                        help="run a single cycle per network and exit")
    args = parser.parse_args()

    nets = [n.strip() for n in args.networks.split(",") if n.strip()]
    for net in nets:
        if net not in NETWORKS:
            raise SystemExit(f"unknown network {net!r}; known: {', '.join(NETWORKS)}")

    take_lock()
    try:
        while True:
            lines = []
            for net in nets:  # serial on purpose: concurrent legs contend and both are junk
                try:
                    result = cycle(net, args.state_root, args.results_dir)
                    lines.append(summarise(result, load_history(args.results_dir, net)))
                except Exception as exc:  # isolate: one network must not stop the others
                    log(f"{net}: cycle failed: {exc}")
                    lines.append(f"*{net}*: cycle failed — `{exc}`")
            post_slack(lines)
            if args.once:
                return 0
            time.sleep(3600)
    finally:
        release_lock()


if __name__ == "__main__":
    sys.exit(main())
