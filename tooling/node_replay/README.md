# node-replay

`node-replay` is an agent-friendly replay tool for executing Ethereum blocks with the same execution logic used by `ethrex`.

It is designed to:

- checkpoint a live `ethrex` RocksDB datadir,
- plan deterministic replay ranges pinned to canonical hashes,
- execute replay runs in an isolated per-run DB copy,
- expose machine-consumable JSON responses and typed error codes.

## High-level flow

1. Create checkpoint
- Open the live datadir read-only.
- Load chain config and initial state cache.
- Select an executable anchor block (see anchor selection below).
- Create a RocksDB checkpoint under the workspace (`checkpoints/<id>/db`).

2. Plan replay
- Read checkpoint metadata.
- Resolve canonical hashes for `[anchor + 1, anchor + blocks]` from live datadir.
- Verify parent-hash continuity to detect reorgs.
- Persist `run_manifest.json` and initial `status.json`.

3. Run replay
- Acquire per-run lock and start lock heartbeat.
- Create per-run DB copy from checkpoint base (`runs/<run_id>/db`).
- Load chain config and initial state cache in the run store.
- Fetch blocks from live datadir and execute them against run DB.
- Persist progress/events and write final summary.

## Architecture diagrams

### Component/data-flow diagram

```text
                        +---------------------------+
                        |        Live ethrex        |
                        |     RocksDB datadir       |
                        +-------------+-------------+
                                      ^
                                      | read-only (hashes, headers, blocks)
                                      |
 +---------------------------+        |        +---------------------------+
 |        node-replay        +--------+------->+  plan/run live store RO  |
 |      (CLI + engine)       |                 +---------------------------+
 +-------------+-------------+
               |
               | create_checkpoint (hard-linked copy)
               v
 +---------------------------+          +----------------------------------+
 | Workspace checkpoints DB  |          | Workspace run DB (per-run copy)  |
 | checkpoints/<id>/db       |----------> runs/<run_id>/db                 |
 +---------------------------+          +-------------------+--------------+
                                                           |
                                                           | execute blocks
                                                           v
                                                +---------------------------+
                                                | ethrex_blockchain pipeline|
                                                | (same execution logic)    |
                                                +---------------------------+
```

### Run lifecycle diagram

```text
checkpoint create
  -> select executable anchor
  -> write checkpoints/<id>/checkpoint.json
  -> write checkpoints/<id>/db

plan
  -> read checkpoint meta
  -> pin canonical hashes
  -> write runs/<run_id>/run_manifest.json
  -> write runs/<run_id>/status.json (planned)
  -> append run_planned event

run
  -> acquire run lock
  -> start heartbeat
  -> materialize runs/<run_id>/db from checkpoint db
  -> execute pinned blocks
  -> append block_started/block_executed events
  -> update status progress
  -> write summary + run_completed event
  -> release lock
```

## Safety model

- Live datadir is opened read-only for checkpointing/planning/block fetch.
- Replay writes only to `runs/<run_id>/db`.
- Checkpoint base DB is never executed in place.
- Path isolation guard prevents run DB from colliding with live/checkpoint paths.
- File lock + heartbeat prevents concurrent executors for the same run.

## Workspace layout

Given `--workspace /path/to/ws`:

```text
/path/to/ws/
  checkpoints/
    <checkpoint_id>/
      checkpoint.json
      db/
  runs/
    <run_id>/
      run_manifest.json
      status.json
      summary.json
      events.ndjson
      cancel.flag                # optional
      locks/
        run.lock
      logs/
        replay.log               # reserved path
      db/
```

## Anchor selection behavior

Checkpoint creation does not blindly trust "latest" as replay anchor.

It:

- starts from latest persisted block number,
- backtracks up to `16,384` blocks,
- picks the first canonical block whose header exists and whose state root is present in DB.

This avoids anchors that cannot be executed due to missing historical state roots in path-based/pruned environments.

## CLI usage

All commands return JSON.

Global form:

```bash
node-replay --workspace <WORKSPACE> <command> [args]
```

### checkpoint create

```bash
node-replay --workspace "$WS" checkpoint create \
  --datadir /home/admin/.local/share/ethrex \
  --label mainnet-snap-001
```

Notes:

- Idempotent by `(label, datadir)`: same pair returns existing checkpoint.
- Use unique labels if you want multiple checkpoints for the same datadir.

### checkpoint list

```bash
node-replay --workspace "$WS" checkpoint list
```

### plan

```bash
node-replay --workspace "$WS" plan \
  --checkpoint "$CHECKPOINT_ID" \
  --blocks 100 \
  --datadir /home/admin/.local/share/ethrex
```

### run

```bash
node-replay --workspace "$WS" run \
  --manifest "$WS/runs/$RUN_ID/run_manifest.json" \
  --mode isolated
```

### status

```bash
node-replay --workspace "$WS" status --run "$RUN_ID"
```

### resume

```bash
node-replay --workspace "$WS" resume --run "$RUN_ID"
```

### cancel

```bash
node-replay --workspace "$WS" cancel --run "$RUN_ID"
```

### verify

```bash
node-replay --workspace "$WS" verify --run "$RUN_ID"
```

### report

```bash
node-replay --workspace "$WS" report --run "$RUN_ID"
```

## Examples

### Example 1: End-to-end 1-block smoke replay

```bash
BIN=target/release/node-replay
DATADIR=/home/admin/.local/share/ethrex
TS=$(date -u +%Y%m%dT%H%M%SZ)
WS=/tmp/node-replay-$TS
mkdir -p "$WS/checkpoints" "$WS/runs"

$BIN --workspace "$WS" checkpoint create --datadir "$DATADIR" --label "smoke-$TS" > /tmp/ckpt.json
CKPT=$(jq -r '.data.checkpoint_id' /tmp/ckpt.json)

$BIN --workspace "$WS" plan --checkpoint "$CKPT" --blocks 1 --datadir "$DATADIR" > /tmp/plan.json
RUN_ID=$(jq -r '.data.run_id' /tmp/plan.json)

$BIN --workspace "$WS" run --manifest "$WS/runs/$RUN_ID/run_manifest.json" --mode isolated > /tmp/run.json
$BIN --workspace "$WS" status --run "$RUN_ID" > /tmp/status.json
$BIN --workspace "$WS" verify --run "$RUN_ID" > /tmp/verify.json
$BIN --workspace "$WS" report --run "$RUN_ID" > /tmp/report.json
```

Quick check:

```bash
jq -r '.success, .data.state, .data.executed_blocks, .data.total_blocks' /tmp/run.json
jq -r '.success, .data.state, .data.last_completed_block' /tmp/status.json
jq -r '.success' /tmp/verify.json
```

### Example 2: Plan with retry until the next block is available

```bash
PLAN_OK=0
for i in 1 2 3 4 5 6 7 8 9 10; do
  $BIN --workspace "$WS" plan --checkpoint "$CKPT" --blocks 1 --datadir "$DATADIR" > /tmp/plan.json || true
  if [ "$(jq -r '.success' /tmp/plan.json)" = "true" ]; then
    PLAN_OK=1
    break
  fi
  sleep 3
done
test "$PLAN_OK" = "1"
```

### Example 3: Multiple checkpoints from the same datadir

```bash
# Different labels -> distinct checkpoints
$BIN --workspace "$WS" checkpoint create --datadir "$DATADIR" --label "snap-a"
$BIN --workspace "$WS" checkpoint create --datadir "$DATADIR" --label "snap-b"
$BIN --workspace "$WS" checkpoint list
```

Idempotency rule:

- same `(label, datadir)` returns the existing checkpoint,
- new label creates a new checkpoint.

### Example 4: Resume a failed/paused run

```bash
$BIN --workspace "$WS" resume --run "$RUN_ID"
$BIN --workspace "$WS" run --manifest "$WS/runs/$RUN_ID/run_manifest.json" --mode isolated
```

### Example 5: Cooperative cancellation

```bash
$BIN --workspace "$WS" cancel --run "$RUN_ID"
$BIN --workspace "$WS" status --run "$RUN_ID"
```

For actively running executors, `cancel` drops `cancel.flag`; executor detects it and finalizes cancellation.

## Per-block metrics

Each `block_executed` event in `runs/<run_id>/events.ndjson` includes
`payload.metrics` captured directly from `add_block_pipeline` timing data.

Example fields:

- `total_ms`
- `validate_ms`
- `exec_ms`
- `merkle_concurrent_ms`
- `merkle_drain_ms`
- `merkle_total_ms`
- `store_ms`
- `warmer_ms`
- `warmer_early_ms`
- `merkle_overlap_pct`
- `merkle_queue_length`
- `throughput_ggas_per_s`
- `bottleneck_phase`

Query example:

```bash
jq -s '
  [ .[] | select(.event=="block_executed") | {
      block_number,
      block_hash,
      total_ms: .payload.metrics.total_ms,
      validate_ms: .payload.metrics.validate_ms,
      exec_ms: .payload.metrics.exec_ms,
      merkle_drain_ms: .payload.metrics.merkle_drain_ms,
      store_ms: .payload.metrics.store_ms,
      bottleneck_phase: .payload.metrics.bottleneck_phase
    } ]
' "$WS/runs/$RUN_ID/events.ndjson"
```

## CPU profiling

Recommended build flags for CPU profiling:

```bash
cargo build -p node-replay --config .cargo/profiling.toml --profile release-with-debug
```

This enables frame pointers (`.cargo/profiling.toml`) and keeps debug symbols
(`release-with-debug`) for useful call stacks in `perf`.

### One-command profiling workflow

Use:

```bash
tooling/node_replay/scripts/profile_replay_range.sh \
  --workspace "$WS" \
  --datadir "$DATADIR" \
  --checkpoint "$CHECKPOINT_ID" \
  --blocks 10 \
  --out-dir /tmp/node-replay-profile-mainnet
```

What it does:

- builds `node-replay` with profiling flags,
- runs a `perf stat` pass on one replay run,
- runs a `perf record` pass on another replay run with the same range,
- writes profiling artifacts and per-block metrics tables.

Artifacts written under `--out-dir`:

- `perf_stat.csv`
- `perf.data`
- `perf_report.txt`
- `plan_stat.json`, `run_stat.json`, `status_stat.json`, `verify_stat.json`
- `plan_record.json`, `run_record.json`, `status_record.json`, `verify_record.json`
- `block_metrics_stat.tsv`, `block_metrics_record.tsv`
- `meta_stat.json`, `meta_record.json`, `profile_session.json`

### Manual perf commands

If you only want the perf tools:

```bash
perf stat -d -d -d -- \
  target/release-with-debug/node-replay --workspace "$WS" run \
    --manifest "$WS/runs/$RUN_ID/run_manifest.json" --mode isolated

perf record -F 999 -g --call-graph fp -o /tmp/node-replay.perf -- \
  target/release-with-debug/node-replay --workspace "$WS" run \
    --manifest "$WS/runs/$RUN_ID/run_manifest.json" --mode isolated

perf report --stdio -i /tmp/node-replay.perf --sort comm,dso,symbol
```

## Agent-facing contracts

### Response envelope

Success:

```json
{
  "success": true,
  "data": { ... },
  "request_id": "optional"
}
```

Error:

```json
{
  "success": false,
  "error": {
    "code": "typed/error_code",
    "message": "human-readable context"
  },
  "request_id": "optional"
}
```

### Error codes and exit codes

- Input: `input/*` -> exit `10`
- State/lock conflicts: `state/*`, `conflict/*`, `lock/*` -> exit `20`
- Chain consistency: `chain/*` -> exit `30`
- Execution/storage runtime: `execution/*`, `storage/*` -> exit `40`
- Verification mismatch: `verify/*` -> exit `50`
- Internal/IO/JSON: `internal/*` -> exit `70`

## Concurrency and locking

- Lock file path: `runs/<run_id>/locks/run.lock`
- Stale lock threshold: `3600s`
- Heartbeat interval: `30s`
- Heartbeat refresh verifies lock ownership (`pid + run_id`).

If a running executor dies, `resume` can recover stale-running runs after stale lock cleanup.

## File descriptor handling

Large RocksDB datasets can require high FD limits.

Current behavior:

- `node-replay` raises process soft `RLIMIT_NOFILE` to hard limit on startup (best effort, Unix).
- Read-only RocksDB opens are configured with bounded `max_open_files` to avoid EMFILE in live sidecar usage.

## Current limitations

- `--mode stop-live-node` is currently accepted but execution path is equivalent to isolated mode.
- `--finality` is currently accepted in planning API, but block selection is currently canonical-head based.
- `verify` validates completion/event consistency; it does not yet recompute state roots/receipts roots.
- Replay requires the target block bodies to be present in the live datadir.

## Troubleshooting

`storage/checkpoint_failed: failed to open store`
- Datadir path wrong, schema mismatch, or RocksDB open problem.
- Check datadir exists and belongs to compatible `ethrex` storage schema.

`storage/checkpoint_failed: Too many open files`
- Host soft FD limit too low.
- Confirm `ulimit -Sn` and increase if process limits prevent automatic raise.

`chain/reorg_detected`
- Planned parent chain does not match pinned anchor/hash continuity.
- Recreate checkpoint and re-plan.

`input/invalid_argument: block N not found in canonical chain`
- Node has not reached that block yet, or canonical hash for range not available.
- Wait for node progress and retry `plan`.

`execution/block_failed: block ... not found in live store`
- Block hash planned, but full block body unavailable in live datadir.

`execution/block_failed: ... state root missing`
- Anchor/range cannot be executed with available historical state.
- Create a fresh checkpoint after node state catches up and re-plan.

## Development and tests

Build:

```bash
cargo build --release -p node-replay
```

Checks:

```bash
cargo check -p node-replay
cargo test -p node-replay
```

Safety tests in `tooling/node_replay/tests/safety.rs` validate:

- live/checkpoint DB immutability guarantees,
- path isolation conflicts,
- lock conflict behavior,
- idempotent checkpoint lookup behavior.
