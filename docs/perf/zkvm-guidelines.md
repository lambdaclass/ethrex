# zkVM-Specific Benchmarking Guidelines

When benchmarking zkVM provers (ZisK, SP1, RISC0, etc.), additional considerations apply beyond standard LEVM benchmarking.

---

## Claude Instructions for zkVM Benchmarking

**When the user asks to benchmark zkVM proving (ZisK, SP1, RISC0):**

### Pre-Flight Checks

1. **Verify prover is installed:**
   ```bash
   # ZisK
   cargo-zisk --version

   # SP1
   cargo prove --version

   # RISC0
   cargo risczero --version
   ```

2. **Check GPU availability (for ZisK/RISC0):**
   ```bash
   nvidia-smi
   ```

3. **Check GPU temperature before starting:**
   ```bash
   nvidia-smi --query-gpu=temperature.gpu --format=csv,noheader
   # Should be <50°C before starting
   ```

### Running zkVM Benchmarks

**IMPORTANT:** zkVM proofs take minutes, not milliseconds. Always use background execution:

```bash
# Start proving in background
Bash(
  command="time cargo-zisk prove -e $ELF -i $INPUT -o /tmp/proof -a -u -y 2>&1 | tee proof_output.log",
  run_in_background=true
)
```

Then check progress:
```bash
TaskOutput(task_id="...", block=false)  # Non-blocking check
# or
tail -20 proof_output.log  # Read recent output
```

### Cooldown Between Proofs

**GPU proofs generate significant heat.** Between runs:
1. Wait for GPU temp to drop below 50°C
2. Minimum 2 minutes between proof attempts
3. Check: `nvidia-smi --query-gpu=temperature.gpu --format=csv,noheader`

### Reporting zkVM Results

Report in this format:
```
zkVM Experiment NNN: [Name]
Prover: ZisK v0.15.0 / SP1 / RISC0
GPU: RTX 3090
Proof time: 262s (baseline: 310s)
Change: -15.5%
Steps: 1.5B
Verdict: KEEP
```

---

## Additional Metrics

Track zkVM-specific metrics beyond time:

| Metric | Description | Why It Matters |
|--------|-------------|----------------|
| **Steps** | Total execution steps | Correlates with proving time |
| **Proof instances** | Number of proof segments | Affects parallelization |
| **Cycles** | CPU cycles in guest | Direct cost measure |
| **Proof size** | Output proof bytes | On-chain verification cost |
| **Memory peak** | Max RAM during proving | Hardware requirements |
| **GPU utilization** | % GPU used | Efficiency measure |

### Baseline Template for zkVM

```yaml
# docs/perf/experiments/NNN-zkvm-experiment/baseline.yaml
primary:
  proof_time_seconds: 262.0
  proof_time_stddev: 5.2

secondary:
  steps: 1_500_000_000
  cycles: 2_100_000_000
  proof_size_bytes: 1_048_576
  memory_peak_mb: 8500
  gpu_utilization_percent: 85

profile:
  top_functions:
    - name: "Node::memoize_hashes"
      cost_percent: 59.08
    - name: "LEVM::execute_tx"
      cost_percent: 36.58
```

---

## Profiler Commands

### ZisK

```bash
# Full profiling with ziskemu
ziskemu -e $ELF -i $INPUT -D -X -S > profile.txt

# Flags:
# -D: Debug output
# -X: Extended profiling
# -S: Step counting

# Prove with timing
time cargo-zisk prove -e $ELF -i $INPUT -o /tmp/proof -a -u -y
```

### SP1

```bash
# Enable tracing
TRACE_FILE=trace.json cargo run --release -- prove

# View trace
# Use trace viewer or convert to flamegraph
```

### RISC0

```bash
# Enable pprof output
RISC0_PPROF_OUT=profile.pb cargo run --release

# View with pprof
go tool pprof -http=:8080 profile.pb
```

---

## Block Selection for zkVM

Select blocks with diverse characteristics:

| Block Type | Characteristics | Why Include |
|------------|-----------------|-------------|
| **Light** | <50 txs, no precompiles | Lower bound, sanity check |
| **Keccak-heavy** | Many storage operations | Tests hash optimization |
| **ECRECOVER-heavy** | Many signature verifications | Tests secp256k1 patch |
| **BN254-heavy** | BN254 pairing operations | Tests bn254 patch |
| **BLS12-381-heavy** | BLS operations | Tests bls12-381 patch |
| **Large state** | Many unique accounts | Tests MPT handling |
| **Typical** | Average mainnet block | Representative case |

### Example Input Selection

```yaml
# For zkVM proving benchmarks
inputs:
  - name: "light_simple"
    block_range: "1000-1010"
    description: "10 early blocks, minimal complexity"

  - name: "ecrecover_heavy"
    block_range: "TBD"
    description: "Blocks with many signature verifications"

  - name: "bn254_heavy"
    block_range: "TBD"
    description: "Blocks with BN254 precompile calls"

  - name: "storage_heavy"
    block_range: "TBD"
    description: "Blocks with many SLOAD/SSTORE"

  - name: "mainnet_typical"
    block_range: "19000000-19000010"
    description: "Recent mainnet blocks"
```

---

## Proof Timeouts by GPU

Expected proving times vary significantly by GPU. Set timeouts accordingly:

| GPU | Typical Time | Timeout (1.10x) | Expected Range |
|-----|--------------|-----------------|----------------|
| RTX 3090 | ~5 min | ~5.5 min | 3-8 min |
| RTX 4090 | ~2 min | ~2.2 min | 1-4 min |
| RTX 5090 | ~1 min | ~1.1 min | 30s-2 min |
| A100 | ~1.5 min | ~1.7 min | 1-3 min |
| H100 | ~30s | ~33s | 20s-1 min |

**Note:** Times are approximate and depend heavily on:
- Block complexity
- Prover version
- Patches enabled
- Memory bandwidth

---

## Common zkVM Optimization Targets

| Target | Typical Impact | Approach |
|--------|----------------|----------|
| **MPT hashing** | 40-60% of cost | Cache intermediate hashes, lazy computation |
| **Deserialization** | 5-15% of cost | Zero-copy, compression |
| **memcpy** | 10-25% of cost | Reduce copies, use references |
| **Crypto patches** | Varies | Ensure all patches active and correct |
| **Memory allocation** | 5-10% | Arena allocator, pooling |

### Checking Patch Status

Before benchmarking, verify patches are active:

```bash
# Check ZisK patches
cargo-zisk check-patches

# Check SP1 patches
# Look for sp1-zkvm features in Cargo.toml

# Check RISC0 patches
# Look for risc0-zkvm features in Cargo.toml
```

---

## zkVM-Specific Decision Criteria

Adjust standard thresholds for zkVM:

| Result | L1 Benchmark | zkVM Benchmark |
|--------|--------------|----------------|
| Keep threshold | >5% | >3% (proving is expensive) |
| Maybe threshold | 2-5% | 1-3% |
| Timeout multiplier | 1.10x | 1.15x (more variance) |

**Why lower thresholds for zkVM:**
- Even small improvements matter when proofs take minutes
- Compound gains are more significant at zkVM scale
- Hardware costs are higher for proving

---

## Environment Documentation for zkVM

Additional environment fields to capture:

```yaml
# docs/perf/experiments/NNN-zkvm-experiment/environment.yaml
machine:
  hostname: ethrex-prover-1
  cpu: AMD Ryzen 9 5950X
  gpu: NVIDIA RTX 3090
  gpu_memory: 24GB
  cuda_version: 13.0
  memory: 64GB
  os: Ubuntu 22.04

toolchain:
  rust: 1.84.0
  zisk: v0.15.0  # or sp1, risc0 version
  cuda: 13.0

patches:
  keccak: true
  secp256k1: true
  bn254: true
  bls12_381: true
  modexp: false  # ZisK-only

commit: abc123def
date: 2026-01-16
```

---

## GPU Monitoring During Benchmarks

Monitor GPU during zkVM proving:

```bash
# Watch GPU stats every second
watch -n 1 nvidia-smi

# Query specific metrics
nvidia-smi --query-gpu=timestamp,name,temperature.gpu,utilization.gpu,memory.used --format=csv -l 1

# Log to file during benchmark
nvidia-smi --query-gpu=timestamp,temperature.gpu,utilization.gpu,memory.used --format=csv -l 1 > gpu_log.csv &
NVIDIA_LOG_PID=$!
# Run benchmark
# ...
kill $NVIDIA_LOG_PID
```

### Thermal Throttling Check

Before running benchmarks, ensure GPU is cool:

```bash
# Check temperature (should be <50C before starting)
nvidia-smi --query-gpu=temperature.gpu --format=csv,noheader

# If too hot, wait for cooldown
while [ $(nvidia-smi --query-gpu=temperature.gpu --format=csv,noheader) -gt 50 ]; do
  echo "Waiting for GPU to cool down..."
  sleep 30
done
```

---

## Correctness Verification for zkVM

Proving outputs must be verified:

```bash
# ZisK: Verify proof
cargo-zisk verify -p /tmp/proof

# Compare execution output
diff baseline_state_root.hex optimized_state_root.hex

# If proofs differ, the optimization is REJECTED
# regardless of speedup
```

---

## Reference: Common Issues

### ZisK CUDA Architecture Error

**Error:** `[CUDA] cudaMemcpyToSymbol(...) failed due to: no kernel image is available for execution`

**Fix:** Rebuild with correct CUDA architecture:
```bash
export CUDA_ARCH=sm_86  # RTX 3090
# Rebuild ZisK from source
```

### SP1 ECADD Bug

**Issue:** substrate-bn causes GasMismatch on mainnet blocks

**Workaround:** Use ark_bn254 instead (slower but correct)

### RISC0 Unstable Features

**Issue:** Keccak/BLS12-381 patches require "unstable" feature

**Workaround:** Enable with `--features unstable` (not production-ready)
