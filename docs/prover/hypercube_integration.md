# SP1 Hypercube Integration Guide for ethrex

> **Last updated:** 2026-02-02
> **Status:** Research & Planning Document

## Table of Contents

1. [Executive Summary](#executive-summary)
2. [SP1 Hypercube Overview](#sp1-hypercube-overview)
3. [Core Components Deep Dive](#core-components-deep-dive)
4. [ethrex L2 Architecture Context](#ethrex-l2-architecture-context)
5. [Integration Architecture](#integration-architecture)
6. [Step-by-Step Integration Guide](#step-by-step-integration-guide)
7. [Subblock Proving Strategy](#subblock-proving-strategy)
8. [Performance Considerations](#performance-considerations)
9. [Migration Path](#migration-path)
10. [References](#references)

---

## Executive Summary

SP1 Hypercube is Succinct's next-generation zkVM that achieves real-time Ethereum proving. It represents a fundamental architectural shift from STARK-based systems to multilinear polynomial-based proofs, delivering up to 5x performance improvements over SP1 Turbo. ([source][hypercube-announce])

**Key Integration Benefits for ethrex:**
- Sub-12 second proof generation for 99.7% of Ethereum blocks ([source][realtime-16gpu])
- 2x fewer GPUs required compared to SP1 Turbo ([source][hypercube-announce])
- Native support for proof aggregation (critical for L2 batches) ([docs][sp1-aggregation])
- Compatible with existing SP1 programs (same RISC-V target)

[hypercube-announce]: https://blog.succinct.xyz/sp1-hypercube/
[realtime-16gpu]: https://blog.succinct.xyz/real-time-proving-16-gpus/
[sp1-aggregation]: https://docs.succinct.xyz/docs/sp1/writing-programs/proof-aggregation

**Integration Scope:**
1. Upgrade SP1 backend to support Hypercube proof system
2. Implement subblock proving for parallel execution
3. Add aggregation layer for batch proofs

---

## SP1 Hypercube Overview

### What is SP1 Hypercube?

SP1 Hypercube is a complete redesign of the SP1 proof system, built entirely on **multilinear polynomials** instead of the univariate polynomials used in traditional STARKs. This architectural change enables:

- **Pay-only-for-what-you-use**: The Jagged PCS (Polynomial Commitment Scheme) eliminates wasted computation
- **Efficient recursion**: LogUp GKR protocol enables fast proof aggregation
- **No proximity gap conjectures**: First hash-based zkVM to eliminate these security assumptions

### Performance Benchmarks

> **Source:** [Real-Time Proving with 16 GPUs](https://blog.succinct.xyz/real-time-proving-16-gpus/) and [SP1 Hypercube Announcement](https://blog.succinct.xyz/sp1-hypercube/)

| Metric | SP1 Turbo | SP1 Hypercube | Improvement |
|--------|-----------|---------------|-------------|
| Blocks proven <12s | ~60% | 99.7% | 1.7x |
| Blocks proven <10s | ~40% | 95.4% | 2.4x |
| GPUs required | ~32 | ~16 | 2x fewer |
| Compute-heavy workloads | baseline | 5x faster | 5x |
| Precompile-heavy (ETH) | baseline | 2x faster | 2x |

**Hardware tested:** 16x NVIDIA RTX 5090 GPUs (954 randomly selected Ethereum L1 blocks, numbers 23,807,739 to 23,812,008)

### Architecture Comparison

> **Source:** [SP1 Hypercube Announcement](https://blog.succinct.xyz/sp1-hypercube/) — "SP1 Hypercube is built entirely on multilinear polynomials... unlike traditional STARKs which rely on univariate polynomials"

```
┌─────────────────────────────────────────────────────────────────────┐
│                      SP1 Turbo (Current)                            │
├─────────────────────────────────────────────────────────────────────┤
│  RISC-V Program → Plonky3 (STARK) → Univariate Polynomials         │
│                 → FRI-based PCS → Proof                             │
└─────────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────────┐
│                      SP1 Hypercube (New)                            │
├─────────────────────────────────────────────────────────────────────┤
│  RISC-V Program → Multilinear Constraints → Jagged PCS             │
│                 → LogUp GKR → Sumcheck → Proof                      │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Core Components Deep Dive

### 1. Jagged PCS (Polynomial Commitment Scheme)

> **Source:** [SP1 Hypercube Announcement](https://blog.succinct.xyz/sp1-hypercube/) and [Research Paper](https://github.com/succinctlabs/hypercube-verifier/blob/main/jagged-polynomial-commitments.pdf)

The Jagged PCS is the heart of Hypercube's efficiency. Traditional polynomial commitment schemes require padding all polynomials to a uniform degree, wasting computation on unused coefficients.

**Key insight:** Multilinear polynomials can be "tiled like rectangles" rather than "packed like spheres," eliminating wasted space. ([source](https://blog.succinct.xyz/sp1-hypercube/))

**How it works:**
1. Each constraint polynomial has its natural degree (no padding)
2. Commitments are computed only for actual coefficients
3. Verification checks only the relevant polynomial evaluations

**Research paper:** [`jagged-polynomial-commitments.pdf`](https://github.com/succinctlabs/hypercube-verifier/blob/main/jagged-polynomial-commitments.pdf) in the [hypercube-verifier repository](https://github.com/succinctlabs/hypercube-verifier)

### 2. LogUp GKR Protocol

LogUp GKR is a multilinear-friendly sumcheck protocol used for:
- Efficient lookup argument verification
- Memory checking (critical for zkVM execution)
- Permutation arguments

**Integration with ethrex:** This protocol handles the memory consistency checks during EVM execution, which is one of the most expensive parts of proving Ethereum blocks.

### 3. Hypercube Verifier

> **Repository:** [github.com/succinctlabs/hypercube-verifier](https://github.com/succinctlabs/hypercube-verifier)

The verifier implementation provides:

```
hypercube-verifier/
├── cli/           # Command-line interface
├── crates/        # Core verification library
├── proof.bin      # Example proof (~1.1 MB)
├── vk.bin         # Verification key (~189 bytes)
└── message.bin    # Public values (~2.9 KB)
```

**Verification API** ([README](https://github.com/succinctlabs/hypercube-verifier#readme)):
```bash
cargo run -- --proof-dir proof.bin --vk-dir vk.bin
```

### 4. Subblock Proving (rsp-subblock)

> **Repository:** [github.com/succinctlabs/rsp-subblock](https://github.com/succinctlabs/rsp-subblock) — "A proof of concept system for generating zero-knowledge proofs of EVM block execution using Reth in real time (Sub 12 seconds)"

Subblock proving enables parallel proof generation by splitting blocks by transaction:

```
┌─────────────────────────────────────────────────────────────────────┐
│                    Ethereum Block                                    │
│  ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐       │
│  │  Tx 1   │ │  Tx 2   │ │  Tx 3   │ │  Tx 4   │ │  Tx 5   │       │
│  └────┬────┘ └────┬────┘ └────┬────┘ └────┬────┘ └────┬────┘       │
└───────┼──────────┼──────────┼──────────┼──────────┼─────────────────┘
        │          │          │          │          │
        v          v          v          v          v
   ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐
   │Subblock │ │Subblock │ │Subblock │ │Subblock │ │Subblock │
   │ Proof 1 │ │ Proof 2 │ │ Proof 3 │ │ Proof 4 │ │ Proof 5 │
   └────┬────┘ └────┬────┘ └────┬────┘ └────┬────┘ └────┬────┘
        │          │          │          │          │
        └──────────┴──────────┴──────────┴──────────┘
                              │
                              v
                    ┌─────────────────┐
                    │  Aggregated     │
                    │  Block Proof    │
                    └─────────────────┘
```

**Subblock structure:**
- Each subblock contains a subset of transactions
- Parent state includes necessary state data for those transactions
- Output: new state root, logs bloom, transaction receipts
- Aggregation verifies all subblocks and ensures consistency with actual block

---

## ethrex L2 Architecture Context

### Current Prover Architecture

ethrex currently supports multiple zkVM backends via the `ProverBackend` trait:

> **Source:** [`crates/l2/prover/src/backend/mod.rs`](https://github.com/lambdaclass/ethrex/blob/main/crates/l2/prover/src/backend/mod.rs)

```rust
pub trait ProverBackend {
    type ProofOutput;
    type SerializedInput;

    fn serialize_input(&self, input: &ProgramInput) -> Result<Self::SerializedInput, BackendError>;
    fn execute(&self, input: ProgramInput) -> Result<(), BackendError>;
    fn prove(&self, input: ProgramInput, format: ProofFormat) -> Result<Self::ProofOutput, BackendError>;
    fn verify(&self, proof: &Self::ProofOutput) -> Result<(), BackendError>;
    fn to_batch_proof(&self, proof: Self::ProofOutput, format: ProofFormat) -> Result<BatchProof, BackendError>;
}
```

**Supported backends:**
| Backend | Status | Features |
|---------|--------|----------|
| SP1 | Production | GPU support, Groth16 |
| RISC0 | Production | Succinct, Groth16 |
| ZisK | Most optimized | GPU, MODEXP precompile |
| OpenVM | Experimental | - |
| Exec | Testing | No proofs |

### Current SP1 Integration

> **Source:** [`crates/l2/prover/src/backend/sp1.rs`](https://github.com/lambdaclass/ethrex/blob/main/crates/l2/prover/src/backend/sp1.rs)

```rust
pub struct Sp1Backend;

impl ProverBackend for Sp1Backend {
    type ProofOutput = Sp1ProveOutput;
    type SerializedInput = SP1Stdin;

    fn prove(&self, input: ProgramInput, format: ProofFormat) -> Result<Self::ProofOutput, BackendError> {
        let stdin = self.serialize_input(&input)?;
        let setup = self.get_setup();
        let sp1_format = Self::convert_format(format);
        let proof = setup.client.prove(&setup.pk, &stdin, sp1_format)?;
        Ok(Sp1ProveOutput::new(proof, setup.vk.clone()))
    }
}
```

### Guest Program Structure

> **Source:** [`crates/guest-program/`](https://github.com/lambdaclass/ethrex/tree/main/crates/guest-program)

**Input** ([`l2/input.rs`](https://github.com/lambdaclass/ethrex/blob/main/crates/guest-program/src/l2/input.rs)):
```rust
pub struct ProgramInput {
    pub blocks: Vec<Block>,
    pub execution_witness: ExecutionWitness,
    pub elasticity_multiplier: u64,
    pub fee_configs: Vec<FeeConfig>,
    pub blob_commitment: [u8; 48],
    pub blob_proof: [u8; 48],
}
```

**Output** ([`l2/output.rs`](https://github.com/lambdaclass/ethrex/blob/main/crates/guest-program/src/l2/output.rs)):
```rust
pub struct ProgramOutput {
    pub initial_state_hash: H256,
    pub final_state_hash: H256,
    pub l1_out_messages_merkle_root: H256,
    pub l1_in_messages_rolling_hash: H256,
    pub l2_in_message_rolling_hashes: Vec<(u64, H256)>,
    pub blob_versioned_hash: H256,
    pub last_block_hash: H256,
    pub chain_id: U256,
    pub non_privileged_count: U256,
    pub balance_diffs: Vec<BalanceDiff>,
}
```

**SP1 Entry Point** ([`bin/sp1/src/main.rs`](https://github.com/lambdaclass/ethrex/blob/main/crates/guest-program/bin/sp1/src/main.rs)):
```rust
sp1_zkvm::entrypoint!(main);

pub fn main() {
    let input = sp1_zkvm::io::read_vec();
    let input = rkyv::from_bytes::<ProgramInput, Error>(&input).unwrap();
    let output = execution_program(input).unwrap();
    sp1_zkvm::io::commit_slice(&output.encode());
}
```

---

## Integration Architecture

### Proposed Architecture with Hypercube

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           ethrex L2 Prover                                   │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ┌──────────────────┐    ┌──────────────────────────────────────────────┐   │
│  │  ProofCoordinator │───▶│              Hypercube Backend               │   │
│  └──────────────────┘    │  ┌─────────────────────────────────────────┐ │   │
│           │               │  │         Subblock Splitter              │ │   │
│           │               │  └───────────────┬─────────────────────────┘ │   │
│           │               │                  │                           │   │
│           │               │  ┌───────────────▼─────────────────────────┐ │   │
│           │               │  │         Parallel Provers                │ │   │
│           │               │  │  ┌────┐ ┌────┐ ┌────┐ ┌────┐ ┌────┐    │ │   │
│           │               │  │  │ P1 │ │ P2 │ │ P3 │ │ P4 │ │ P5 │    │ │   │
│           │               │  │  └──┬─┘ └──┬─┘ └──┬─┘ └──┬─┘ └──┬─┘    │ │   │
│           │               │  └─────┼──────┼──────┼──────┼──────┼──────┘ │   │
│           │               │        │      │      │      │      │        │   │
│           │               │  ┌─────▼──────▼──────▼──────▼──────▼──────┐ │   │
│           │               │  │            Proof Aggregator            │ │   │
│           │               │  └───────────────────┬────────────────────┘ │   │
│           │               └──────────────────────┼──────────────────────┘   │
│           │                                      │                          │
│           │               ┌──────────────────────▼──────────────────────┐   │
│           └──────────────▶│              BatchProof Output              │   │
│                           └─────────────────────────────────────────────┘   │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

### New Components Required

1. **HypercubeBackend** - New backend implementation using Hypercube API
2. **SubblockSplitter** - Splits batch blocks into transaction-based subblocks
3. **ParallelProverPool** - Manages parallel proof generation
4. **ProofAggregator** - Aggregates subblock proofs into single batch proof
5. **HypercubeGuestProgram** - Subblock-aware guest program variant

---

## Step-by-Step Integration Guide

### Phase 1: Environment Setup

#### 1.1 Prerequisites

```bash
# Install Rust toolchain (specific version for Hypercube)
rustup install nightly-2024-12-01
rustup target add riscv32im-succinct-zkvm-elf

# Install SP1 CLI (Hypercube version when available)
curl -L https://sp1.succinct.xyz | bash
sp1up --version hypercube  # or specific version

# GPU requirements (for local proving)
# - NVIDIA RTX 4090 or better (RTX 5090 recommended)
# - CUDA 12.0+
# - 24GB+ VRAM per GPU
```

#### 1.2 Update Dependencies

Update `crates/l2/prover/Cargo.toml`:

```toml
[dependencies]
# Update to Hypercube-compatible SP1 SDK
sp1-sdk = { version = "4.0", features = ["hypercube"] }  # Version TBD
sp1-prover = { version = "4.0", features = ["hypercube"] }
sp1-zkvm = { version = "4.0" }
```

### Phase 2: Backend Implementation

#### 2.1 Create Hypercube Backend Module

> **Pattern based on:** [`crates/l2/prover/src/backend/sp1.rs`](https://github.com/lambdaclass/ethrex/blob/main/crates/l2/prover/src/backend/sp1.rs) — the existing SP1 backend implementation

Create `crates/l2/prover/src/backend/hypercube.rs`:

```rust
use ethrex_guest_program::{ZKVM_SP1_PROGRAM_ELF, input::ProgramInput};
use ethrex_l2_common::prover::{BatchProof, ProofBytes, ProofCalldata, ProofFormat, ProverType};
use sp1_sdk::{
    HashableKey, Prover, SP1ProofMode, SP1ProofWithPublicValues,
    SP1ProvingKey, SP1Stdin, SP1VerifyingKey,
    hypercube::{HypercubeProver, HypercubeConfig},  // New imports
};
use std::sync::OnceLock;

use crate::backend::{BackendError, ProverBackend};

/// Hypercube prover setup data
pub struct HypercubeSetup {
    prover: HypercubeProver,
    pk: SP1ProvingKey,
    vk: SP1VerifyingKey,
}

pub static HYPERCUBE_SETUP: OnceLock<HypercubeSetup> = OnceLock::new();

/// Initialize Hypercube prover with optional GPU configuration
pub fn init_hypercube_setup(config: HypercubeConfig) -> HypercubeSetup {
    let prover = HypercubeProver::new(config);
    let (pk, vk) = prover.setup(ZKVM_SP1_PROGRAM_ELF);
    HypercubeSetup { prover, pk, vk }
}

/// Hypercube proof output
pub struct HypercubeProveOutput {
    pub proof: SP1ProofWithPublicValues,
    pub vk: SP1VerifyingKey,
}

/// Hypercube backend implementation
pub struct HypercubeBackend {
    config: HypercubeConfig,
}

impl HypercubeBackend {
    pub fn new(config: HypercubeConfig) -> Self {
        Self { config }
    }

    pub fn with_gpu_count(gpu_count: usize) -> Self {
        Self::new(HypercubeConfig {
            gpu_count,
            ..Default::default()
        })
    }

    fn get_setup(&self) -> &HypercubeSetup {
        HYPERCUBE_SETUP.get_or_init(|| init_hypercube_setup(self.config.clone()))
    }
}

impl ProverBackend for HypercubeBackend {
    type ProofOutput = HypercubeProveOutput;
    type SerializedInput = SP1Stdin;

    fn serialize_input(&self, input: &ProgramInput) -> Result<Self::SerializedInput, BackendError> {
        let mut stdin = SP1Stdin::new();
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(input)
            .map_err(BackendError::serialization)?;
        stdin.write_slice(bytes.as_slice());
        Ok(stdin)
    }

    fn execute(&self, input: ProgramInput) -> Result<(), BackendError> {
        let stdin = self.serialize_input(&input)?;
        let setup = self.get_setup();
        setup.prover
            .execute(ZKVM_SP1_PROGRAM_ELF, &stdin)
            .map_err(BackendError::execution)?;
        Ok(())
    }

    fn prove(
        &self,
        input: ProgramInput,
        format: ProofFormat,
    ) -> Result<Self::ProofOutput, BackendError> {
        let stdin = self.serialize_input(&input)?;
        let setup = self.get_setup();

        let mode = match format {
            ProofFormat::Compressed => SP1ProofMode::Compressed,
            ProofFormat::Groth16 => SP1ProofMode::Groth16,
        };

        let proof = setup.prover
            .prove(&setup.pk, &stdin, mode)
            .map_err(BackendError::proving)?;

        Ok(HypercubeProveOutput {
            proof,
            vk: setup.vk.clone(),
        })
    }

    fn verify(&self, proof: &Self::ProofOutput) -> Result<(), BackendError> {
        let setup = self.get_setup();
        setup.prover
            .verify(&proof.proof, &proof.vk)
            .map_err(BackendError::verification)?;
        Ok(())
    }

    fn to_batch_proof(
        &self,
        proof: Self::ProofOutput,
        format: ProofFormat,
    ) -> Result<BatchProof, BackendError> {
        match format {
            ProofFormat::Compressed => Ok(BatchProof::ProofBytes(ProofBytes {
                prover_type: ProverType::SP1,
                proof: bincode::serialize(&proof.proof)
                    .map_err(BackendError::batch_proof)?,
                public_values: proof.proof.public_values.to_vec(),
            })),
            ProofFormat::Groth16 => Ok(BatchProof::ProofCalldata(ProofCalldata {
                prover_type: ProverType::SP1,
                calldata: vec![Value::Bytes(proof.proof.bytes().into())],
            })),
        }
    }
}
```

#### 2.2 Register Backend

> **Extend:** [`crates/l2/prover/src/backend/mod.rs`](https://github.com/lambdaclass/ethrex/blob/main/crates/l2/prover/src/backend/mod.rs)

```rust
#[cfg(feature = "hypercube")]
pub mod hypercube;

#[cfg(feature = "hypercube")]
pub use hypercube::HypercubeBackend;

#[derive(Default, Debug, Deserialize, Serialize, Copy, Clone, ValueEnum, PartialEq)]
pub enum BackendType {
    #[default]
    Exec,
    #[cfg(feature = "sp1")]
    SP1,
    #[cfg(feature = "hypercube")]
    Hypercube,  // Add new variant
    #[cfg(feature = "risc0")]
    RISC0,
    // ...
}
```

### Phase 3: Subblock Proving Implementation

#### 3.1 Define Subblock Types

> **Inspired by:** [rsp-subblock primitives](https://github.com/succinctlabs/rsp-subblock/tree/main/crates/primitives) — adapted for ethrex's `ExecutionWitness` and `FeeConfig` structures

Create `crates/guest-program/src/l2/subblock.rs`:

```rust
use ethrex_common::types::{Block, Transaction, Receipt};
use ethrex_common::H256;
use rkyv::{Archive, Deserialize as RDeserialize, Serialize as RSerialize};
use serde::{Deserialize, Serialize};

/// Input for a single subblock proof
#[derive(Serialize, Deserialize, RDeserialize, RSerialize, Archive)]
pub struct SubblockInput {
    /// Block header (same for all subblocks of a block)
    pub block_header: BlockHeader,
    /// Subset of transactions to prove
    pub transactions: Vec<Transaction>,
    /// Transaction indices within the original block
    pub tx_indices: Vec<usize>,
    /// Parent state for these transactions
    pub parent_state: SubblockState,
    /// Fee config for this block
    pub fee_config: FeeConfig,
}

/// State data needed for subblock execution
#[derive(Serialize, Deserialize, RDeserialize, RSerialize, Archive)]
pub struct SubblockState {
    /// State root before this subblock
    pub pre_state_root: H256,
    /// Execution witness (subset relevant to these transactions)
    pub execution_witness: ExecutionWitness,
    /// Cumulative gas used before this subblock
    pub cumulative_gas_used: u64,
    /// Logs bloom before this subblock
    pub pre_logs_bloom: Bloom,
}

/// Output from a single subblock proof
#[derive(Serialize, Deserialize)]
pub struct SubblockOutput {
    /// State root after this subblock
    pub post_state_root: H256,
    /// Cumulative gas used after this subblock
    pub cumulative_gas_used: u64,
    /// Logs bloom for this subblock's transactions
    pub logs_bloom: Bloom,
    /// Receipts for this subblock's transactions
    pub receipts: Vec<Receipt>,
    /// Hash of parent subblock output (for chaining)
    pub parent_hash: H256,
}

impl SubblockOutput {
    pub fn hash(&self) -> H256 {
        keccak256(&self.encode())
    }
}
```

#### 3.2 Create Subblock Splitter

Create `crates/l2/prover/src/subblock/splitter.rs`:

```rust
use ethrex_guest_program::l2::{ProgramInput, SubblockInput};
use ethrex_common::types::Block;

/// Configuration for subblock splitting
pub struct SubblockConfig {
    /// Target number of subblocks per block
    pub target_subblocks: usize,
    /// Maximum transactions per subblock
    pub max_txs_per_subblock: usize,
    /// Maximum gas per subblock
    pub max_gas_per_subblock: u64,
}

impl Default for SubblockConfig {
    fn default() -> Self {
        Self {
            target_subblocks: 8,  // Parallel on 8 GPUs
            max_txs_per_subblock: 50,
            max_gas_per_subblock: 3_750_000,  // ~1/8 of block gas limit
        }
    }
}

/// Splits a batch of blocks into subblocks for parallel proving
pub struct SubblockSplitter {
    config: SubblockConfig,
}

impl SubblockSplitter {
    pub fn new(config: SubblockConfig) -> Self {
        Self { config }
    }

    /// Split ProgramInput into parallel-provable subblocks
    pub fn split(&self, input: ProgramInput) -> Vec<SubblockInput> {
        let mut subblocks = Vec::new();

        for (block_idx, block) in input.blocks.iter().enumerate() {
            let block_subblocks = self.split_block(
                block,
                &input.execution_witness,
                &input.fee_configs[block_idx],
            );
            subblocks.extend(block_subblocks);
        }

        subblocks
    }

    fn split_block(
        &self,
        block: &Block,
        witness: &ExecutionWitness,
        fee_config: &FeeConfig,
    ) -> Vec<SubblockInput> {
        let txs = &block.body.transactions;
        let mut subblocks = Vec::new();
        let mut current_txs = Vec::new();
        let mut current_gas = 0u64;
        let mut tx_start_idx = 0;

        for (idx, tx) in txs.iter().enumerate() {
            let tx_gas = tx.gas_limit();

            // Check if we should start a new subblock
            if !current_txs.is_empty() &&
               (current_txs.len() >= self.config.max_txs_per_subblock ||
                current_gas + tx_gas > self.config.max_gas_per_subblock) {

                // Create subblock with current transactions
                subblocks.push(self.create_subblock(
                    block,
                    current_txs.clone(),
                    tx_start_idx..idx,
                    witness,
                    fee_config,
                ));

                current_txs.clear();
                current_gas = 0;
                tx_start_idx = idx;
            }

            current_txs.push(tx.clone());
            current_gas += tx_gas;
        }

        // Don't forget the last subblock
        if !current_txs.is_empty() {
            subblocks.push(self.create_subblock(
                block,
                current_txs,
                tx_start_idx..txs.len(),
                witness,
                fee_config,
            ));
        }

        subblocks
    }

    fn create_subblock(
        &self,
        block: &Block,
        transactions: Vec<Transaction>,
        tx_indices: Range<usize>,
        witness: &ExecutionWitness,
        fee_config: &FeeConfig,
    ) -> SubblockInput {
        // Extract only the witness data needed for these transactions
        let subblock_witness = self.extract_subblock_witness(
            &transactions,
            witness,
        );

        SubblockInput {
            block_header: block.header.clone(),
            transactions,
            tx_indices: tx_indices.collect(),
            parent_state: SubblockState {
                pre_state_root: H256::zero(), // Set during proving
                execution_witness: subblock_witness,
                cumulative_gas_used: 0,
                pre_logs_bloom: Bloom::default(),
            },
            fee_config: fee_config.clone(),
        }
    }

    fn extract_subblock_witness(
        &self,
        transactions: &[Transaction],
        full_witness: &ExecutionWitness,
    ) -> ExecutionWitness {
        // Extract only the accounts and storage touched by these transactions
        // This is a simplified version - real implementation needs access analysis
        let touched_accounts: HashSet<Address> = transactions
            .iter()
            .flat_map(|tx| {
                let mut accounts = vec![tx.sender()];
                if let Some(to) = tx.to() {
                    accounts.push(to);
                }
                accounts
            })
            .collect();

        full_witness.filter_by_accounts(&touched_accounts)
    }
}
```

#### 3.3 Create Proof Aggregator

> **Pattern based on:** [SP1 Proof Aggregation Docs](https://docs.succinct.xyz/docs/sp1/writing-programs/proof-aggregation) — uses `stdin.write_proof()` for recursive verification

Create `crates/l2/prover/src/subblock/aggregator.rs`:

```rust
use sp1_sdk::{SP1Stdin, SP1ProofWithPublicValues};
use ethrex_guest_program::l2::{SubblockOutput, ProgramOutput};

/// Aggregates subblock proofs into a single batch proof
pub struct ProofAggregator {
    backend: HypercubeBackend,
    aggregator_elf: &'static [u8],  // Compiled aggregation program
}

impl ProofAggregator {
    pub fn new(backend: HypercubeBackend) -> Self {
        Self {
            backend,
            aggregator_elf: include_bytes!("../../guest-program/target/aggregator.elf"),
        }
    }

    /// Aggregate multiple subblock proofs into a single proof
    pub fn aggregate(
        &self,
        subblock_proofs: Vec<(SubblockOutput, SP1ProofWithPublicValues)>,
        batch_metadata: BatchMetadata,
    ) -> Result<SP1ProofWithPublicValues, BackendError> {
        let setup = self.backend.get_setup();
        let (agg_pk, agg_vk) = setup.prover.setup(self.aggregator_elf);

        let mut stdin = SP1Stdin::new();

        // Write all subblock proofs to stdin
        for (output, proof) in &subblock_proofs {
            // SP1's recursive proof verification
            stdin.write_proof(proof.clone(), setup.vk.clone());
            stdin.write(&output);
        }

        // Write batch metadata
        stdin.write(&batch_metadata);

        // Generate aggregation proof
        let agg_proof = setup.prover
            .prove(&agg_pk, &stdin, SP1ProofMode::Compressed)
            .map_err(BackendError::proving)?;

        Ok(agg_proof)
    }
}

/// Metadata for batch aggregation
#[derive(Serialize, Deserialize)]
pub struct BatchMetadata {
    pub batch_number: u64,
    pub initial_state_root: H256,
    pub expected_final_state_root: H256,
    pub blob_commitment: [u8; 48],
    pub blob_proof: [u8; 48],
}
```

#### 3.4 Create Aggregation Guest Program

> **API Reference:** [SP1 Proof Aggregation](https://docs.succinct.xyz/docs/sp1/writing-programs/proof-aggregation) — `verify_sp1_proof()` triggers recursive proof verification inside the zkVM

Create `crates/guest-program/bin/aggregator/src/main.rs`:

```rust
#![no_main]

use ethrex_guest_program::l2::{SubblockOutput, ProgramOutput, BatchMetadata};

sp1_zkvm::entrypoint!(main);

pub fn main() {
    // Read batch metadata
    let metadata: BatchMetadata = sp1_zkvm::io::read();

    // Read and verify all subblock proofs
    let mut subblock_outputs: Vec<SubblockOutput> = Vec::new();
    let mut prev_state_root = metadata.initial_state_root;

    // Number of subblocks is encoded in the proof stream
    let num_subblocks: usize = sp1_zkvm::io::read();

    for i in 0..num_subblocks {
        // Verify subblock proof recursively
        let vkey_hash: [u8; 32] = sp1_zkvm::io::read();
        let public_values_digest: [u8; 32] = sp1_zkvm::io::read();

        // This triggers recursive proof verification (SP1 handles proof data automatically)
        sp1_zkvm::lib::verify::verify_sp1_proof(&vkey_hash, &public_values_digest);

        // Read the subblock output
        let output: SubblockOutput = sp1_zkvm::io::read();

        // Verify state chaining
        assert_eq!(output.parent_hash, prev_state_root, "State chain broken");
        prev_state_root = output.post_state_root;

        subblock_outputs.push(output);
    }

    // Verify final state matches expected
    assert_eq!(
        prev_state_root,
        metadata.expected_final_state_root,
        "Final state mismatch"
    );

    // Compute aggregated output
    let output = compute_batch_output(&subblock_outputs, &metadata);

    // Commit public values
    sp1_zkvm::io::commit_slice(&output.encode());
}

fn compute_batch_output(
    subblocks: &[SubblockOutput],
    metadata: &BatchMetadata,
) -> ProgramOutput {
    // Aggregate receipts, compute message hashes, etc.
    // This mirrors the logic in the original execution_program
    // but operates on subblock outputs instead of re-executing

    ProgramOutput {
        initial_state_hash: metadata.initial_state_root,
        final_state_hash: subblocks.last().unwrap().post_state_root,
        // ... aggregate other fields
    }
}
```

### Phase 4: Parallel Proving Infrastructure

#### 4.1 Create Parallel Prover Pool

Create `crates/l2/prover/src/subblock/parallel.rs`:

```rust
use tokio::sync::mpsc;
use std::sync::Arc;

/// Pool of parallel provers for subblock proving
pub struct ParallelProverPool {
    workers: Vec<ProverWorker>,
    task_tx: mpsc::Sender<ProveTask>,
    result_rx: mpsc::Receiver<ProveResult>,
}

struct ProveTask {
    subblock: SubblockInput,
    task_id: usize,
}

struct ProveResult {
    task_id: usize,
    proof: Result<(SubblockOutput, SP1ProofWithPublicValues), BackendError>,
}

impl ParallelProverPool {
    /// Create a pool with specified number of workers
    pub async fn new(num_workers: usize, backend_config: HypercubeConfig) -> Self {
        let (task_tx, task_rx) = mpsc::channel(num_workers * 2);
        let (result_tx, result_rx) = mpsc::channel(num_workers * 2);
        let task_rx = Arc::new(tokio::sync::Mutex::new(task_rx));

        let mut workers = Vec::with_capacity(num_workers);

        for worker_id in 0..num_workers {
            let worker = ProverWorker::new(
                worker_id,
                Arc::clone(&task_rx),
                result_tx.clone(),
                backend_config.clone(),
            );
            workers.push(worker);
        }

        Self {
            workers,
            task_tx,
            result_rx,
        }
    }

    /// Prove all subblocks in parallel
    pub async fn prove_subblocks(
        &mut self,
        subblocks: Vec<SubblockInput>,
    ) -> Result<Vec<(SubblockOutput, SP1ProofWithPublicValues)>, BackendError> {
        let num_tasks = subblocks.len();

        // Submit all tasks
        for (idx, subblock) in subblocks.into_iter().enumerate() {
            self.task_tx.send(ProveTask {
                subblock,
                task_id: idx,
            }).await.map_err(|_| BackendError::Internal("Channel closed".into()))?;
        }

        // Collect results
        let mut results = vec![None; num_tasks];
        for _ in 0..num_tasks {
            let result = self.result_rx.recv().await
                .ok_or_else(|| BackendError::Internal("Channel closed".into()))?;

            results[result.task_id] = Some(result.proof?);
        }

        Ok(results.into_iter().map(|r| r.unwrap()).collect())
    }
}

struct ProverWorker {
    handle: tokio::task::JoinHandle<()>,
}

impl ProverWorker {
    fn new(
        worker_id: usize,
        task_rx: Arc<tokio::sync::Mutex<mpsc::Receiver<ProveTask>>>,
        result_tx: mpsc::Sender<ProveResult>,
        config: HypercubeConfig,
    ) -> Self {
        let handle = tokio::spawn(async move {
            let backend = HypercubeBackend::new(config);

            loop {
                let task = {
                    let mut rx = task_rx.lock().await;
                    match rx.recv().await {
                        Some(task) => task,
                        None => break,
                    }
                };

                let proof = backend.prove_subblock(&task.subblock);

                let _ = result_tx.send(ProveResult {
                    task_id: task.task_id,
                    proof,
                }).await;
            }
        });

        Self { handle }
    }
}
```

### Phase 5: Integration with ProofCoordinator

#### 5.1 Update Prover Configuration

> **Extend:** [`crates/l2/prover/src/config.rs`](https://github.com/lambdaclass/ethrex/blob/main/crates/l2/prover/src/config.rs)

```rust
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProverConfig {
    pub backend: BackendType,
    pub proof_coordinators: Vec<Url>,
    pub proving_time_ms: u64,

    // New Hypercube-specific config
    #[cfg(feature = "hypercube")]
    pub hypercube: HypercubeConfig,
}

#[cfg(feature = "hypercube")]
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HypercubeConfig {
    /// Number of GPUs for parallel proving
    pub gpu_count: usize,
    /// Enable subblock proving
    pub enable_subblocks: bool,
    /// Subblock configuration
    pub subblock_config: SubblockConfig,
}

impl Default for HypercubeConfig {
    fn default() -> Self {
        Self {
            gpu_count: 1,
            enable_subblocks: true,
            subblock_config: SubblockConfig::default(),
        }
    }
}
```

#### 5.2 Update Prover Main Loop

> **Extend:** [`crates/l2/prover/src/prover.rs`](https://github.com/lambdaclass/ethrex/blob/main/crates/l2/prover/src/prover.rs)

```rust
impl<B: ProverBackend> Prover<B> {
    pub async fn prove_batch(&self, input: ProverInputData) -> Result<BatchProof, ProverError> {
        #[cfg(feature = "hypercube")]
        if self.config.hypercube.enable_subblocks {
            return self.prove_batch_with_subblocks(input).await;
        }

        // Fallback to standard proving
        self.prove_batch_standard(input).await
    }

    #[cfg(feature = "hypercube")]
    async fn prove_batch_with_subblocks(
        &self,
        input: ProverInputData,
    ) -> Result<BatchProof, ProverError> {
        let program_input = ProgramInput::from(input);

        // Step 1: Split into subblocks
        let splitter = SubblockSplitter::new(self.config.hypercube.subblock_config.clone());
        let subblocks = splitter.split(program_input.clone());

        tracing::info!(
            "Split batch into {} subblocks for parallel proving",
            subblocks.len()
        );

        // Step 2: Prove subblocks in parallel
        let mut pool = ParallelProverPool::new(
            self.config.hypercube.gpu_count,
            self.config.hypercube.clone(),
        ).await;

        let subblock_proofs = pool.prove_subblocks(subblocks).await?;

        // Step 3: Aggregate proofs
        let aggregator = ProofAggregator::new(self.backend.clone());
        let batch_metadata = BatchMetadata {
            batch_number: input.batch_number,
            initial_state_root: program_input.execution_witness.initial_state_root(),
            expected_final_state_root: program_input.blocks.last()
                .map(|b| b.header.state_root)
                .unwrap_or_default(),
            blob_commitment: program_input.blob_commitment,
            blob_proof: program_input.blob_proof,
        };

        let aggregated_proof = aggregator.aggregate(subblock_proofs, batch_metadata)?;

        // Step 4: Convert to BatchProof
        self.backend.to_batch_proof(
            HypercubeProveOutput {
                proof: aggregated_proof,
                vk: self.backend.get_setup().vk.clone(),
            },
            self.format,
        )
    }
}
```

---

## Subblock Proving Strategy

### When to Use Subblock Proving

| Scenario | Recommendation |
|----------|----------------|
| Single block, <50 txs | Standard proving (simpler) |
| Single block, >50 txs | Subblock proving |
| Multi-block batch | Subblock proving |
| Real-time requirements (<12s) | Subblock proving with 8+ GPUs |

### Optimal Subblock Configuration

> **Based on:** [Real-Time Proving with 16 GPUs](https://blog.succinct.xyz/real-time-proving-16-gpus/) and [rsp-subblock](https://github.com/succinctlabs/rsp-subblock)

```rust
// For 16 GPUs, targeting <12s proving
SubblockConfig {
    target_subblocks: 16,
    max_txs_per_subblock: 20,
    max_gas_per_subblock: 1_875_000,  // 30M / 16
}

// For 8 GPUs, targeting <20s proving
SubblockConfig {
    target_subblocks: 8,
    max_txs_per_subblock: 40,
    max_gas_per_subblock: 3_750_000,
}

// For cost-optimized (fewer GPUs, longer time)
SubblockConfig {
    target_subblocks: 4,
    max_txs_per_subblock: 75,
    max_gas_per_subblock: 7_500_000,
}
```

---

## Performance Considerations

### Hardware Requirements

> **Source:** [SP1 Hypercube Announcement](https://blog.succinct.xyz/sp1-hypercube/) — "a cluster capable of real-time proving >90% of mainnet blocks with SP1 Hypercube requires ~160 4090 GPUs and can be built for ~$300-400k"

| Configuration | GPUs | Expected Performance | Estimated Cost |
|---------------|------|---------------------|----------------|
| Minimum | 4x RTX 4090 | ~30s per batch | ~$8,000 |
| Recommended | 8x RTX 4090 | ~15s per batch | ~$16,000 |
| Real-time | 16x RTX 5090 | <12s per batch | ~$40,000 |
| Production cluster | 160x RTX 4090 | Real-time >90% | ~$300-400k |

### Memory Requirements

- Per GPU: 24GB VRAM (RTX 4090) or 32GB (RTX 5090)
- System RAM: 64GB minimum, 128GB recommended
- Storage: NVMe SSD for proof artifacts

---

## Migration Path

### From SP1 Turbo to Hypercube

1. **Phase 1: Parallel Deployment**
   - Deploy Hypercube backend alongside existing SP1 backend
   - Run both in shadow mode, compare results
   - Feature flag for gradual rollout

2. **Phase 2: Validation**
   - Verify proof equivalence (same public values)
   - Compare proving times
   - Monitor for any edge cases

3. **Phase 3: Cutover**
   - Switch production traffic to Hypercube
   - Keep SP1 Turbo as fallback
   - Remove after confidence period

### Version Compatibility

| ethrex Version | SP1 Version | Hypercube Support |
|----------------|-------------|-------------------|
| Current | SP1 Turbo (v3.x) | No |
| Next | SP1 v4.x | Hypercube available |
| Future | SP1 v5.x | Hypercube default |

---

## References

### Official Succinct Documentation

| Resource | URL | Used For |
|----------|-----|----------|
| SP1 Hypercube Announcement | https://blog.succinct.xyz/sp1-hypercube/ | Architecture, performance claims, cost estimates |
| Real-Time Proving with 16 GPUs | https://blog.succinct.xyz/real-time-proving-16-gpus/ | Benchmark numbers (99.7%, 95.4%) |
| SP1 Documentation | https://docs.succinct.xyz/docs/sp1/introduction | General SP1 usage |
| SP1 Proof Aggregation | https://docs.succinct.xyz/docs/sp1/writing-programs/proof-aggregation | `verify_sp1_proof()` API, `stdin.write_proof()` |

### Repositories

| Repository | URL | Used For |
|------------|-----|----------|
| hypercube-verifier | https://github.com/succinctlabs/hypercube-verifier | Verifier implementation, Jagged PCS paper |
| rsp-subblock | https://github.com/succinctlabs/rsp-subblock | Subblock proving patterns |
| SP1 | https://github.com/succinctlabs/sp1 | Main SP1 zkVM |

### Research Papers

| Paper | Location | Topic |
|-------|----------|-------|
| Jagged Polynomial Commitments | [hypercube-verifier/jagged-polynomial-commitments.pdf](https://github.com/succinctlabs/hypercube-verifier/blob/main/jagged-polynomial-commitments.pdf) | Core PCS theory |

### ethrex Source Files

| File | Path | Contains |
|------|------|----------|
| ProverBackend trait | [`crates/l2/prover/src/backend/mod.rs`](https://github.com/lambdaclass/ethrex/blob/main/crates/l2/prover/src/backend/mod.rs) | Backend interface |
| SP1 Backend | [`crates/l2/prover/src/backend/sp1.rs`](https://github.com/lambdaclass/ethrex/blob/main/crates/l2/prover/src/backend/sp1.rs) | Reference implementation |
| Prover Config | [`crates/l2/prover/src/config.rs`](https://github.com/lambdaclass/ethrex/blob/main/crates/l2/prover/src/config.rs) | Configuration structures |
| Prover Main Loop | [`crates/l2/prover/src/prover.rs`](https://github.com/lambdaclass/ethrex/blob/main/crates/l2/prover/src/prover.rs) | Proving logic |
| ProgramInput | [`crates/guest-program/src/l2/input.rs`](https://github.com/lambdaclass/ethrex/blob/main/crates/guest-program/src/l2/input.rs) | Input structures |
| ProgramOutput | [`crates/guest-program/src/l2/output.rs`](https://github.com/lambdaclass/ethrex/blob/main/crates/guest-program/src/l2/output.rs) | Output structures |
| SP1 Entry Point | [`crates/guest-program/bin/sp1/src/main.rs`](https://github.com/lambdaclass/ethrex/blob/main/crates/guest-program/bin/sp1/src/main.rs) | Guest program entry |

### ethrex Documentation

- [Prover Overview](./prover.md)
- [Guest Program](./guest_program.md)

---

## Appendix A: Feature Flag Configuration

Add to `crates/l2/prover/Cargo.toml`:

```toml
[features]
default = ["sp1"]
sp1 = ["sp1-sdk", "sp1-prover", "sp1-zkvm"]
hypercube = ["sp1", "sp1-sdk/hypercube"]
gpu = ["sp1-sdk/cuda"]

# All features for development
full = ["sp1", "hypercube", "risc0", "zisk", "gpu"]
```

## Appendix B: Environment Variables

```bash
# GPU Configuration
export CUDA_VISIBLE_DEVICES=0,1,2,3  # Select GPUs
export SP1_GPU_COUNT=4               # Number of GPUs for Hypercube

# Debug
export RUST_LOG=ethrex_prover=debug,sp1=info
export SP1_DEV=1                     # Development mode (faster, less secure)
```

## Appendix C: Troubleshooting

### Common Issues

1. **CUDA out of memory**
   - Reduce `max_gas_per_subblock`
   - Increase `target_subblocks`
   - Use smaller batch sizes

2. **Proof aggregation fails**
   - Verify all subblock proofs are valid
   - Check state chaining is correct
   - Ensure metadata matches

3. **Performance below expected**
   - Check GPU utilization with `nvidia-smi`
   - Verify CUDA version compatibility
