# SP1 Hypercube Integration Guide

> **Last updated:** 2026-02-04

---

## Roadmap

### Phase 1: SP1 Version Upgrade

**Goal:** Upgrade SP1 SDK to Hypercube, keep existing guest program unchanged, measure baseline improvement.

- Update SP1 dependencies to Hypercube-compatible versions
- No changes to guest program or prover architecture
- Expected gains: ~2x faster for precompile-heavy workloads

**Deliverables:**
- [ ] Updated SP1 dependencies
- [ ] Benchmark comparison: SP1 Turbo vs Hypercube

### Phase 2: Subblock Proving

**Goal:** Implement subblock splitting and parallel proving for GPU utilization.

- Create subblock guest program that proves transaction subsets
- Implement parallel prover pool for distributing work across GPUs
- Add aggregation program to combine subblock proofs
- Subblock types and splitting logic are zkVM-agnostic (reusable by RISC0, ZisK)

**Deliverables:**
- [ ] `SubblockInput`/`SubblockOutput` types (shared across zkVMs)
- [ ] `SubblockSplitter` (reusable)
- [ ] SP1 subblock guest program
- [ ] SP1 aggregation guest program
- [ ] `ParallelProverPool` for multi-GPU proving

### Phase 3: L2 Subbatch Integration

**Goal:** Prover-side decomposition of batches into subbatches (blocks) and subblocks.

- Prover splits incoming batches into subbatches (individual blocks)
- Each subbatch can be further split into subblocks (transaction groups)
- Sequencer remains unchanged - all splitting is prover-side

**Deliverables:**
- [ ] Batch → subbatch (block) splitter in prover
- [ ] End-to-end L2 proving tests

---

## Integration Guide

### Phase 1: SP1 Version Upgrade

#### 1.1 Update Dependencies

Update SP1 dependencies in:
- `crates/l2/prover/Cargo.toml`
- `crates/guest-program/bin/sp1/Cargo.toml`

#### 1.2 Verify & Benchmark

- Build guest program and run existing tests
- Compare proving time: SP1 Turbo vs Hypercube (same input)

---

### Phase 2: Subblock Proving

#### 2.1 Subblock Types

Create `crates/guest-program/src/l2/subblock.rs`:

```rust
/// Input for a single subblock proof
pub struct SubblockInput {
    pub block_header: BlockHeader,
    pub transactions: Vec<Transaction>,
    pub tx_indices: Vec<usize>,
    pub parent_state: SubblockState,
    pub fee_config: FeeConfig,
}

/// Output from a single subblock proof
pub struct SubblockOutput {
    pub post_state_root: H256,
    pub cumulative_gas_used: u64,
    pub logs_bloom: Bloom,
    pub receipts: Vec<Receipt>,
    pub parent_hash: H256,
}
```

#### 2.2 Subblock Splitter

Create `crates/l2/prover/src/subblock/splitter.rs`:

```rust
pub struct SubblockConfig {
    pub target_subblocks: usize,
    pub max_txs_per_subblock: usize,
    pub max_gas_per_subblock: u64,
}

pub struct SubblockSplitter {
    config: SubblockConfig,
}

impl SubblockSplitter {
    pub fn split(&self, input: ProgramInput) -> Vec<SubblockInput> {
        // Split blocks into subblocks based on config
    }
}
```

#### 2.3 Aggregation Guest Program

Create `crates/guest-program/bin/aggregator/src/main.rs`:

```rust
sp1_zkvm::entrypoint!(main);

pub fn main() {
    let metadata: BatchMetadata = sp1_zkvm::io::read();
    let num_subblocks: usize = sp1_zkvm::io::read();

    for _ in 0..num_subblocks {
        let vkey_hash: [u8; 32] = sp1_zkvm::io::read();
        let public_values_digest: [u8; 32] = sp1_zkvm::io::read();

        // Recursive proof verification
        sp1_zkvm::lib::verify::verify_sp1_proof(&vkey_hash, &public_values_digest);

        let output: SubblockOutput = sp1_zkvm::io::read();
        // Verify state chaining...
    }

    // Commit aggregated output
    sp1_zkvm::io::commit_slice(&output.encode());
}
```

#### 2.4 Parallel Prover Pool

Create `crates/l2/prover/src/subblock/parallel.rs`:

```rust
pub struct ParallelProverPool {
    workers: Vec<ProverWorker>,
}

impl ParallelProverPool {
    pub async fn new(num_workers: usize) -> Self { ... }

    pub async fn prove_subblocks(
        &mut self,
        subblocks: Vec<SubblockInput>,
    ) -> Result<Vec<(SubblockOutput, SP1ProofWithPublicValues)>, BackendError> {
        // Distribute subblocks across workers, collect results
    }
}
```

---

### Phase 3: L2 Subbatch Integration

#### 3.1 Update Prover Config

```rust
pub struct HypercubeConfig {
    pub gpu_count: usize,
    pub enable_subblocks: bool,
    pub subblock_config: SubblockConfig,
}
```

#### 3.2 Update Prover Main Loop

```rust
impl Prover {
    pub async fn prove_batch(&self, input: ProverInputData) -> Result<BatchProof, ProverError> {
        if self.config.hypercube.enable_subblocks {
            // 1. Split batch into subblocks
            let subblocks = splitter.split(program_input);

            // 2. Prove subblocks in parallel
            let subblock_proofs = pool.prove_subblocks(subblocks).await?;

            // 3. Aggregate proofs
            let aggregated_proof = aggregator.aggregate(subblock_proofs, metadata)?;

            return Ok(aggregated_proof);
        }

        // Fallback to standard proving
        self.prove_batch_standard(input).await
    }
}
```

---

## References

- [SP1 Hypercube Announcement](https://blog.succinct.xyz/sp1-hypercube/)
- [Real-Time Proving with 16 GPUs](https://blog.succinct.xyz/real-time-proving-16-gpus/)
- [SP1 Proof Aggregation](https://docs.succinct.xyz/docs/sp1/writing-programs/proof-aggregation)
- [rsp-subblock](https://github.com/succinctlabs/rsp-subblock) - Reference implementation
