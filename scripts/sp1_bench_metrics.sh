#!/usr/bin/env bash
# Tails prover log file and enriches batch data from Prometheus metrics.
#
# Usage: ./scripts/sp1_bench_metrics.sh <PROVER_LOG_FILE> [METRICS_URL]
#
# The prover logs lines like:
#   batch=3 proving_time_s=47 proving_time_ms=47123 Proved batch 3 in 47.12s
#
# The script also polls the L2 metrics endpoint (default localhost:3702/metrics)
# to fetch batch_gas_used, batch_tx_count, and batch_size per batch.
#
# On Ctrl+C it prints a full summary table and writes to CSV.

set -euo pipefail

if [[ $# -lt 1 ]]; then
    echo "Usage: $0 <prover_log_file> [metrics_url]"
    echo "  Redirect prover output to a file, then point this script at it."
    echo "  Example: make init-prover-sp1 2>&1 | tee prover.log"
    echo "           ./scripts/sp1_bench_metrics.sh prover.log"
    echo "           ./scripts/sp1_bench_metrics.sh prover.log http://localhost:3702/metrics"
    exit 1
fi

LOG_FILE="$1"
METRICS_URL="${2:-http://localhost:3702/metrics}"
OUTPUT="sp1_bench_results.csv"

if [[ ! -f "$OUTPUT" ]]; then
    echo "batch,proving_time_s,proving_time_ms,gas_used,tx_count,blocks" > "$OUTPUT"
fi

# Load already-recorded batches
declare -A seen
while IFS=, read -r batch _rest; do
    [[ "$batch" == "batch" ]] && continue
    seen["$batch"]=1
done < "$OUTPUT"

# Fetch a metric value for a given batch from the Prometheus endpoint.
# Usage: fetch_metric <metric_name> <batch_number>
fetch_metric() {
    local metric="$1" batch="$2"
    curl -s "$METRICS_URL" 2>/dev/null \
        | grep "^${metric}{" \
        | grep "batch_number=\"${batch}\"" \
        | awk '{print $2}' \
        | head -1
}

# Fetch all batch metrics from Prometheus and enrich the CSV rows that
# are missing them (gas_used, tx_count, blocks still empty).
enrich_from_prometheus() {
    local tmpfile
    tmpfile=$(mktemp)
    head -1 "$OUTPUT" > "$tmpfile"

    while IFS=, read -r batch secs ms gas txs blocks; do
        [[ "$batch" == "batch" ]] && continue
        if [[ -z "$gas" || "$gas" == "-" ]]; then
            gas=$(fetch_metric "batch_gas_used" "$batch")
            gas="${gas:-"-"}"
        fi
        if [[ -z "$txs" || "$txs" == "-" ]]; then
            txs=$(fetch_metric "batch_tx_count" "$batch")
            txs="${txs:-"-"}"
        fi
        if [[ -z "$blocks" || "$blocks" == "-" ]]; then
            blocks=$(fetch_metric "batch_size" "$batch")
            blocks="${blocks:-"-"}"
        fi
        echo "$batch,$secs,$ms,$gas,$txs,$blocks" >> "$tmpfile"
    done < "$OUTPUT"

    mv "$tmpfile" "$OUTPUT"
}

echo "Watching $LOG_FILE — writing to $OUTPUT"
echo "Metrics endpoint: $METRICS_URL"
echo "Press Ctrl+C to stop and print summary."
echo ""

print_table() {
    # Enrich any rows that are missing Prometheus data before printing.
    enrich_from_prometheus 2>/dev/null || true

    if [[ ! -f "$OUTPUT" ]] || [[ $(wc -l < "$OUTPUT") -le 1 ]]; then
        echo "(no batches recorded yet)"
        return
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

    # Summary stats (proving time)
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
}

trap print_table EXIT

tail -n 0 -f "$LOG_FILE" | while read -r line; do
    # Match lines containing "proving_time_ms="
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

        # Try to fetch Prometheus data immediately.
        gas=$(fetch_metric "batch_gas_used" "$batch" 2>/dev/null || echo "-")
        txs=$(fetch_metric "batch_tx_count" "$batch" 2>/dev/null || echo "-")
        blocks=$(fetch_metric "batch_size" "$batch" 2>/dev/null || echo "-")
        gas="${gas:-"-"}"
        txs="${txs:-"-"}"
        blocks="${blocks:-"-"}"

        echo "$batch,$secs,$ms,$gas,$txs,$blocks" >> "$OUTPUT"
        seen["$batch"]=1
        printf "[Batch %s] proving=%ss  gas=%s  txs=%s  blocks=%s\n" "$batch" "$secs" "$gas" "$txs" "$blocks"
    fi
done
