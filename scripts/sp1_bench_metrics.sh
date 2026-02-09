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
# Outputs a markdown file (sp1_bench_results.md) with a results table and summary.

set -euo pipefail

if [[ $# -lt 1 ]]; then
    echo "Usage: $0 <prover_log_file> [metrics_url]"
    echo "  Example: ./scripts/sp1_bench_metrics.sh prover.log"
    echo "           ./scripts/sp1_bench_metrics.sh prover.log http://myhost:3702/metrics"
    exit 1
fi

LOG_FILE="$1"
METRICS_URL="${2:-http://localhost:3702/metrics}"
OUTPUT="sp1_bench_results.md"

# Fetch a metric value for a given batch from the Prometheus endpoint.
fetch_metric() {
    local metric="$1" batch="$2"
    curl -s "$METRICS_URL" 2>/dev/null \
        | grep "^${metric}{" \
        | grep "batch_number=\"${batch}\"" \
        | awk '{print $2}' \
        | head -1
}

# Collect rows into arrays.
declare -A seen
batches=()
secs_arr=()
ms_arr=()
gas_arr=()
txs_arr=()
blocks_arr=()

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

        batches+=("$batch")
        secs_arr+=("${secs:-"-"}")
        ms_arr+=("$ms")
        gas_arr+=("${gas:-"-"}")
        txs_arr+=("${txs:-"-"}")
        blocks_arr+=("${blocks:-"-"}")
        seen["$batch"]=1
    fi
done < "$LOG_FILE"

if [[ ${#batches[@]} -eq 0 ]]; then
    echo "(no batches found in $LOG_FILE)"
    exit 0
fi

# Detect hardware specs.
detect_cpu() {
    if [[ -f /proc/cpuinfo ]]; then
        grep -m1 'model name' /proc/cpuinfo | cut -d: -f2 | xargs
    elif command -v sysctl &>/dev/null; then
        sysctl -n machdep.cpu.brand_string 2>/dev/null
    fi
}

detect_ram() {
    if [[ -f /proc/meminfo ]]; then
        local kb
        kb=$(grep MemTotal /proc/meminfo | awk '{print $2}')
        echo "$((kb / 1024 / 1024)) GB"
    elif command -v sysctl &>/dev/null; then
        local bytes
        bytes=$(sysctl -n hw.memsize 2>/dev/null)
        echo "$((bytes / 1024 / 1024 / 1024)) GB"
    fi
}

detect_gpu() {
    if command -v nvidia-smi &>/dev/null; then
        nvidia-smi --query-gpu=name,memory.total --format=csv,noheader,nounits 2>/dev/null \
            | head -1 \
            | awk -F', ' '{printf "%s %d GB", $1, $2/1024}'
    fi
}

cpu=$(detect_cpu)
ram=$(detect_ram)
gpu=$(detect_gpu)

# Write markdown.
{
    echo "# Proving Benchmark Results"
    echo ""
    if [[ -n "$cpu" || -n "$ram" || -n "$gpu" ]]; then
        echo "## Server Specs"
        echo ""
        [[ -n "$cpu" ]] && echo "- $cpu"
        [[ -n "$ram" ]] && echo "- $ram RAM"
        [[ -n "$gpu" ]] && echo "- $gpu"
        echo ""
    fi
    echo "| Batch | Time (s) | Time (ms) | Gas Used | Tx Count | Blocks |"
    echo "|-------|----------|-----------|----------|----------|--------|"
    for i in "${!batches[@]}"; do
        echo "| ${batches[$i]} | ${secs_arr[$i]} | ${ms_arr[$i]} | ${gas_arr[$i]} | ${txs_arr[$i]} | ${blocks_arr[$i]} |"
    done

    # Summary stats.
    count=0; total=0; min=999999999; max=0; total_gas=0; total_txs=0
    for i in "${!batches[@]}"; do
        ms=${ms_arr[$i]}
        gas=${gas_arr[$i]}
        txs=${txs_arr[$i]}
        count=$((count + 1))
        total=$((total + ms))
        ((ms < min)) && min=$ms
        ((ms > max)) && max=$ms
        [[ "$gas" != "-" && -n "$gas" ]] && total_gas=$((total_gas + ${gas%%.*}))
        [[ "$txs" != "-" && -n "$txs" ]] && total_txs=$((total_txs + ${txs%%.*}))
    done

    if [[ $count -gt 0 ]]; then
        avg=$((total / count))
        echo ""
        echo "## Summary"
        echo ""
        echo "| Metric | Value |"
        echo "|--------|-------|"
        echo "| Batches | $count |"
        echo "| Avg | $((avg/1000))s (${avg}ms) |"
        echo "| Min | $((min/1000))s (${min}ms) |"
        echo "| Max | $((max/1000))s (${max}ms) |"
        echo "| Total gas | $total_gas |"
        echo "| Total txs | $total_txs |"
    fi
} > "$OUTPUT"

cat "$OUTPUT"
echo ""
echo "Results written to $OUTPUT"
