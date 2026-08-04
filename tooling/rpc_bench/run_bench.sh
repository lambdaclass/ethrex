#!/usr/bin/env bash
# Run a vegeta rate-sweep over every target file produced by gen_workload.py.
#
# Usage: run_bench.sh <targets_dir> <out_dir> [rates] [duration]
#   targets_dir  directory of <method>.json vegeta target files
#   out_dir      directory to write <method>__<rate>.json vegeta reports into
#   rates        space-separated request rates (default: "10 100 500 1000")
#   duration     per-rate attack duration (default: 30s)
#
# Each report is vegeta's JSON summary; feed the out_dir to summarize.py.
set -euo pipefail

TGT=${1:?usage: run_bench.sh <targets_dir> <out_dir> [rates] [duration]}
OUT=${2:?usage: run_bench.sh <targets_dir> <out_dir> [rates] [duration]}
RATES=${3:-"10 100 500 1000"}
DUR=${4:-30s}

mkdir -p "$OUT"
echo "run: targets=$TGT out=$OUT rates=[$RATES] duration=$DUR"
for f in "$TGT"/*.json; do
  [ -e "$f" ] || continue
  m=$(basename "$f" .json)
  for r in $RATES; do
    echo ">>> $m @ ${r}rps"
    vegeta attack -format=json -targets="$f" -rate="$r/1s" -duration="$DUR" \
      -timeout=30s -max-workers=512 \
      | vegeta report -type=json > "$OUT/${m}__${r}.json"
  done
done
echo "done -> $OUT"
