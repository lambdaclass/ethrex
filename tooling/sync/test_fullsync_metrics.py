"""Tests for the full-sync metric extraction.

The fixtures below are real lines captured from ethrex full-sync runs on
`ethrex-mainnet-test-1` during the #7008/#7023 benchmarking, not invented samples —
the parser is only useful if it matches what the node actually emits.

Run: python3 -m pytest tooling/sync/test_fullsync_metrics.py
 or: python3 tooling/sync/test_fullsync_metrics.py
"""

from fullsync_metrics import parse_run

BATCH_LINES = [
    "2026-07-28T15:06:49.398765Z  INFO [METRICS] Executed and stored: Range: 1024, Last block num: 25578362, Last block gas limit: 60000000, Total transactions: 460710, Total Gas: 31058623694, Throughput: 0.21295251385695252 Gigagas/s",
    "2026-07-28T15:09:08.113221Z  INFO Executed and stored 1024 blocks in 138.714 seconds (7.382 blocks/s). First block: 25578363 (0x0947…b9cc). Last block: 25579386 (0xe929…f320).",
    "2026-07-28T15:09:08.113900Z  INFO [METRICS] Executed and stored: Range: 1024, Last block num: 25579386, Last block gas limit: 60000000, Total transactions: 373323, Total Gas: 31381203148, Throughput: 0.21821452280452489 Gigagas/s",
    "2026-07-28T15:11:30.552104Z  INFO [METRICS] Executed and stored: Range: 1024, Last block num: 25580410, Last block gas limit: 60000000, Total transactions: 401221, Total Gas: 30939154682, Throughput: 0.2454995472715409 Gigagas/s",
]

# The `(unified pipeline)` wording appeared on the #7008 branch; both must parse.
UNIFIED_LINE = "2026-07-20T20:03:43.225897Z  INFO [METRICS] Executed and stored (unified pipeline): Range: 1024, Last block num: 25530262, Last block gas limit: 60000000, Total transactions: 471224, Total Gas: 30939154682, Throughput: 0.3936213097254789 Gigagas/s"

PER_BLOCK_LINES = [
    "2026-07-23T17:38:20.225580Z  INFO [METRIC] BLOCK 500 0x5f6929cf8b4d0fccf50629ed23704c43819811c5ddbe3bfa0aca83f463afaf2e | 1.750 Ggas/s | 51.41 ms | 2569 txs | 90 Mgas (98%)",
    "2026-07-23T17:38:20.225584Z  INFO   |- validate:    3.50 ms  ( 7%)",
    "2026-07-23T17:38:20.225586Z  INFO   |- exec:       47.41 ms  (92%) << BOTTLENECK",
    "2026-07-23T17:38:20.225588Z  INFO   |- merkle:      0.07 ms  ( 0%)  [concurrent: 37.38 ms, drain: 0.07 ms, overlap: 100%, queue: 2, start_delay: 0.25 ms]",
    "2026-07-23T17:38:20.225591Z  INFO   |- store:       0.43 ms  ( 1%)",
    "2026-07-23T17:38:20.225593Z  INFO   `- warmer:     20.33 ms         [finished: 27.08 ms before exec]",
]

REGEN_LINES = [
    "2026-07-28T15:02:10.100000Z  INFO Regenerating state from block 25577200 to 25577338",
    "2026-07-28T15:02:52.600000Z  INFO Finished regenerating state",
]


def test_batch_throughput_mean_and_samples():
    got = parse_run(BATCH_LINES)
    assert got["batches"] == 3
    assert got["throughput_ggas_s"]["samples"] == [
        0.21295251385695252,
        0.21821452280452489,
        0.2454995472715409,
    ]
    # Mean of the three; the run's headline number.
    assert abs(got["throughput_ggas_s"]["mean"] - 0.22555552797767277) < 1e-12
    assert got["last_block"] == 25580410
    assert got["total_gas"] == 31058623694 + 31381203148 + 30939154682


def test_blocks_per_second_is_captured_separately():
    got = parse_run(BATCH_LINES)
    assert abs(got["blocks_per_s_mean"] - 7.382) < 1e-9


def test_unified_pipeline_wording_also_parses():
    got = parse_run([UNIFIED_LINE])
    assert got["batches"] == 1
    assert abs(got["throughput_ggas_s"]["mean"] - 0.3936213097254789) < 1e-12


def test_phases_are_normalised_per_mgas():
    got = parse_run(PER_BLOCK_LINES)
    phases = got["phase_ms_per_mgas"]
    # One block of 90 Mgas: each phase's ms divided by 90.
    assert abs(phases["exec"] - 47.41 / 90) < 1e-4
    assert abs(phases["validate"] - 3.50 / 90) < 1e-4
    assert abs(phases["merkle"] - 0.07 / 90) < 1e-4
    assert abs(phases["store"] - 0.43 / 90) < 1e-4


def test_phase_lines_without_a_preceding_block_are_ignored():
    # Guards against a truncated log opening mid-record and skewing the ratio.
    got = parse_run(PER_BLOCK_LINES[1:])
    assert got["phase_ms_per_mgas"] is None


def test_state_regeneration_time_is_its_own_signal():
    got = parse_run(REGEN_LINES)
    assert abs(got["state_regen_seconds"] - 42.5) < 1e-6


def test_run_with_no_batches_yields_none_not_zero():
    # A run that died before its first batch must be reported invalid, never as 0 Ggas/s.
    got = parse_run(["2026-07-28T15:00:00.000000Z  INFO Starting full sync cycle"])
    assert got["batches"] == 0
    assert got["throughput_ggas_s"]["mean"] is None
    assert got["last_block"] is None
    assert got["total_gas"] is None
    assert got["state_regen_seconds"] is None


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
