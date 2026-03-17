# SP1 Performance Comparison: evm-l2 vs zk-dex Guest Program

**Date**: 2026-02-23
**Branch**: `feat/zk/guest-program-modularization`
**SP1 Version**: v5.0.8 (succinct toolchain)

## Objective

Compare SP1 proving performance between the standard evm-l2 guest program (full EVM
execution) and the application-specific zk-dex guest program (keccak256 hashing only).
This validates the core thesis of guest program modularization: **application-specific
programs can achieve dramatically lower proving times by eliminating unnecessary
computation**.

## Environment

| Item | Value |
|------|-------|
| Machine | Apple M4 Max |
| Architecture | x86_64 (Rosetta 2 emulation) |
| OS | macOS Darwin 24.4.0 |
| CPU mode | CPU only (no GPU) |
| SP1 SDK | 5.0.8 |

> Same environment as the baseline profiling (`sp1-profiling-baseline.md`).

---

## Benchmark Method

### evm-l2 (Baseline)

Data from `tokamak-notes/sp1-profiling-baseline.md`. The evm-l2 guest program processes
a full L2 batch: EVM transaction execution, state trie updates, receipt validation,
and state root computation.

### zk-dex

Measured using the `sp1_benchmark` binary (`crates/l2/prover/src/bin/sp1_benchmark.rs`).
The zk-dex guest program processes a batch of token transfers: for each transfer,
it computes `state = keccak256(state || from || to || token || amount || nonce)`.

```bash
cargo run --release --features sp1 --bin sp1_benchmark -- \
  --program zk-dex --transfers 100
```

Input: 100 deterministic DexTransfer entries, rkyv-serialized.

---

## Results

### Summary

| Metric | evm-l2 (Batch 1) | zk-dex (100 transfers) | Reduction |
|--------|-------------------|------------------------|-----------|
| Total cycles | 65,360,896 | *pending* | — |
| Execution time | ~1s | *pending* | — |
| Total proving time | 27.7 min (1,664.55s) | *pending* | — |
| STARK core proving | ~12.4 min | *pending* | — |
| Recursive compression | ~14.5 min | *pending* | — |
| Groth16 wrapping | ~17s | *pending* | — |

### Cycle Breakdown

#### evm-l2 (65,360,896 total cycles)

| Section | Cycles | % |
|---------|--------|---|
| `read_input` | 1,012,951 | 1.55% |
| `execution` | 64,345,179 | 98.45% |
| `commit_public_inputs` | 2,766 | 0.004% |

Top execution components:
- `execute_block`: 29,363,722 (45.6%)
- `validate_receipts_root`: 4,619,876 (7.2%)
- `apply_account_updates`: 2,824,380 (4.4%)
- Unattributed overhead: 24,442,288 (38.0%)

#### zk-dex (pending)

| Section | Cycles | % |
|---------|--------|---|
| `read_input` | *pending* | — |
| `execution` | *pending* | — |
| `commit_public_inputs` | *pending* | — |

Expected: execution is dominated by 100x keccak256(108 bytes) — significantly
fewer cycles than full EVM execution.

---

## Analysis

### Why zk-dex Should Be Faster

The evm-l2 guest program includes:
- Full EVM interpreter (opcode dispatch, stack, memory, storage)
- Merkle Patricia Trie operations (state root, receipts root, account updates)
- RLP encoding for receipts and transactions
- Block header validation
- Gas accounting

The zk-dex guest program includes **only**:
- rkyv deserialization of transfer batch
- N iterations of keccak256(108 bytes) for state transitions
- Output encoding (72 bytes)

This eliminates ~99% of the computation in the evm-l2 program.

### Implications for Guest Program Modularization

If the zk-dex proving time is significantly lower (e.g., <1 minute vs 27.7 minutes),
this demonstrates that:

1. **Application-specific guest programs are viable** for production use
2. **Proving time scales with actual computation**, not framework overhead
3. **SP1's recursive compression time may be the new bottleneck** for small programs
   (since it's somewhat fixed regardless of cycle count)
4. **L2 operators can choose the right tradeoff**: full EVM flexibility vs
   application-specific proving speed

---

## Reproduction

### Build zk-dex SP1 ELF

```bash
cd crates/guest-program
GUEST_PROGRAMS=zk-dex cargo build --release -p ethrex-guest-program --features sp1
```

### Run Benchmark

```bash
# Execute only (fast, cycle count only)
cargo run --release --features sp1 --bin sp1_benchmark -- \
  --program zk-dex --transfers 100 --execute-only

# Full benchmark with Compressed proof
cargo run --release --features sp1 --bin sp1_benchmark -- \
  --program zk-dex --transfers 100

# Full benchmark with Groth16 proof
cargo run --release --features sp1 --bin sp1_benchmark -- \
  --program zk-dex --transfers 100 --format groth16
```

### Run evm-l2 Baseline

See `tokamak-notes/sp1-profiling-baseline.md` for the evm-l2 baseline methodology.
The evm-l2 benchmark requires a running L2 stack (Docker Compose) to generate
real batch inputs.

---

## Files

| File | Description |
|------|-------------|
| `crates/l2/prover/src/bin/sp1_benchmark.rs` | Benchmark binary |
| `crates/guest-program/bin/sp1-zk-dex/src/main.rs` | zk-dex SP1 guest (with cycle tracking) |
| `crates/guest-program/src/programs/zk_dex/execution.rs` | zk-dex execution logic |
| `tokamak-notes/sp1-profiling-baseline.md` | evm-l2 baseline data |
