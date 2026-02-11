# Execution Witness Benchmark Testing Workflow

## Overview

An **execution witness** contains all the initial state values (state nodes, codes, storage keys, block headers) needed to execute a block without access to the full Ethereum state. This is essential for zkVM provers, as the entire state wouldn't fit in a zkVM program.

The `debug_executionWitness` RPC endpoint returns the execution witness for a given block. For more details on execution witnesses and their role in proving, see the [Execution Witness documentation](../l2/fundamentals/execution_witness.md).

## Purpose

Testing `debug_executionWitness` latency improvements by running two servers in parallel:
- **Baseline server**: main branch with `--precompute-witnesses` flag
- **Test server**: PR branch with optimization and `--precompute-witnesses` flag

This workflow benchmarks any PR that aims to improve execution witness generation or retrieval performance.

## Agent Setup Instructions

**Before starting any test session, the agent MUST prompt the user for:**

1. **Baseline server hostname** - SSH alias or full hostname for the main branch server
2. **Test server hostname** - SSH alias or full hostname for the PR branch server
3. **Network** - Ethereum network to sync (e.g., hoodi, sepolia, mainnet)
4. **PR number/branch** - The PR or branch being tested
5. **Servers already synced?** - Skip sync steps if servers are already synced and running

Example prompt:
```
Before starting the execution witness benchmark, I need the following information:
- Baseline server hostname (running main branch):
- Test server hostname (running PR branch):
- Network to sync:
- PR number or branch being tested:
- Are the servers already synced with --precompute-witnesses enabled? (yes/no)
```

## Prerequisites

- SSH access to both servers
- Lighthouse installed (available in PATH)
- JWT secret at `~/secrets/jwt.hex`
- ethrex repository cloned at `~/ethrex`
- ethrex-replay repository cloned at `~/ethrex-replay`

## Commands

### Lighthouse (Consensus Client)

```bash
lighthouse bn --network <NETWORK> --execution-endpoint http://localhost:8551 --execution-jwt ~/secrets/jwt.hex --http --checkpoint-sync-url <CHECKPOINT_SYNC_URL>
```

**Checkpoint Sync URLs by Network:**
| Network | Checkpoint Sync URL |
|---------|---------------------|
| hoodi | https://hoodi-checkpoint-sync.stakely.io |
| sepolia | https://sepolia.checkpoint-sync.ethpandaops.io |
| mainnet | https://mainnet.checkpoint.sigp.io |

**Notes:**
- Will fail initially if ethrex is not running (expected behavior)
- **NEVER clear Lighthouse DB without explicit user approval** - ask first with full context
- To clear DB (only with approval): add `--purge-db` for interactive confirmation or `--purge-db-force` to skip confirmation

### Ethrex (Execution Client)

```bash
cargo run --release --bin ethrex -- --http.addr 0.0.0.0 --network <NETWORK> --authrpc.jwtsecret ~/secrets/jwt.hex --precompute-witnesses
```

**Critical flags:**
- `--precompute-witnesses` - **REQUIRED** for this benchmark. Generates and stores execution witnesses during payload execution.

**Notes:**
- Run in release mode for performance
- **NEVER clear DB without explicit approval** - ask first with full context
- To clear DB (only with approval): `cargo run --release --bin ethrex -- removedb --force`

---

## Workflow

### Option A: Fresh Sync (servers not yet synced)

#### 1. Start Lighthouse (in tmux session "lighthouse")

```bash
ssh <server>
tmux new -s lighthouse
lighthouse bn --network <NETWORK> --execution-endpoint http://localhost:8551 --execution-jwt ~/secrets/jwt.hex --http --checkpoint-sync-url <CHECKPOINT_SYNC_URL>
# Detach: Ctrl+B, D
```

#### 2. Start Ethrex (in tmux session "ethrex")

```bash
ssh <server>
tmux new -s ethrex
cd ~/ethrex  # or wherever the repo is

# For baseline:
git checkout main && git pull

# For PR:
git fetch origin pull/<PR_NUMBER>/head:pr-<PR_NUMBER> && git checkout pr-<PR_NUMBER>

cargo run --release --bin ethrex -- --network <NETWORK> --authrpc.jwtsecret ~/secrets/jwt.hex --precompute-witnesses
# Detach: Ctrl+B, D
```

#### 3. Wait for Sync

Wait for both servers to sync to the head of the chain. Monitor progress:

```bash
# Reattach to sessions
tmux attach -t lighthouse
tmux attach -t ethrex

# List sessions
tmux ls
```

**Sync completion indicators:**
- Lighthouse: "Synced" status in logs
- Ethrex: Following new payloads from beacon chain

#### 4. Proceed to Benchmarking (Step 3)

---

### Option B: Servers Already Synced

If servers are already synced with `--precompute-witnesses` enabled:

#### 1. Verify Configuration

```bash
# On both servers, verify ethrex is running with --precompute-witnesses
ssh <server> "ps aux | grep ethrex | grep precompute-witnesses"
```

#### 2. Proceed to Benchmarking (Step 3)

---

### 3. Run Benchmark with ethrex-replay

> [!IMPORTANT]
> All measurements should be obtained using an RPC node running on the **same machine** as ethrex to avoid network-related latency affecting results.

#### Build ethrex-replay with Debug Logging

On both servers, enable debug logging for `ethrex-replay`:

```bash
ssh <server>
cd ~/ethrex-replay

# Edit src/main.rs in ethrex-replay to enable debug logging
# Change line 21 to:
#   add_directive(Directive::from(tracing::Level::DEBUG))

# Or use sed:
sed -i 's/add_directive(Directive::from(tracing::Level::INFO))/add_directive(Directive::from(tracing::Level::DEBUG))/' src/main.rs

# Build ethrex-replay
cargo build --release
```

#### Run ethrex-replay in Endless Mode

```bash
ssh <server>
tmux new -s replay
cd ~/ethrex-replay

# Run with logging capture
cargo run --release -- blocks --endless --rpc-url http://localhost:8545 2>&1 | grep --line-buffered 'Got execution witness for block' | tee execution_witness_times.txt

# Detach: Ctrl+B, D
```

#### 4. Collect Measurements

Let the benchmark run for at least **200 blocks** (or more for statistical significance).

```bash
# Check progress
ssh <server> "wc -l ~/ethrex-replay/execution_witness_times.txt"

# View sample output
ssh <server> "tail -10 ~/ethrex-replay/execution_witness_times.txt"
```

**Expected log format:**
```
2024-01-15T10:30:00.123Z DEBUG ethrex_replay: Got execution witness for block 24191178 in 131ms
```

---

## Monitoring Protocol

### Real-time Comparison Script

**Usage:** Set `MAIN_HOST` and `TEST_HOST` variables, then run the script.

```bash
#!/bin/bash
# Execution Witness Latency Comparison Script
# Monitors ethrex-replay output and compares latencies between servers

# Required environment variables
if [[ -z "$MAIN_HOST" || -z "$TEST_HOST" ]]; then
    echo "Error: MAIN_HOST and TEST_HOST must be set"
    echo "Usage: MAIN_HOST=<baseline-server> TEST_HOST=<test-server> $0"
    exit 1
fi

# Optional configuration
MIN_BLOCKS="${MIN_BLOCKS:-200}"  # Minimum blocks to collect
POLL_INTERVAL="${POLL_INTERVAL:-30}"  # Seconds between polls

# Temporary files
MAIN_TIMES="/tmp/main_witness_times.txt"
TEST_TIMES="/tmp/test_witness_times.txt"

fetch_times() {
    local host=$1
    local output=$2
    ssh "$host" "cat ~/ethrex-replay/execution_witness_times.txt 2>/dev/null" > "$output"
}

calculate_stats() {
    local file=$1
    if [[ ! -s "$file" ]]; then
        echo "0 0 0 0 0"
        return
    fi

    # Extract times from log lines: "... in 131ms"
    local times=$(grep -oE 'in [0-9]+ms' "$file" | grep -oE '[0-9]+' | sort -n)
    local count=$(echo "$times" | wc -l | tr -d ' ')

    if [[ $count -eq 0 ]]; then
        echo "0 0 0 0 0"
        return
    fi

    local min=$(echo "$times" | head -1)
    local max=$(echo "$times" | tail -1)
    local sum=$(echo "$times" | awk '{s+=$1} END {print s}')
    local avg=$((sum / count))

    # Median
    local mid=$((count / 2))
    local median=$(echo "$times" | sed -n "${mid}p")

    echo "$count $min $max $avg $median"
}

print_comparison() {
    local main_stats=($1)
    local test_stats=($2)

    local main_count=${main_stats[0]}
    local main_min=${main_stats[1]}
    local main_max=${main_stats[2]}
    local main_avg=${main_stats[3]}
    local main_median=${main_stats[4]}

    local test_count=${test_stats[0]}
    local test_min=${test_stats[1]}
    local test_max=${test_stats[2]}
    local test_avg=${test_stats[3]}
    local test_median=${test_stats[4]}

    clear
    echo "EXECUTION WITNESS LATENCY COMPARISON"
    echo "Main: $MAIN_HOST (baseline) | Test: $TEST_HOST (optimization)"
    echo "Polling every ${POLL_INTERVAL} seconds... (Ctrl+C to stop)"
    echo ""
    echo "╔══════════════════════╦══════════════╦══════════════╦════════════════╗"
    echo "║       Metric         ║     Main     ║     Test     ║   Improvement  ║"
    echo "╠══════════════════════╬══════════════╬══════════════╬════════════════╣"
    printf "║ %-20s ║ %12s ║ %12s ║ %14s ║\n" "Blocks Analyzed" "$main_count" "$test_count" "--"

    # Calculate improvements
    local avg_improvement="--"
    local median_improvement="--"
    local max_improvement="--"

    if [[ $main_avg -gt 0 && $test_avg -gt 0 ]]; then
        avg_improvement=$(echo "scale=1; (($main_avg - $test_avg) * 100) / $main_avg" | bc -l)
        avg_improvement="${avg_improvement}%"
    fi

    if [[ $main_median -gt 0 && $test_median -gt 0 ]]; then
        median_improvement=$(echo "scale=1; (($main_median - $test_median) * 100) / $main_median" | bc -l)
        median_improvement="${median_improvement}%"
    fi

    if [[ $main_max -gt 0 && $test_max -gt 0 ]]; then
        max_improvement=$(echo "scale=1; (($main_max - $test_max) * 100) / $main_max" | bc -l)
        max_improvement="${max_improvement}%"
    fi

    printf "║ %-20s ║ %10s ms ║ %10s ms ║ %14s ║\n" "Min Time" "$main_min" "$test_min" "--"
    printf "║ %-20s ║ %10s ms ║ %10s ms ║ %14s ║\n" "Max Time" "$main_max" "$test_max" "$max_improvement"
    printf "║ %-20s ║ %10s ms ║ %10s ms ║ %14s ║\n" "Average Time" "$main_avg" "$test_avg" "$avg_improvement"
    printf "║ %-20s ║ %10s ms ║ %10s ms ║ %14s ║\n" "Median Time" "$main_median" "$test_median" "$median_improvement"
    echo "╚══════════════════════╩══════════════╩══════════════╩════════════════╝"
    echo ""
    echo "Last updated: $(date -u '+%Y-%m-%d %H:%M:%S UTC')"

    # Check if we have enough data
    local min_count=$((main_count < test_count ? main_count : test_count))
    if [[ $min_count -lt $MIN_BLOCKS ]]; then
        echo ""
        echo "Collecting data... Need at least $MIN_BLOCKS blocks (currently: $min_count)"
    else
        echo ""
        echo "Sufficient data collected. Press Ctrl+C to finish and see final report."
    fi
}

# Main loop
echo "Starting execution witness latency monitoring..."
echo "Main: $MAIN_HOST | Test: $TEST_HOST"
echo ""

while true; do
    fetch_times "$MAIN_HOST" "$MAIN_TIMES"
    fetch_times "$TEST_HOST" "$TEST_TIMES"

    main_stats=$(calculate_stats "$MAIN_TIMES")
    test_stats=$(calculate_stats "$TEST_TIMES")

    print_comparison "$main_stats" "$test_stats"

    sleep "$POLL_INTERVAL"
done
```

### Quick Start

```bash
# Required: Set hosts (replace with actual server hostnames/aliases)
export MAIN_HOST="<baseline-server>"  # e.g., ethrex-mainnet-2
export TEST_HOST="<test-server>"      # e.g., ethrex-mainnet-3

# Optional configuration
export MIN_BLOCKS=200   # default: 200
export POLL_INTERVAL=30 # default: 30 seconds

# Run the monitoring script (save script above as witness_monitor.sh)
chmod +x witness_monitor.sh
./witness_monitor.sh
```

---

## Post-Processing Script

After collecting sufficient data, use this script to generate the final report:

```bash
#!/bin/bash
# Generate final comparison report

if [[ -z "$MAIN_HOST" || -z "$TEST_HOST" ]]; then
    echo "Error: MAIN_HOST and TEST_HOST must be set"
    exit 1
fi

echo "Fetching final data..."

# Fetch data
ssh "$MAIN_HOST" "cat ~/ethrex-replay/execution_witness_times.txt" > /tmp/main_final.txt
ssh "$TEST_HOST" "cat ~/ethrex-replay/execution_witness_times.txt" > /tmp/test_final.txt

# Process main data
echo ""
echo "=== BASELINE (Main Branch) ==="
main_times=$(grep -oE 'in [0-9]+ms' /tmp/main_final.txt | grep -oE '[0-9]+')
main_blocks=$(grep -oE 'block [0-9]+' /tmp/main_final.txt | grep -oE '[0-9]+')
main_first=$(echo "$main_blocks" | head -1)
main_last=$(echo "$main_blocks" | tail -1)
main_count=$(echo "$main_times" | wc -l | tr -d ' ')
main_min=$(echo "$main_times" | sort -n | head -1)
main_max=$(echo "$main_times" | sort -n | tail -1)
main_sum=$(echo "$main_times" | awk '{s+=$1} END {print s}')
main_avg=$((main_sum / main_count))
main_median=$(echo "$main_times" | sort -n | sed -n "$((main_count / 2))p")

echo "Blocks analyzed: $main_count"
echo "Block range: $main_first - $main_last"
echo "Min: ${main_min}ms | Max: ${main_max}ms | Avg: ${main_avg}ms | Median: ${main_median}ms"

# Process test data
echo ""
echo "=== TEST (PR Branch) ==="
test_times=$(grep -oE 'in [0-9]+ms' /tmp/test_final.txt | grep -oE '[0-9]+')
test_blocks=$(grep -oE 'block [0-9]+' /tmp/test_final.txt | grep -oE '[0-9]+')
test_first=$(echo "$test_blocks" | head -1)
test_last=$(echo "$test_blocks" | tail -1)
test_count=$(echo "$test_times" | wc -l | tr -d ' ')
test_min=$(echo "$test_times" | sort -n | head -1)
test_max=$(echo "$test_times" | sort -n | tail -1)
test_sum=$(echo "$test_times" | awk '{s+=$1} END {print s}')
test_avg=$((test_sum / test_count))
test_median=$(echo "$test_times" | sort -n | sed -n "$((test_count / 2))p")

echo "Blocks analyzed: $test_count"
echo "Block range: $test_first - $test_last"
echo "Min: ${test_min}ms | Max: ${test_max}ms | Avg: ${test_avg}ms | Median: ${test_median}ms"

# Calculate improvements
echo ""
echo "=== IMPROVEMENTS ==="
avg_improvement=$(echo "scale=1; (($main_avg - $test_avg) * 100) / $main_avg" | bc -l)
median_improvement=$(echo "scale=1; (($main_median - $test_median) * 100) / $main_median" | bc -l)
max_improvement=$(echo "scale=1; (($main_max - $test_max) * 100) / $main_max" | bc -l)
avg_reduction=$((main_avg - test_avg))

echo "Average latency reduction: ${avg_reduction}ms (${avg_improvement}%)"
echo "Median latency reduction: $((main_median - test_median))ms (${median_improvement}%)"
echo "Max latency reduction: $((main_max - test_max))ms (${max_improvement}%)"
```

---

## Results Template

### Test Session Info

| Field | Value |
|-------|-------|
| **Date** | YYYY-MM-DD |
| **Network** | |
| **Baseline Server** | |
| **Test Server** | |
| **PR/Branch** | |
| **Start Time (UTC)** | |
| **End Time (UTC)** | |

### Execution Witness Latency Comparison

| Metric | Baseline (Main) | Test (PR Branch) | Improvement |
|--------|-----------------|--------------------------|-------------|
| **Total Blocks Analyzed** | | | -- |
| **Min Time** | ms | ms | -- |
| **Max Time** | ms | ms | % |
| **Average Time** | ms | ms | % |
| **Median Time** | ms | ms | % |
| **Block Range** | – | – | -- |

### Observations

(Add notes about any anomalies, errors, or interesting findings)

### Conclusion

(Summarize whether the PR improves, regresses, or has no effect on performance)

---

## Troubleshooting

### Common Issues

| Issue | Solution |
|-------|----------|
| No execution witness times logged | Ensure `--precompute-witnesses` flag is enabled |
| Empty execution_witness_times.txt | Check DEBUG logging is enabled in ethrex-replay |
| Very high latencies on both | RPC may be on different machine; run replay locally |
| Inconsistent results | Let sync stabilize before starting benchmark |
| "witness not found" errors | Witnesses only stored for 128 blocks; ensure replay is within range |

### Verification Steps

```bash
# 1. Verify ethrex running with precompute-witnesses
ssh <server> "ps aux | grep ethrex | grep precompute-witnesses"

# 2. Verify ethrex-replay has DEBUG logging
ssh <server> "grep -n 'Level::DEBUG' ~/ethrex-replay/src/main.rs"

# 3. Check replay output
ssh <server> "tail -10 ~/ethrex-replay/execution_witness_times.txt"
```

### Log Locations

- ethrex: `~/ethrex/ethrex.log` or stdout in tmux
- ethrex-replay: `~/ethrex-replay/execution_witness_times.txt`
- lighthouse: stdout in tmux session
