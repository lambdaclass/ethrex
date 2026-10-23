"""Tests for the runner's chain-tip oracle.

These pin a failure seen on the box: with the execution node stopped between cycles, the
beacon node reports `el_offline` and stops advancing its head. It sat 17 hours behind and
unmoving on all three networks, so a tip read from it made `tip - base` zero forever and
every cycle skipped itself — the watch would have run for weeks producing nothing.

Run: python3 tooling/sync/test_fullsync_bench.py
"""

import json
import os
import sys
import tempfile
import time

sys.argv = ["test"]  # fullsync_bench parses args only under __main__, but be explicit
import fullsync_bench as fb


def _base_at(head, age_seconds, with_created_at=True):
    """A base directory recorded `age_seconds` ago at block `head`."""
    base = tempfile.mkdtemp()
    created = time.time() - age_seconds
    meta = {"head": head}
    if with_created_at:
        meta["created_at"] = created
    path = os.path.join(base, fb.BASE_META)
    with open(path, "w") as fh:
        json.dump(meta, fh)
    if not with_created_at:
        os.utime(path, (created, created))
    return base


def test_tip_advances_while_the_beacon_node_is_frozen():
    # No beacon node is listening on the test port, which is the same situation as one
    # that answers but is stuck: the estimate must come from elapsed time either way.
    day = 24 * 3600
    base = _base_at(head=1_000_000, age_seconds=3 * day)
    tip = fb.chain_tip("mainnet", base)
    blocks = tip - 1_000_000
    # 3 days of 12s slots is 21600 slots; at the 0.97 fill ratio, ~20952 blocks.
    assert 20_000 < blocks < 21_600, blocks


def test_estimate_stays_below_a_full_slot_count():
    # Over-estimating aims a leg at a block that does not exist yet and burns its whole
    # timeout, so the estimate must never exceed one block per slot.
    base = _base_at(head=500, age_seconds=10 * 24 * 3600)
    slots = 10 * 24 * 3600 // fb.SLOT_SECONDS
    assert fb.chain_tip("hoodi", base) - 500 < slots


def test_gap_opens_far_enough_to_release_a_cycle():
    # The practical question: after M days, has enough chain accumulated to measure?
    measure = fb.NETWORKS["mainnet"]["measure_blocks"]
    base = _base_at(head=25_691_647, age_seconds=4 * 24 * 3600)
    assert fb.chain_tip("mainnet", base) - 25_691_647 >= measure


def test_a_fresh_base_releases_nothing():
    measure = fb.NETWORKS["mainnet"]["measure_blocks"]
    base = _base_at(head=25_691_647, age_seconds=60)
    assert fb.chain_tip("mainnet", base) - 25_691_647 < measure


def test_bases_without_created_at_fall_back_to_the_file_mtime():
    # The first bases were written before `created_at` was recorded; they must still work.
    base = _base_at(head=42, age_seconds=3 * 24 * 3600, with_created_at=False)
    assert "created_at" not in json.load(open(os.path.join(base, fb.BASE_META)))
    assert fb.chain_tip("sepolia", base) - 42 > 20_000


def _run_all():
    failures = 0
    for name, fn in sorted(globals().items()):
        if name.startswith("test_") and callable(fn):
            try:
                fn()
                print(f"ok   {name}")
            except AssertionError as exc:
                failures += 1
                print(f"FAIL {name}: {exc}")
    print("all passed" if not failures else f"{failures} failure(s)")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(_run_all())
