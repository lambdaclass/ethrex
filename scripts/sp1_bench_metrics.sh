#!/usr/bin/env bash
# Tails prover log file and builds a proving-time table.
# Usage: ./scripts/sp1_bench_metrics.sh <PROVER_LOG_FILE>
#
# The prover logs lines like:
#   batch=3 proving_time_s=47 proving_time_ms=47123 Proved batch 3 in 47.12s
#
# This script watches for those lines, records them to a CSV, and prints
# a summary table on Ctrl+C.

set -euo pipefail

if [[ $# -lt 1 ]]; then
    echo "Usage: $0 <prover_log_file>"
    echo "  Redirect prover output to a file, then point this script at it."
    echo "  Example: make init-prover-sp1 2>&1 | tee prover.log"
    echo "           ./scripts/sp1_bench_metrics.sh prover.log"
    exit 1
fi

LOG_FILE="$1"
OUTPUT="sp1_bench_results.csv"

if [[ ! -f "$OUTPUT" ]]; then
    echo "batch,proving_time_s,proving_time_ms" > "$OUTPUT"
fi

# Load already-recorded batches
declare -A seen
while IFS=, read -r batch _rest; do
    [[ "$batch" == "batch" ]] && continue
    seen["$batch"]=1
done < "$OUTPUT"

echo "Watching $LOG_FILE — writing to $OUTPUT"
echo "Press Ctrl+C to stop and print summary."
echo ""

print_table() {
    if [[ ! -f "$OUTPUT" ]] || [[ $(wc -l < "$OUTPUT") -le 1 ]]; then
        echo "(no batches recorded yet)"
        return
    fi
    echo ""
    echo "===== SP1 Proving Benchmark Results ====="
    echo ""
    printf "%-7s %-16s %-16s\n" "Batch" "Time (s)" "Time (ms)"
    printf "%-7s %-16s %-16s\n" "-----" "--------" "---------"
    tail -n +2 "$OUTPUT" | sort -t, -k1 -n | while IFS=, read -r batch secs ms; do
        printf "%-7s %-16s %-16s\n" "$batch" "$secs" "$ms"
    done
    echo ""

    # Summary stats
    count=0; total=0; min=999999999; max=0
    while IFS=, read -r _b _s ms; do
        [[ "$_b" == "batch" ]] && continue
        count=$((count + 1))
        total=$((total + ms))
        ((ms < min)) && min=$ms
        ((ms > max)) && max=$ms
    done < "$OUTPUT"

    if [[ $count -gt 0 ]]; then
        avg=$((total / count))
        echo "Batches: $count | Avg: $((avg/1000))s (${avg}ms) | Min: $((min/1000))s (${min}ms) | Max: $((max/1000))s (${max}ms)"
    fi
    echo ""
}

trap print_table EXIT

tail -n 0 -f "$LOG_FILE" | while read -r line; do
    # Match lines containing both "batch=" and "proving_time_ms="
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

        echo "$batch,$secs,$ms" >> "$OUTPUT"
        seen["$batch"]=1
        printf "[Batch %s] proving_time=%ss (%sms)\n" "$batch" "$secs" "$ms"
    fi
done
