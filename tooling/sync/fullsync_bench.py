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
import fcntl
import json
import os
import shutil
import socket
import subprocess
import sys
import time
from datetime import datetime, timezone

import requests

from fullsync_metrics import parse_run, parse_run_file

HERE = os.path.dirname(os.path.abspath(__file__))

# Same `.env` convention as docker_monitor.py, but resolved against this file rather than
# the working directory: the watch runs unattended from wherever a service manager put it,
# and silently losing the Slack webhook to a `cd` is not a good failure.
_ENV_FILE = os.path.join(HERE, ".env")
if os.path.exists(_ENV_FILE):
    with open(_ENV_FILE) as _fh:
        for _line in _fh:
            _line = _line.strip()
            if _line and not _line.startswith("#"):
                _key, _, _value = _line.partition("=")
                os.environ.setdefault(_key.strip(), _value.strip())

COMPOSE_FILES = ["docker-compose.multisync.yaml", "docker-compose.fullsync-bench.yaml"]
COMPOSE_PROJECT = "fullsync-bench"

# Node data, base generations and results all live under here, and must stay on one
# filesystem — see `assert_same_filesystem`. Kept in step with BENCH_DATA_ROOT in the
# compose override, which bind-mounts `<root>/data/<net>` into each container.
DATA_ROOT = os.environ.get("BENCH_DATA_ROOT", "/mnt/raid10/fullsync-bench")
DEFAULT_RESULTS_DIR = os.path.join(DATA_ROOT, "results")

# One lock for the whole box: two measurement legs at once contend for CPU and disk and
# both results are junk. Cycles take it exclusively, bootstraps share it (see `take_lock`).
# The A/B tool (#7112) must take the same lock exclusively.
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

# One round per day. The base advances at chain rate, so measuring more often just resamples
# a window that has barely moved: the extra points are not independent and would make the
# trailing median look far more stable than the series really is.
CYCLE_INTERVAL_SECONDS = 24 * 3600
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
    """Host path bind-mounted into the node as /data."""
    return os.path.join(DATA_ROOT, "data", net)


def base_dir(state_root, net, generation=0):
    return os.path.join(state_root, net, f"base.{generation}")


def assert_same_filesystem(net, state_root):
    """Bases and live data must share a filesystem.

    `snapshot` relies on `rsync --link-dest` hardlinking unchanged SST files, which only
    works within one filesystem. Across two, rsync copies instead — no error, just every
    retained generation quietly costing a full database.
    """
    os.makedirs(data_dir(net), exist_ok=True)
    os.makedirs(os.path.join(state_root, net), exist_ok=True)
    if os.stat(data_dir(net)).st_dev != os.stat(os.path.join(state_root, net)).st_dev:
        raise SystemExit(
            f"{net}: data dir {data_dir(net)} and base dir {state_root}/{net} are on "
            "different filesystems; --link-dest would silently fall back to full copies"
        )


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


SLOT_SECONDS = 12

# Slots that actually carry a block. Deliberately below the real figure (~99% on mainnet):
# under-estimating the tip only makes a cycle wait a little longer, while over-estimating
# would aim a leg at a block nobody has produced, which costs its full timeout.
SLOT_FILL_RATIO = 0.97


def cl_is_current(net, timeout=8):
    """Whether the beacon node's head can be believed right now."""
    url = f"http://localhost:{NETWORKS[net]['cl_port']}/eth/v1/node/syncing"
    data = requests.get(url, timeout=timeout).json()["data"]
    return not data["el_offline"] and int(data["sync_distance"]) <= SYNCED_MARGIN_BLOCKS


def chain_tip(net, base):
    """Best available estimate of the network's current execution head.

    The beacon node cannot be asked for this between cycles. With the execution node
    stopped it reports `el_offline` and stops advancing its head — it sat 17 hours behind
    and unmoving on all three networks the first time this ran — so `tip - base` would read
    zero forever and every cycle would skip itself. Its head is therefore used only when it
    is demonstrably current, which in practice means during or just after a leg.

    Otherwise extrapolate from when the base was recorded. Slot timing is fixed and known,
    so elapsed wall clock gives a good enough tip for both callers: a "has enough chain
    accumulated yet" check, and an advance target a whole GAP below the tip.
    """
    try:
        if cl_is_current(net):
            return cl_head(net)
    except Exception:
        pass

    meta_path = os.path.join(base, BASE_META)
    with open(meta_path) as fh:
        meta = json.load(fh)
    # Bases written before `created_at` existed fall back to the metadata file's mtime,
    # which is when the base was recorded.
    created = meta.get("created_at", os.path.getmtime(meta_path))
    elapsed = max(0.0, time.time() - created)
    return meta["head"] + int(elapsed / SLOT_SECONDS * SLOT_FILL_RATIO)


def start_node(net):
    compose(["up", "-d", "--no-deps", f"ethrex-{net}"])


def start_consensus(net):
    """Bring up the beacon node (and its JWT setup)."""
    compose(["up", "-d", f"consensus-{net}"])


# Checkpoint sync is a download of one finalised state; minutes, not hours.
CL_REFRESH_TIMEOUT_SECONDS = 900


def refresh_consensus(net, min_block, timeout=CL_REFRESH_TIMEOUT_SECONDS):
    """Put the beacon node far enough ahead of the base to drive a real leg, and wait.

    This is load-bearing, not hygiene. The execution node only does bulk 1024-block batch
    sync when fork choice hands it a head far ahead of where it is. A beacon node that was
    stopped alongside it knows nothing newer than the base, so on restart the pair crawl
    forward together and the node imports ~30 blocks at a time: that measures incremental
    block import, not full sync, and emits no batch throughput metric at all.

    Recreating the container re-runs its `--purge-db-force` checkpoint sync, which lands it
    near the tip in a couple of minutes. It also regenerates the JWT, which is precisely why
    the execution node must be started *after* this returns.

    The wait is for the beacon node to know a block at or above this leg's target, not for
    it to reach the tip. It cannot reach the tip: with no execution node it has nothing to
    verify payloads against, so it parks at its checkpoint anchor a couple of epochs back
    and from there falls further behind at one slot per slot. Waiting for a small
    `sync_distance` would simply time out every time.
    """
    compose(["up", "-d", "--force-recreate", f"consensus-{net}"])
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            head = cl_head(net)
            if head >= min_block:
                log(f"{net}: beacon node at block {head}, ahead of the target {min_block}")
                return True
            log(f"{net}: beacon node at {head}, needs {min_block}")
        except Exception:
            pass
        time.sleep(10)
    return False


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


# ----------------------------------------------------------------------- bootstrap


# Snap-syncing mainnet from scratch is an overnight job, and a stall should not hang the
# runner forever.
BOOTSTRAP_TIMEOUT_SECONDS = 48 * 3600

# How close to the consensus tip counts as "caught up" — a couple of epochs of slack, so a
# node still importing the last few blocks is not called done early.
SYNCED_MARGIN_BLOCKS = 64


def bootstrap(net, state_root):
    """Create the first base for a network.

    Snap-syncs to the tip and snapshots the result. The node is then left stopped, and the
    gap the watch needs opens by itself at one day per day as the chain moves on — no
    manual anchoring. Cycles can start once head - base >= measure_blocks, which `cycle`
    checks and waits for.
    """
    cfg = NETWORKS[net]
    base = base_dir(state_root, net, 0)
    if os.path.isdir(base):
        raise SystemExit(f"{net}: base already exists at {base}; remove it to re-bootstrap")
    assert_same_filesystem(net, state_root)

    # Snap for the initial fill only: full-syncing from genesis would take weeks. The
    # measurement legs themselves always run full — that is the thing being measured.
    os.environ["BENCH_SYNCMODE"] = "snap"
    start_consensus(net)
    start_node(net)
    log(f"{net}: snap-syncing to tip; expect hours")

    deadline = time.time() + BOOTSTRAP_TIMEOUT_SECONDS
    head = None
    try:
        while True:
            try:
                head, tip = rpc_block_number(cfg["rpc_port"]), cl_head(net)
                if head >= tip - SYNCED_MARGIN_BLOCKS:
                    log(f"{net}: caught up at {head} (tip {tip})")
                    break
                log(f"{net}: {head} / {tip} ({tip - head} behind)")
            except Exception as exc:
                log(f"{net}: waiting for RPC/beacon ({exc})")
            if time.time() > deadline:
                raise SystemExit(f"{net}: bootstrap timed out after "
                                 f"{BOOTSTRAP_TIMEOUT_SECONDS}s at head {head}")
            time.sleep(60)
    finally:
        os.environ.pop("BENCH_SYNCMODE", None)
        stop_node(net)

    if not exited_cleanly(net):
        raise SystemExit(f"{net}: node did not exit cleanly; refusing to promote the base")

    os.makedirs(os.path.dirname(base), exist_ok=True)
    snapshot(net, base)
    write_base_head(base, head)
    log(f"{net}: base created at {base} (block {head}); "
        f"first cycle once the chain is {cfg['measure_blocks']} blocks past it")
    return head


# --------------------------------------------------------------------- the primitive


def run_leg(net, target_block, log_path, timeout_seconds):
    """Sync from the currently-restored state up to `target_block`, then stop.

    Pure: the caller decides what to do with the resulting state. Returns a LegResult
    dict; `status` is `ok` only for a run that reached its target and exited cleanly.
    """
    cfg = NETWORKS[net]

    # Before the clock starts: the node must be handed a distant head, or this measures
    # the wrong thing entirely. Costs a couple of minutes and is excluded from the timing.
    if not refresh_consensus(net, target_block):
        log(f"{net}: beacon node never learned block {target_block}; refusing to run a leg "
            "that would measure incremental import instead of full sync")
        return {"network": net, "target_block": target_block, "reached_block": None,
                "wall_seconds": 0.0, "status": "cl_not_synced", "log": log_path,
                **parse_run([])}

    drop_page_cache()
    started = time.time()
    # `docker logs -f` replays the container's whole history before following, and the
    # container is reused across legs — so without `--since` every run re-parses every
    # earlier run's batches and the reported throughput becomes a cumulative average.
    # Captured before the node starts so no output can slip in ahead of the cutoff.
    since = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%S")
    start_node(net)

    logger = subprocess.Popen(["docker", "logs", "-f", "--since", since, f"ethrex-{net}"],
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
        raise SystemExit(f"no base for {net} at {base}; run with --bootstrap first")

    assert_same_filesystem(net, state_root)
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    os.makedirs(os.path.join(results_dir, net), exist_ok=True)

    base_head = read_base_head(net, base, cfg)

    # The whole measurement window has to exist on-chain already. A base less than M
    # blocks behind the tip would send the leg after a target nobody has produced yet,
    # which can only end in its timeout hours later.
    try:
        tip = chain_tip(net, base)
    except Exception as exc:
        log(f"{net}: could not determine the chain tip ({exc}); skipping cycle")
        return None
    if tip - base_head < cfg["measure_blocks"]:
        log(f"{net}: gap is {tip - base_head} blocks, need {cfg['measure_blocks']}; "
            "waiting for it to open")
        return None

    # --- measurement leg: fixed M blocks, resulting state thrown away -----------
    restore(net, base)
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
    # Re-read rather than reusing the tip from before the measurement leg: that leg has
    # just spent hours running, and the chain moved on while it did.
    try:
        advance_target = chain_tip(net, base) - cfg["gap_blocks"]
    except Exception as exc:
        log(f"{net}: could not determine the chain tip ({exc}); skipping advance this cycle")
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

    # Only for a base created outside `bootstrap`, which has no metadata yet.
    log(f"{net}: base has no {BASE_META}; reading head once and recording it")
    restore(net, base)
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
    # `created_at` is what lets the tip be extrapolated while the beacon node is frozen.
    with open(os.path.join(base, BASE_META), "w") as fh:
        json.dump({"head": head, "created_at": time.time()}, fh)


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


def post_slack(lines, all_ok=True):
    """Report a round of cycles.

    A round where something broke goes to the failure webhook. Note what this does *not*
    mean while the watch is observe-only: a throughput number being low is not a failure,
    because there is no baseline to judge it against yet. Only a cycle that could not
    produce a valid measurement at all counts — a crashed or OOM-killed node, an unclean
    exit, a leg that never reached its target.
    """
    key = "SLACK_WEBHOOK_URL_FAILED" if not all_ok else "SLACK_WEBHOOK_URL_SUCCESS"
    url = os.environ.get(key)
    if not url and not all_ok:
        # Never drop a failure report just because the failure webhook is unconfigured.
        url = os.environ.get("SLACK_WEBHOOK_URL_SUCCESS")
        if url:
            log(f"{key} unset; sending the failure report to the success webhook")
    if not url:
        log(f"{key} unset; skipping Slack")
        return
    # Observe-only by design for now: thresholds and step detection land once the
    # bootstrap period has produced enough data to derive them (see #7111).
    header = ("Full-sync throughput (observe-only)" if all_ok
              else "Full-sync watch: cycle problem")
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


_LOCK_FD = None


def take_lock(shared=False):
    """Serialise against other runs on this box.

    Measurement legs must never overlap with anything else that loads the machine, so
    cycles take the lock exclusively. Bootstraps only take it shared: several networks can
    snap-sync at once — nothing is being measured yet — but a cycle still cannot start
    while one is in flight.

    Uses flock rather than an O_EXCL lockfile so the kernel drops the lock when the process
    dies. A lockfile outlives a SIGKILL or a reboot, which for an unattended watch means
    every later run refuses to start until someone removes it by hand.
    """
    global _LOCK_FD
    _LOCK_FD = os.open(LOCK_PATH, os.O_CREAT | os.O_RDWR)
    try:
        fcntl.flock(_LOCK_FD, (fcntl.LOCK_SH if shared else fcntl.LOCK_EX) | fcntl.LOCK_NB)
    except BlockingIOError:
        held = "a measurement cycle" if shared else "another run"
        raise SystemExit(f"{held} holds {LOCK_PATH}; refusing to run concurrently")
    os.truncate(_LOCK_FD, 0)
    os.write(_LOCK_FD, str(os.getpid()).encode())


def release_lock():
    global _LOCK_FD
    if _LOCK_FD is not None:
        fcntl.flock(_LOCK_FD, fcntl.LOCK_UN)
        os.close(_LOCK_FD)
        _LOCK_FD = None


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--networks", default="mainnet",
                        help="comma-separated (mainnet,sepolia,hoodi). Run serially.")
    parser.add_argument("--state-root", default=os.path.join(DATA_ROOT, "state"),
                        help="where per-network base generations live")
    parser.add_argument("--results-dir", default=DEFAULT_RESULTS_DIR)
    parser.add_argument("--bootstrap", action="store_true",
                        help="snap-sync each network to the tip and record it as the first "
                             "base, then exit. Run once per network before watching.")
    parser.add_argument("--once", action="store_true",
                        help="run a single cycle per network and exit")
    parser.add_argument("--measure-blocks", type=int,
                        help="override the measurement window. SMOKE TESTS ONLY — the "
                             "series is only comparable if this stays fixed, so a run "
                             "using it must also use a throwaway --results-dir.")
    parser.add_argument("--gap-blocks", type=int,
                        help="override the base's distance behind the tip. Smoke tests "
                             "only, for the same reason.")
    args = parser.parse_args()

    if (args.measure_blocks or args.gap_blocks) and args.results_dir == DEFAULT_RESULTS_DIR:
        # Mixing a 2k-block sample into a series of 21.6k-block ones would shift the
        # trailing median the comparison depends on.
        raise SystemExit("--measure-blocks/--gap-blocks change what is being measured; "
                         "point --results-dir somewhere throwaway so the real series is "
                         "not polluted")

    nets = [n.strip() for n in args.networks.split(",") if n.strip()]
    for net in nets:
        if net not in NETWORKS:
            raise SystemExit(f"unknown network {net!r}; known: {', '.join(NETWORKS)}")

    # Bootstraps may overlap each other; cycles get the box to themselves.
    for net in nets:
        if args.measure_blocks:
            NETWORKS[net]["measure_blocks"] = args.measure_blocks
        if args.gap_blocks:
            NETWORKS[net]["gap_blocks"] = args.gap_blocks

    take_lock(shared=args.bootstrap)
    try:
        if args.bootstrap:
            for net in nets:
                bootstrap(net, args.state_root)
            return 0

        while True:
            round_started = time.time()
            lines, all_ok = [], True
            for net in nets:  # serial on purpose: concurrent legs contend and both are junk
                try:
                    result = cycle(net, args.state_root, args.results_dir)
                    if result is None:  # gap not open yet, or no tip to compare against
                        continue
                    if result.get("status") != "ok":
                        all_ok = False
                    lines.append(summarise(result, load_history(args.results_dir, net)))
                except Exception as exc:  # isolate: one network must not stop the others
                    log(f"{net}: cycle failed: {exc}")
                    lines.append(f"*{net}*: cycle failed — `{exc}`")
                    all_ok = False
            if lines:
                post_slack(lines, all_ok=all_ok)
            if args.once:
                return 0
            # Pace on round start, not round end: a long round should not push the next
            # day's measurement later and later.
            time.sleep(max(0.0, CYCLE_INTERVAL_SECONDS - (time.time() - round_started)))
    finally:
        release_lock()


if __name__ == "__main__":
    sys.exit(main())
