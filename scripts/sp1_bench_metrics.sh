#!/usr/bin/env bash
# Parses a prover log file and enriches batch data from Prometheus metrics.
#
# Usage: ./scripts/sp1_bench_metrics.sh <PROVER_LOG_FILE> [METRICS_URL]
#
# The prover logs lines like:
#   batch=3 proving_time_s=47 proving_time_ms=47123 Proved batch 3 in 47.12s
#
# The script also fetches batch_gas_used, batch_tx_count, and batch_size
# from the L2 metrics endpoint (default localhost:3702/metrics).
#
# Outputs a summary table to stdout and writes a CSV to sp1_bench_results.csv.

set -euo pipefail

if [[ $# -lt 1 ]]; then
    echo "Usage: $0 <prover_log_file> [metrics_url]"
    echo "  Example: ./scripts/sp1_bench_metrics.sh prover.log"
    echo "           ./scripts/sp1_bench_metrics.sh prover.log http://myhost:3702/metrics"
    exit 1
fi

LOG_FILE="$1"
METRICS_URL="${2:-http://localhost:3702/metrics}"
OUTPUT="sp1_bench_results.csv"

echo "batch,proving_time_s,proving_time_ms,gas_used,tx_count,blocks" > "$OUTPUT"

# Fetch a metric value for a given batch from the Prometheus endpoint.
fetch_metric() {
    local metric="$1" batch="$2"
    curl -s "$METRICS_URL" 2>/dev/null \
        | grep "^${metric}{" \
        | grep "batch_number=\"${batch}\"" \
        | awk '{print $2}' \
        | head -1
}

# Parse all proving_time lines from the log file.
declare -A seen
while read -r line; do
    if echo "$line" | grep -q 'proving_time_ms='; then
        batch=$(echo "$line" | grep -o 'batch=[0-9]*' | head -1 | cut -d= -f2)
        secs=$(echo "$line" | grep -o 'proving_time_s=[0-9]*' | cut -d= -f2)
        ms=$(echo "$line" | grep -o 'proving_time_ms=[0-9]*' | cut -d= -f2)

        if [[ -z "$batch" || -z "$ms" ]]; then
            continue
        fi

        if [[ -n "${seen[$batch]:-}" ]]; then
            continue
        fi

        # Fetch batch metadata from Prometheus.
        gas=$(fetch_metric "batch_gas_used" "$batch" 2>/dev/null || true)
        txs=$(fetch_metric "batch_tx_count" "$batch" 2>/dev/null || true)
        blocks=$(fetch_metric "batch_size" "$batch" 2>/dev/null || true)
        gas="${gas:-"-"}"
        txs="${txs:-"-"}"
        blocks="${blocks:-"-"}"

        echo "$batch,$secs,$ms,$gas,$txs,$blocks" >> "$OUTPUT"
        seen["$batch"]=1
    fi
done < "$LOG_FILE"

# Print table.
if [[ $(wc -l < "$OUTPUT") -le 1 ]]; then
    echo "(no batches found in $LOG_FILE)"
    exit 0
fi

echo ""
echo "===== SP1 Proving Benchmark Results ====="
echo ""
printf "%-7s %12s %14s %14s %10s %8s\n" "Batch" "Time (s)" "Time (ms)" "Gas Used" "Tx Count" "Blocks"
printf "%-7s %12s %14s %14s %10s %8s\n" "-----" "--------" "---------" "--------" "--------" "------"
tail -n +2 "$OUTPUT" | sort -t, -k1 -n | while IFS=, read -r batch secs ms gas txs blocks; do
    printf "%-7s %12s %14s %14s %10s %8s\n" "$batch" "$secs" "$ms" "$gas" "$txs" "$blocks"
done
echo ""

# Summary stats.
count=0; total=0; min=999999999; max=0; total_gas=0; total_txs=0
while IFS=, read -r _b _s ms gas txs _blocks; do
    [[ "$_b" == "batch" ]] && continue
    count=$((count + 1))
    total=$((total + ms))
    ((ms < min)) && min=$ms
    ((ms > max)) && max=$ms
    [[ "$gas" != "-" && -n "$gas" ]] && total_gas=$((total_gas + ${gas%%.*}))
    [[ "$txs" != "-" && -n "$txs" ]] && total_txs=$((total_txs + ${txs%%.*}))
done < "$OUTPUT"

if [[ $count -gt 0 ]]; then
    avg=$((total / count))
    echo "Batches: $count | Avg: $((avg/1000))s (${avg}ms) | Min: $((min/1000))s | Max: $((max/1000))s"
    echo "Total gas: $total_gas | Total txs: $total_txs"
fi
echo ""
echo "Results written to $OUTPUT"
