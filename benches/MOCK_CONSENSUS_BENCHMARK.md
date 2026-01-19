# Mock Consensus Benchmark

A benchmarking tool that simulates a consensus client to measure ethrex's Engine API performance under realistic block production conditions.

## Overview

The mock consensus client (`mock_consensus`) acts as a simplified consensus layer that:

1. **Generates transactions**: Creates EIP-1559 transfer transactions between randomly selected accounts from a pre-funded genesis
2. **Sends transactions to mempool**: Submits transactions via `eth_sendRawTransaction`
3. **Triggers block building**: Calls `engine_forkchoiceUpdatedV3` with payload attributes
4. **Retrieves built blocks**: Calls `engine_getPayloadV3` to get the assembled block
5. **Validates blocks**: Calls `engine_newPayloadV3` to execute and validate the block
6. **Finalizes blocks**: Calls `engine_forkchoiceUpdatedV3` to make the block canonical

This simulates the Ethereum mainnet block production cycle (12-second slots) while measuring the time taken for each Engine API call.

## Prerequisites

### 1. Build the binaries

```bash
# Build ethrex node with dev feature
cargo build --bin ethrex --release --features dev

# Build the mock consensus client
cargo build -p ethrex-benches --bin mock_consensus --release
```

### 2. Generate a genesis file with pre-funded accounts

```bash
# Generate genesis with 1 million accounts (default)
cargo run -p ethrex-benches --bin generate_genesis --release -- \
    --accounts 1000000 \
    --output test_data/genesis_1m

# This creates:
#   - test_data/genesis_1m/genesis.json (~160 MB)
#   - test_data/genesis_1m/private_keys.txt (~67 MB)
```

## Running the Benchmark

### Quick Start (Automated Script)

The easiest way to run the benchmark is using the provided script:

```bash
./benches/run_mock_consensus_bench.sh
```

This script handles:
- Building binaries if needed
- Creating JWT secret
- Starting the ethrex node
- Running the benchmark
- Collecting results
- Cleaning up

### Script Options

```bash
./benches/run_mock_consensus_bench.sh [NUM_BLOCKS] [TXS_PER_BLOCK] [SLOT_TIME_MS] [MAX_ACCOUNTS]
```

| Parameter | Default | Description |
|-----------|---------|-------------|
| `NUM_BLOCKS` | 100 | Number of blocks to produce |
| `TXS_PER_BLOCK` | 400 | Transactions per block |
| `SLOT_TIME_MS` | 1000 | Time between blocks (ms) |
| `MAX_ACCOUNTS` | 10000 | Max accounts to load for signing |

**Examples:**

```bash
# Default: 100 blocks, 400 txs/block, 1s slots
./benches/run_mock_consensus_bench.sh

# Quick test: 10 blocks
./benches/run_mock_consensus_bench.sh 10

# High throughput: 200 blocks, 600 txs/block, 500ms slots
./benches/run_mock_consensus_bench.sh 200 600 500

# Mainnet simulation: 100 blocks, 400 txs, 12s slots
./benches/run_mock_consensus_bench.sh 100 400 12000
```

### Manual Execution

If you prefer to run components separately:

```bash
# 1. Create JWT secret
echo "0x$(openssl rand -hex 32)" > jwt.hex

# 2. Start the node (in one terminal)
./target/release/ethrex \
    --dev \
    --dev.no-blocks \
    --network test_data/genesis_1m/genesis.json \
    --datadir dev_data \
    --force \
    --authrpc.jwtsecret jwt.hex

# 3. Run mock consensus (in another terminal)
./target/release/mock_consensus \
    --node-url http://localhost:8545 \
    --auth-url http://localhost:8551 \
    --jwt-secret jwt.hex \
    --keys-file test_data/genesis_1m/private_keys.txt \
    --num-blocks 100 \
    --txs-per-block 400 \
    --slot-time 1000 \
    --output timing_results.csv
```

### Mock Consensus Options

| Option | Default | Description |
|--------|---------|-------------|
| `--node-url` | `http://localhost:8545` | HTTP RPC endpoint |
| `--auth-url` | `http://localhost:8551` | Auth RPC endpoint (Engine API) |
| `--jwt-secret` | `jwt.hex` | Path to JWT secret file |
| `--keys-file` | `test_data/genesis_1m/private_keys.txt` | Path to private keys |
| `--num-blocks` | 10 | Number of blocks to produce |
| `--txs-per-block` | 400 | Transactions per block |
| `--slot-time` | 12000 | Time between blocks (ms) |
| `--max-accounts` | 10000 | Max accounts to load |
| `--output` | `timing_results.csv` | Output file for timing data |

## Output Files

After running the benchmark, you'll find:

### 1. Timing Results CSV

**File:** `timing_results_YYYYMMDD_HHMMSS.csv`

Contains detailed timing for every Engine API call:

```csv
block_number,timestamp,call_type,duration_ms,success,error
1,2026-01-16T13:01:17.284456+00:00,forkchoiceUpdatedV3 (build),1.990,true,
1,2026-01-16T13:01:17.307811+00:00,getPayloadV3,23.350,true,
1,2026-01-16T13:01:17.324684+00:00,newPayloadV3,16.865,true,
1,2026-01-16T13:01:17.325293+00:00,forkchoiceUpdatedV3 (finalize),0.603,true,
...
```

**Columns:**
- `block_number`: Block being produced
- `timestamp`: ISO 8601 timestamp of the call
- `call_type`: Engine API method called
- `duration_ms`: Time taken in milliseconds
- `success`: Whether the call succeeded
- `error`: Error message if failed

### 2. Node Log

**File:** `node_output_YYYYMMDD_HHMMSS.log`

Contains the ethrex node output including:
- Genesis initialization times
- Block execution logs
- Any errors or warnings

### 3. Console Summary

The benchmark prints a summary to stdout:

```
========================================
           TIMING SUMMARY
========================================

  forkchoiceUpdatedV3 (build): count=100, mean=0.53ms, min=0.46ms, max=1.99ms, success=100, failed=0
  getPayloadV3: count=100, mean=13.11ms, min=10.95ms, max=23.35ms, success=100, failed=0
  newPayloadV3: count=100, mean=10.19ms, min=8.91ms, max=16.87ms, success=100, failed=0
  forkchoiceUpdatedV3 (finalize): count=100, mean=0.58ms, min=0.52ms, max=0.76ms, success=100, failed=0

Total blocks attempted: 100
Overall API call success rate: 400/400 (100.0%)

========================================

========================================
         BENCHMARK RUNTIME
========================================
  Total time: 1m 42s
  Blocks produced: 100
  Avg time per block: 1024.15ms
========================================
```

The benchmark runtime section shows the total execution time excluding initial setup (waiting for the node's first response). This is useful for comparing different configurations.

## Analyzing Results

### Using Python/Pandas

```python
import pandas as pd
import matplotlib.pyplot as plt

# Load data
df = pd.read_csv('timing_results_20260116_140058.csv')

# Summary statistics by call type
print(df.groupby('call_type')['duration_ms'].describe())

# Plot timing distribution
df.boxplot(column='duration_ms', by='call_type', figsize=(10, 6))
plt.title('Engine API Call Duration Distribution')
plt.ylabel('Duration (ms)')
plt.xticks(rotation=45)
plt.tight_layout()
plt.savefig('timing_distribution.png')

# Plot timing over blocks
for call_type in df['call_type'].unique():
    subset = df[df['call_type'] == call_type]
    plt.plot(subset['block_number'], subset['duration_ms'], label=call_type)
plt.xlabel('Block Number')
plt.ylabel('Duration (ms)')
plt.legend()
plt.title('Engine API Timing Over Blocks')
plt.savefig('timing_over_blocks.png')
```

### Using Command Line Tools

```bash
# Quick statistics
awk -F',' 'NR>1 {sum[$3]+=$4; count[$3]++} END {for (c in sum) print c, sum[c]/count[c] "ms avg"}' timing_results.csv

# Find slowest calls
sort -t',' -k4 -rn timing_results.csv | head -10

# Count by call type
cut -d',' -f3 timing_results.csv | sort | uniq -c

# Filter failed calls
grep ',false,' timing_results.csv
```

## What to Expect

### Typical Timing Results (1M account genesis, 400 txs/block)

| API Call | Expected Range |
|----------|---------------|
| forkchoiceUpdatedV3 (build) | 0.3-2ms |
| getPayloadV3 | 10-25ms |
| newPayloadV3 | 8-20ms |
| forkchoiceUpdatedV3 (finalize) | 0.5-1ms |

### Genesis Initialization

| Phase | Expected Time (1M accounts) |
|-------|----------------------------|
| JSON parsing | ~700ms |
| Trie construction | ~9-10s |
| DB commit | ~1-2s |
| **Total** | **~11-12s** |

### Performance Notes

1. **First block is slower**: The first block after genesis may take longer due to cache warming
2. **Memory usage**: Loading 10,000 accounts for signing uses ~100MB RAM
3. **Disk I/O**: Results depend heavily on storage speed (SSD recommended)
4. **CPU**: Block building and execution are CPU-bound operations

## Troubleshooting

### "Unsupported fork" errors

The genesis file must match the Engine API version:
- **Cancun fork**: Use `engine_*V3` APIs (current default)
- **Prague fork**: Use `engine_*V4/V5` APIs

The genesis generator creates Cancun-level genesis by default.

### Node not starting

Check the node log for errors:
```bash
tail -50 node_output_*.log
```

Common issues:
- Port already in use (8545 or 8551)
- Invalid genesis file
- Missing JWT secret

### Transactions failing

If transactions fail to send:
- Ensure accounts have sufficient balance
- Check nonce tracking (restart clears nonces)
- Verify chain ID matches genesis (default: 1337)

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                         Mock Consensus Client                        │
├─────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐          │
│  │   Account    │    │  Transaction │    │    Timing    │          │
│  │   Manager    │───▶│  Generator   │───▶│   Recorder   │          │
│  │ (10k keys)   │    │ (400 txs)    │    │   (CSV)      │          │
│  └──────────────┘    └──────────────┘    └──────────────┘          │
│         │                   │                   │                   │
│         ▼                   ▼                   ▼                   │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │                      Engine API Client                       │   │
│  │  - forkchoiceUpdatedV3 (build)     ──────────────────┐      │   │
│  │  - getPayloadV3                    ──────────────────┤      │   │
│  │  - newPayloadV3                    ──────────────────┤      │   │
│  │  - forkchoiceUpdatedV3 (finalize)  ──────────────────┘      │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                              │                                      │
└──────────────────────────────┼──────────────────────────────────────┘
                               │ HTTP + JWT Auth
                               ▼
┌─────────────────────────────────────────────────────────────────────┐
│                          ethrex Node                                 │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐           │
│  │ Mempool  │  │  Block   │  │   EVM    │  │  State   │           │
│  │          │  │ Builder  │  │          │  │  Trie    │           │
│  └──────────┘  └──────────┘  └──────────┘  └──────────┘           │
└─────────────────────────────────────────────────────────────────────┘
```

## Related Files

- `benches/src/mock_consensus.rs` - Mock consensus client implementation
- `benches/src/generate_genesis.rs` - Genesis file generator
- `benches/run_mock_consensus_bench.sh` - Benchmark automation script
- `cmd/ethrex/cli.rs` - Node CLI with `--dev.no-blocks` flag
