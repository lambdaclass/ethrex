"""Metric extraction from an ethrex full-sync run log.

Kept separate from the runner so it can be unit-tested against captured logs without
needing a node, a box or Docker. See issue #7111.

Throughput is reported as the mean of the per-batch `Gigagas/s` values, deliberately
*not* as blocks divided by wall clock: wall clock includes startup state regeneration
(the node re-executes the uncommitted window on restart), which is a real signal but a
separate one, so it is extracted on its own instead of being folded into throughput.
"""

import re
from datetime import datetime

# `... [METRICS] Executed and stored: Range: 1024, Last block num: 25578362, ...,
#  Total Gas: 31058623694, Throughput: 0.21086104043277357 Gigagas/s`
# The `(unified pipeline)` variant of the same line is also matched.
BATCH_RE = re.compile(
    r"\[METRICS\] Executed and stored.*?"
    r"Last block num: (?P<last_block>\d+).*?"
    r"Total Gas: (?P<gas>\d+).*?"
    r"Throughput: (?P<throughput>[0-9.]+) Gigagas/s"
)

# `... Executed and stored 1024 blocks in 78.813 seconds (12.993 blocks/s). ...`
BLOCKS_PER_S_RE = re.compile(
    r"Executed and stored \d+ blocks in [0-9.]+ seconds \((?P<bps>[0-9.]+) blocks/s\)"
)

# `... [METRIC] BLOCK 500 0x5f69… | 1.750 Ggas/s | 51.41 ms | 2569 txs | 90 Mgas (98%)`
BLOCK_HEADER_RE = re.compile(
    r"\[METRIC\] BLOCK (?P<number>\d+) .*?\| [0-9.]+ Ggas/s \| (?P<total_ms>[0-9.]+) ms "
    r"\| \d+ txs \| (?P<mgas>[0-9.]+) Mgas"
)

# `  |- exec:       47.41 ms  (92%) << BOTTLENECK`  /  `  `- warmer:  20.33 ms  [...]`
PHASE_RE = re.compile(r"[|`]- (?P<phase>validate|exec|merkle|store): +(?P<ms>[0-9.]+) ms")

REGEN_START_RE = re.compile(r"Regenerating state from block (?P<from>\d+) to (?P<to>\d+)")
REGEN_END_RE = re.compile(r"Finished regenerating state")

# Leading tracing timestamp, e.g. `2026-07-28T15:06:49.398765Z  INFO ...`
TS_RE = re.compile(r"(?P<ts>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?)Z")

# Phases whose cost we track per unit of gas. `merkle` is the drain time only: the bulk of
# merkleization runs concurrently with execution and is already reflected in `exec`.
TRACKED_PHASES = ("validate", "exec", "merkle", "store")


def _parse_ts(line):
    match = TS_RE.search(line)
    if not match:
        return None
    text = match.group("ts")
    # Python's fromisoformat rejects more than 6 fractional digits.
    if "." in text:
        head, frac = text.split(".", 1)
        text = f"{head}.{frac[:6]}"
    try:
        return datetime.fromisoformat(text)
    except ValueError:
        return None


def _mean(values):
    return sum(values) / len(values) if values else None


def _median(values):
    if not values:
        return None
    ordered = sorted(values)
    mid = len(ordered) // 2
    if len(ordered) % 2:
        return ordered[mid]
    return (ordered[mid - 1] + ordered[mid]) / 2


def _stdev(values):
    if len(values) < 2:
        return None
    mean = _mean(values)
    return (sum((v - mean) ** 2 for v in values) / len(values)) ** 0.5


def parse_run(lines):
    """Extract the tracked metrics from a run's log lines.

    Returns a dict; every value may be `None` when the log does not contain that signal
    (e.g. a run that died before its first batch), so callers must treat the run as
    invalid rather than assume zeros.
    """
    throughputs = []
    blocks_per_s = []
    last_block = None
    total_gas = 0

    # Per-phase totals, accumulated as (ms, Mgas) so the ratio is gas-normalised.
    phase_ms = {name: 0.0 for name in TRACKED_PHASES}
    block_mgas = 0.0
    pending_mgas = None

    regen_start = None
    regen_end = None

    for line in lines:
        batch = BATCH_RE.search(line)
        if batch:
            throughputs.append(float(batch.group("throughput")))
            last_block = int(batch.group("last_block"))
            total_gas += int(batch.group("gas"))
            continue

        bps = BLOCKS_PER_S_RE.search(line)
        if bps:
            blocks_per_s.append(float(bps.group("bps")))
            continue

        header = BLOCK_HEADER_RE.search(line)
        if header:
            # A new per-block record: the phase lines that follow belong to this block.
            pending_mgas = float(header.group("mgas"))
            block_mgas += pending_mgas
            continue

        phase = PHASE_RE.search(line)
        if phase and pending_mgas is not None:
            phase_ms[phase.group("phase")] += float(phase.group("ms"))
            continue

        if REGEN_START_RE.search(line):
            regen_start = _parse_ts(line)
            continue

        if REGEN_END_RE.search(line):
            regen_end = _parse_ts(line)

    regen_seconds = None
    if regen_start and regen_end and regen_end >= regen_start:
        regen_seconds = (regen_end - regen_start).total_seconds()

    # ms per Mgas: comparable across runs whose blocks carry different amounts of gas.
    phase_ms_per_mgas = None
    if block_mgas > 0:
        phase_ms_per_mgas = {
            name: round(total / block_mgas, 4) for name, total in phase_ms.items()
        }

    return {
        "batches": len(throughputs),
        "throughput_ggas_s": {
            "mean": _mean(throughputs),
            "median": _median(throughputs),
            "stdev": _stdev(throughputs),
            "samples": throughputs,
        },
        "blocks_per_s_mean": _mean(blocks_per_s),
        "last_block": last_block,
        "total_gas": total_gas or None,
        "phase_ms_per_mgas": phase_ms_per_mgas,
        "state_regen_seconds": regen_seconds,
    }


def parse_run_file(path):
    with open(path, "r", errors="replace") as handle:
        return parse_run(handle)
