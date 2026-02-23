#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Profile node-replay execution for a fixed replay range.

Usage:
  tooling/node_replay/scripts/profile_replay_range.sh \
    --workspace <workspace> \
    --datadir <live_datadir> \
    --checkpoint <checkpoint_id> \
    [--blocks <N>] \
    [--out-dir <dir>] \
    [--sample-freq <hz>] \
    [--skip-build]

Notes:
  - Builds with: --config .cargo/profiling.toml --profile release-with-debug
  - Runs two profiling passes on the same block range:
      1) perf stat
      2) perf record (+ perf report)
  - Writes artifacts under --out-dir (default: /tmp/node-replay-profile-<ts>)
EOF
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

WORKSPACE=""
DATADIR=""
CHECKPOINT_ID=""
BLOCKS=10
OUT_DIR=""
SAMPLE_FREQ=999
SKIP_BUILD=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --workspace)
      WORKSPACE="$2"
      shift 2
      ;;
    --datadir)
      DATADIR="$2"
      shift 2
      ;;
    --checkpoint)
      CHECKPOINT_ID="$2"
      shift 2
      ;;
    --blocks)
      BLOCKS="$2"
      shift 2
      ;;
    --out-dir)
      OUT_DIR="$2"
      shift 2
      ;;
    --sample-freq)
      SAMPLE_FREQ="$2"
      shift 2
      ;;
    --skip-build)
      SKIP_BUILD=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage
      exit 2
      ;;
  esac
done

if [[ -z "$WORKSPACE" || -z "$DATADIR" || -z "$CHECKPOINT_ID" ]]; then
  echo "Missing required arguments." >&2
  usage
  exit 2
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required" >&2
  exit 2
fi
if ! command -v perf >/dev/null 2>&1; then
  echo "perf is required" >&2
  exit 2
fi

TS="$(date -u +%Y%m%dT%H%M%SZ)"
OUT_DIR="${OUT_DIR:-/tmp/node-replay-profile-${TS}}"
mkdir -p "$OUT_DIR"

BIN="$REPO_ROOT/target/release-with-debug/node-replay"

if [[ "$SKIP_BUILD" -eq 0 ]]; then
  echo "[profile] building node-replay with frame pointers + debug symbols..."
  (
    cd "$REPO_ROOT"
    cargo build -p node-replay --config .cargo/profiling.toml --profile release-with-debug
  )
fi

if [[ ! -x "$BIN" ]]; then
  echo "node-replay binary not found: $BIN" >&2
  exit 2
fi

run_pass() {
  local pass="$1"  # stat or record
  local plan_json="$OUT_DIR/plan_${pass}.json"
  local run_json="$OUT_DIR/run_${pass}.json"
  local status_json="$OUT_DIR/status_${pass}.json"
  local verify_json="$OUT_DIR/verify_${pass}.json"
  local metrics_tsv="$OUT_DIR/block_metrics_${pass}.tsv"

  "$BIN" --workspace "$WORKSPACE" plan \
    --checkpoint "$CHECKPOINT_ID" \
    --blocks "$BLOCKS" \
    --datadir "$DATADIR" >"$plan_json"

  local run_id
  run_id="$(jq -r '.data.run_id' "$plan_json")"
  local start_block
  start_block="$(jq -r '.data.start_number' "$plan_json")"
  local end_block
  end_block="$(jq -r '.data.end_number' "$plan_json")"

  local manifest_path="$WORKSPACE/runs/$run_id/run_manifest.json"

  if [[ "$pass" == "stat" ]]; then
    perf stat -d -d -d -x, -o "$OUT_DIR/perf_stat.csv" -- \
      "$BIN" --workspace "$WORKSPACE" run --manifest "$manifest_path" --mode isolated >"$run_json"
  else
    perf record -F "$SAMPLE_FREQ" -g --call-graph fp -o "$OUT_DIR/perf.data" -- \
      "$BIN" --workspace "$WORKSPACE" run --manifest "$manifest_path" --mode isolated >"$run_json"
    perf report --stdio -i "$OUT_DIR/perf.data" --sort comm,dso,symbol --percent-limit 0.50 \
      >"$OUT_DIR/perf_report.txt"
  fi

  "$BIN" --workspace "$WORKSPACE" status --run "$run_id" >"$status_json"
  "$BIN" --workspace "$WORKSPACE" verify --run "$run_id" >"$verify_json"

  jq -s -r '
    [ .[] | select(.event=="block_executed") | {
        block_number,
        block_hash,
        total_ms: .payload.metrics.total_ms,
        validate_ms: .payload.metrics.validate_ms,
        exec_ms: .payload.metrics.exec_ms,
        merkle_drain_ms: .payload.metrics.merkle_drain_ms,
        store_ms: .payload.metrics.store_ms,
        warmer_ms: .payload.metrics.warmer_ms,
        throughput_ggas_per_s: .payload.metrics.throughput_ggas_per_s,
        bottleneck_phase: .payload.metrics.bottleneck_phase
      } ]
    | sort_by(.block_number)
    | (["block_number","block_hash","total_ms","validate_ms","exec_ms","merkle_drain_ms","store_ms","warmer_ms","throughput_ggas_per_s","bottleneck_phase"] | @tsv),
      (.[] | [ .block_number, .block_hash, .total_ms, .validate_ms, .exec_ms, .merkle_drain_ms, .store_ms, .warmer_ms, .throughput_ggas_per_s, .bottleneck_phase ] | @tsv)
  ' "$WORKSPACE/runs/$run_id/events.ndjson" >"$metrics_tsv"

  jq -n \
    --arg pass "$pass" \
    --arg run_id "$run_id" \
    --argjson start "$start_block" \
    --argjson end "$end_block" \
    '{pass:$pass, run_id:$run_id, start_block:$start, end_block:$end}' \
    >"$OUT_DIR/meta_${pass}.json"
}

run_pass "stat"
run_pass "record"

jq -n \
  --arg workspace "$WORKSPACE" \
  --arg datadir "$DATADIR" \
  --arg checkpoint_id "$CHECKPOINT_ID" \
  --argjson blocks "$BLOCKS" \
  --arg out_dir "$OUT_DIR" \
  --arg bin "$BIN" \
  --arg ts "$TS" \
  '{
    timestamp_utc: $ts,
    workspace: $workspace,
    datadir: $datadir,
    checkpoint_id: $checkpoint_id,
    blocks: $blocks,
    binary: $bin,
    artifacts_dir: $out_dir
  }' >"$OUT_DIR/profile_session.json"

echo "[profile] done"
echo "[profile] artifacts: $OUT_DIR"
echo "[profile] perf stat: $OUT_DIR/perf_stat.csv"
echo "[profile] perf record: $OUT_DIR/perf.data"
echo "[profile] perf report: $OUT_DIR/perf_report.txt"
