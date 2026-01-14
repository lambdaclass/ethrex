# Ethrex L2 Integration with Aligned Layer

This document provides a comprehensive technical overview of how ethrex L2 integrates with Aligned Layer for proof aggregation and verification.

## Table of Contents

1. [Overview](#overview)
2. [What is Aligned Layer?](#what-is-aligned-layer)
3. [Architecture](#architecture)
4. [Component Details](#component-details)
5. [Proof Workflow](#proof-workflow)
6. [Smart Contract Integration](#smart-contract-integration)
7. [Configuration](#configuration)
8. [Deployment Guide](#deployment-guide)
9. [Behavioral Differences](#behavioral-differences)

---

## Overview

Ethrex L2 supports two modes of proof verification:

1. **Standard Mode**: Proofs are verified directly on L1 via smart contract verifiers (SP1Verifier, RISC0Verifier, TDXVerifier)
2. **Aligned Mode**: Proofs are sent to Aligned Layer for aggregation, then verified on L1 via the `AlignedProofAggregatorService` contract

Aligned mode offers significant cost savings by aggregating multiple proofs before on-chain verification, reducing the gas cost per proof verification.

### Key Benefits of Aligned Mode

- **Lower verification costs**: Proof aggregation amortizes verification costs across multiple proofs
- **Batch verification**: Multiple batches can be verified in a single L1 transaction
- **Compressed proofs**: Uses STARK compressed format instead of Groth16, optimized for aggregation

---

## What is Aligned Layer?

[Aligned Layer](https://docs.alignedlayer.com/) is a proof aggregation and verification infrastructure for Ethereum. It provides:

- **Proof Aggregation Service**: Collects proofs from multiple sources and aggregates them
- **Batcher**: Receives individual proofs and batches them for aggregation
- **On-chain Verification**: Verifies aggregated proofs via the `AlignedProofAggregatorService` contract
- **SDK**: Client libraries for submitting proofs and checking verification status

### Supported Proving Systems

Ethrex L2 supports the following proving systems with Aligned:

| Prover Type | Aligned ProvingSystemId | Notes |
|-------------|------------------------|-------|
| SP1 | `ProvingSystemId::SP1` | Compressed STARK format |
| RISC0 | `ProvingSystemId::Risc0` | Compressed STARK format |

---

## Architecture

### High-Level System Flow

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              ETHREX L2 NODE                                 │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  ┌──────────────┐    ┌──────────────────┐    ┌──────────────────┐           │
│  │    Prover    │───▶│ ProofCoordinator │───▶│  L1ProofSender   │           │
│  │   (SP1/R0)   │    │    (TCP Server)  │    │                  │           │
│  └──────────────┘    └──────────────────┘    └────────┬─────────┘           │
│         │                     │                       │                     │
│         │              ┌──────▼──────┐                │                     │
│         │              │RollupStorage│                │                     │
│         │              │   (Proofs)  │◀───────────────┤                     │
│         │              └─────────────┘                │                     │
│         │                                             │                     │
│         ▼                                             ▼                     │
│  ┌──────────────┐                          ┌──────────────────┐             │
│  │  Compressed  │                          │  Aligned Batcher │             │
│  │    Proof     │                          │   (WebSocket)    │             │
│  └──────────────┘                          └────────┬─────────┘             │
│                                                     │                       │
└─────────────────────────────────────────────────────┼───────────────────────┘
                                                      │
                                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                           ALIGNED LAYER                                     │
├─────────────────────────────────────────────────────────────────────────────┤
│  ┌──────────────────┐    ┌──────────────────┐    ┌──────────────────┐       │
│  │  Proof Batcher   │───▶│ Proof Aggregator │───▶│  L1 Settlement   │       │
│  │                  │    │   (SP1/RISC0)    │    │                  │       │
│  └──────────────────┘    └──────────────────┘    └──────────────────┘       │
└─────────────────────────────────────────────────────────────────────────────┘
                                                      │
                                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                              ETHEREUM L1                                    │
├─────────────────────────────────────────────────────────────────────────────┤
│  ┌──────────────────────────────┐    ┌─────────────────────────────────┐    │
│  │ AlignedProofAggregatorService│◀───│      OnChainProposer            │    │
│  │   (Merkle proof validation)  │    │  (verifyBatchesAligned())       │    │
│  └──────────────────────────────┘    └─────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Component Interactions

#### Proof Sender Flow (Aligned Mode)

```
┌─────────────────┐
│  proof_sender   │
└────────┬────────┘
         │
    ┌────┴────┐
    │         │
    ▼         ▼
┌────────┐ ┌─────────────────┐
│rollup_ │ │    Aligned      │
│storage │ │    Batcher      │
└────────┘ └─────────────────┘
     │              │
     │              │
     └──────┬───────┘
            │
   lastSentProof / send_proof_to_aligned
```

#### Proof Verifier Flow (Aligned Mode)

```
┌──────────────────┐
│  proof_verifier  │
└────────┬─────────┘
         │
    ┌────┴────────────────────────┐
    │                             │
    ▼                             ▼
┌────────────────┐      ┌─────────────────┐
│OnChainProposer │      │AlignedProof     │
│lastVerifiedBatch│     │AggregatorService│
└───────┬────────┘      └────────┬────────┘
        │                        │
        │  verifyBatchesAligned  │
        └────────────────────────┘
                    │
                    ▼
           verifyProofInclusion
```

---

## Component Details

### 1. L1ProofSender (`l1_proof_sender.rs`)

The `L1ProofSender` handles submitting proofs to Aligned Layer.

**Key Responsibilities**:
- Monitors for completed proofs in the rollup store
- Sends compressed proofs to the Aligned Batcher via WebSocket
- Tracks the last sent batch proof number
- Handles nonce management for the Aligned batcher

**Aligned-Specific Logic**:

```rust
async fn send_proof_to_aligned(
    &self,
    batch_number: u64,
    batch_proofs: impl IntoIterator<Item = &BatchProof>,
) -> Result<(), ProofSenderError> {
    // Estimate fee from Aligned
    let fee_estimation = Self::estimate_fee(self).await?;

    // Get nonce from Aligned batcher
    let nonce = get_nonce_from_batcher(self.network.clone(), self.signer.address()).await?;

    for batch_proof in batch_proofs {
        // Build verification data for Aligned
        let verification_data = VerificationData {
            proving_system: match prover_type {
                ProverType::RISC0 => ProvingSystemId::Risc0,
                ProverType::SP1 => ProvingSystemId::SP1,
            },
            proof: batch_proof.compressed(),
            proof_generator_addr: self.signer.address(),
            vm_program_code: Some(vm_program_code),  // ELF or VK
            pub_input: Some(batch_proof.public_values()),
            verification_key: None,
        };

        // Submit to Aligned batcher
        submit(self.network.clone(), &verification_data, fee_estimation, wallet, nonce).await?;
    }
}
```

**Configuration** (`AlignedConfig`):

| Field | Description |
|-------|-------------|
| `aligned_mode` | Enable/disable Aligned integration |
| `network` | Aligned network (devnet, testnet, mainnet) |
| `fee_estimate` | Fee estimation type ("instant" or "default") |
| `beacon_urls` | Beacon client URLs for blob verification |
| `aligned_verifier_interval_ms` | Polling interval for verification checks |

### 2. L1ProofVerifier (`l1_proof_verifier.rs`)

The `L1ProofVerifier` monitors Aligned Layer for aggregated proofs and triggers on-chain verification.

**Key Responsibilities**:
- Polls Aligned Layer to check if proofs have been aggregated
- Collects Merkle proofs of inclusion for verified proofs
- Batches multiple verified proofs into a single L1 transaction
- Calls `verifyBatchesAligned()` on the OnChainProposer contract

**Verification Flow**:

```rust
async fn verify_proofs_aggregation(&self, first_batch_number: u64) -> Result<Option<H256>> {
    let mut sp1_merkle_proofs_list = Vec::new();
    let mut risc0_merkle_proofs_list = Vec::new();

    // For each consecutive batch
    loop {
        for (prover_type, proof) in proofs_for_batch {
            // Build verification data
            let verification_data = match prover_type {
                ProverType::SP1 => AggregationModeVerificationData::SP1 {
                    vk: self.sp1_vk,
                    public_inputs: proof.public_values(),
                },
                ProverType::RISC0 => AggregationModeVerificationData::Risc0 {
                    image_id: self.risc0_vk,
                    public_inputs: proof.public_values(),
                },
            };

            // Check if proof was aggregated by Aligned
            if let Some((merkle_root, merkle_path)) =
                self.check_proof_aggregation(verification_data).await?
            {
                aggregated_proofs.insert(prover_type, merkle_path);
            }
        }

        // Collect merkle proofs for this batch
        sp1_merkle_proofs_list.push(sp1_merkle_proof);
        risc0_merkle_proofs_list.push(risc0_merkle_proof);
    }

    // Send single transaction to verify all batches
    let calldata = encode_calldata(
        "verifyBatchesAligned(uint256,uint256,bytes32[][],bytes32[][])",
        &[first_batch, last_batch, sp1_proofs, risc0_proofs]
    );

    send_verify_tx(calldata, target_address).await
}
```

### 3. Prover Modification

In Aligned mode, the prover generates **Compressed** proofs instead of **Groth16** proofs.

**Proof Format Selection**:

```rust
pub enum ProofFormat {
    /// Groth16 - EVM-friendly, for direct on-chain verification
    Groth16,
    /// Compressed STARK - For Aligned Layer aggregation
    Compressed,
}
```

**BatchProof Types**:

```rust
pub enum BatchProof {
    /// For direct on-chain verification (Standard mode)
    ProofCalldata(ProofCalldata),
    /// For Aligned Layer submission (Aligned mode)
    ProofBytes(ProofBytes),
}

pub struct ProofBytes {
    pub prover_type: ProverType,
    pub proof: Vec<u8>,           // Compressed STARK proof
    pub public_values: Vec<u8>,   // Public inputs
}
```

---

## Smart Contract Integration

### OnChainProposer Contract

The `OnChainProposer.sol` contract supports both verification modes:

**State Variables**:

```solidity
/// True if verification is done through Aligned Layer
bool public ALIGNED_MODE;

/// Address of the AlignedProofAggregatorService contract
address public ALIGNEDPROOFAGGREGATOR;

/// Verification keys per git commit hash and verifier type
mapping(bytes32 commitHash => mapping(uint8 verifierId => bytes32 vk))
    public verificationKeys;
```

**Standard Verification** (`verifyBatch`):

```solidity
function verifyBatch(
    uint256 batchNumber,
    bytes memory risc0BlockProof,
    bytes memory sp1ProofBytes,
    bytes memory tdxSignature
) external onlyOwner whenNotPaused {
    require(!ALIGNED_MODE, "008");  // Use verifyBatchesAligned instead

    // Verify proofs directly via verifier contracts
    if (REQUIRE_SP1_PROOF) {
        ISP1Verifier(SP1_VERIFIER_ADDRESS).verifyProof(sp1Vk, publicInputs, sp1ProofBytes);
    }
    if (REQUIRE_RISC0_PROOF) {
        IRiscZeroVerifier(RISC0_VERIFIER_ADDRESS).verify(risc0BlockProof, risc0Vk, sha256(publicInputs));
    }
}
```

**Aligned Verification** (`verifyBatchesAligned`):

```solidity
function verifyBatchesAligned(
    uint256 firstBatchNumber,
    uint256 lastBatchNumber,
    bytes32[][] calldata sp1MerkleProofsList,
    bytes32[][] calldata risc0MerkleProofsList
) external onlyOwner whenNotPaused {
    require(ALIGNED_MODE, "00h");  // Use verifyBatch instead

    for (uint256 i = 0; i < batchesToVerify; i++) {
        bytes memory publicInputs = _getPublicInputsFromCommitment(batchNumber);

        if (REQUIRE_SP1_PROOF) {
            _verifyProofInclusionAligned(
                sp1MerkleProofsList[i],
                verificationKeys[commitHash][SP1_VERIFIER_ID],
                publicInputs
            );
        }

        if (REQUIRE_RISC0_PROOF) {
            _verifyProofInclusionAligned(
                risc0MerkleProofsList[i],
                verificationKeys[commitHash][RISC0_VERIFIER_ID],
                publicInputs
            );
        }
    }
}
```

**Aligned Proof Inclusion Verification**:

```solidity
function _verifyProofInclusionAligned(
    bytes32[] calldata merkleProofsList,
    bytes32 verificationKey,
    bytes memory publicInputsList
) internal view {
    bytes memory callData = abi.encodeWithSignature(
        "verifyProofInclusion(bytes32[],bytes32,bytes)",
        merkleProofsList,
        verificationKey,
        publicInputsList
    );

    (bool callResult, bytes memory response) = ALIGNEDPROOFAGGREGATOR.staticcall(callData);
    require(callResult, "00y");  // Call to ALIGNEDPROOFAGGREGATOR failed

    bool proofVerified = abi.decode(response, (bool));
    require(proofVerified, "00z");  // Aligned proof verification failed
}
```

### Public Inputs Structure

The public inputs for proof verification are reconstructed from batch commitments:

```
Fixed-size fields (256 bytes):
├── bytes 0-32:    Initial state root (from last verified batch)
├── bytes 32-64:   Final state root (from current batch)
├── bytes 64-96:   Withdrawals merkle root
├── bytes 96-128:  Processed privileged transactions rolling hash
├── bytes 128-160: Blob KZG versioned hash
├── bytes 160-192: Last block hash
├── bytes 192-224: Chain ID
└── bytes 224-256: Non-privileged transactions count

Variable-size fields:
├── For each balance diff:
│   ├── Chain ID (32 bytes)
│   ├── Value (32 bytes)
│   └── Asset diffs + Message hashes
└── For each L2 message rolling hash:
    ├── Chain ID (32 bytes)
    └── Rolling hash (32 bytes)
```

---

## Configuration

### Sequencer Configuration

```rust
pub struct AlignedConfig {
    /// Enable Aligned mode
    pub aligned_mode: bool,

    /// Interval (ms) between verification checks
    pub aligned_verifier_interval_ms: u64,

    /// Beacon client URLs for blob verification
    pub beacon_urls: Vec<Url>,

    /// Aligned network (devnet, testnet, mainnet)
    pub network: Network,

    /// Fee estimation type ("instant" or "default")
    pub fee_estimate: String,
}
```

### CLI Flags

| Flag | Description |
|------|-------------|
| `--aligned` | Enable Aligned mode |
| `--aligned-network` | Network for Aligned SDK (devnet/testnet/mainnet) |
| `--aligned.beacon-url` | Beacon client URL supporting `/eth/v1/beacon/blobs` |

### Environment Variables

| Variable | Description |
|----------|-------------|
| `ETHREX_ALIGNED_MODE` | Enable Aligned mode |
| `ETHREX_ALIGNED_BEACON_URL` | Beacon client URL |
| `ETHREX_ALIGNED_NETWORK` | Aligned network |
| `ETHREX_L2_ALIGNED` | Enable Aligned during deployment |
| `ETHREX_DEPLOYER_ALIGNED_AGGREGATOR_ADDRESS` | Address of `AlignedProofAggregatorService` |

---

## Deployment Guide

### Prerequisites

1. **Prover ELF/VK**: Generate the prover program and verification key:
   ```bash
   make -C crates/l2 build-prover-<sp1/risc0>  # Optional: GPU=true
   ```

2. **Aligned Environment**: Ensure Aligned Layer infrastructure is running on your target network

3. **Beacon Client**: Access to a beacon client supporting `/eth/v1/beacon/blobs`

### Step 1: Deploy L1 Contracts

```bash
COMPILE_CONTRACTS=true \
ETHREX_L2_ALIGNED=true \
ETHREX_DEPLOYER_ALIGNED_AGGREGATOR_ADDRESS=<ALIGNED_AGGREGATOR_ADDRESS> \
ETHREX_L2_SP1=true \
ethrex l2 deploy \
  --eth-rpc-url <ETH_RPC_URL> \
  --private-key <YOUR_PRIVATE_KEY> \
  --on-chain-proposer-owner <OWNER_ADDRESS> \
  --bridge-owner <BRIDGE_OWNER_ADDRESS> \
  --genesis-l2-path fixtures/genesis/l2.json \
  --proof-sender.l1-address <PROOF_SENDER_ADDRESS>
```

### Step 2: Fund Aligned Batcher

Deposit funds to the `AlignedBatcherPaymentService` contract:

```bash
aligned deposit-to-batcher \
  --network <NETWORK> \
  --private_key <PROOF_SENDER_PRIVATE_KEY> \
  --rpc_url <RPC_URL> \
  --amount <DEPOSIT_AMOUNT>
```

### Step 3: Start L2 Node

```bash
ethrex l2 \
  --network fixtures/genesis/l2.json \
  --l1.bridge-address <BRIDGE_ADDRESS> \
  --l1.on-chain-proposer-address <ON_CHAIN_PROPOSER_ADDRESS> \
  --eth.rpc-url <ETH_RPC_URL> \
  --aligned \
  --aligned-network <ALIGNED_NETWORK> \
  --aligned.beacon-url <BEACON_URL> \
  --block-producer.coinbase-address <COINBASE> \
  --committer.l1-private-key <COMMITTER_PK> \
  --proof-coordinator.l1-private-key <PROOF_COORDINATOR_PK> \
  --datadir ethrex_l2
```

### Step 4: Start Prover(s)

```bash
make -C crates/l2 init-prover-<sp1/risc0>  # Optional: GPU=true
```

### Step 5: Trigger Proof Aggregation

After proofs are submitted, trigger aggregation:

```bash
make -C aligned_layer proof_aggregator_start AGGREGATOR=<sp1/risc0>
# Or with GPU:
make -C aligned_layer proof_aggregator_start_gpu AGGREGATOR=<sp1/risc0>
```

---

## Behavioral Differences

### Standard Mode vs Aligned Mode

| Aspect | Standard Mode | Aligned Mode |
|--------|---------------|--------------|
| **Proof Format** | Groth16 (EVM-friendly) | Compressed STARK |
| **Submission Target** | OnChainProposer contract | Aligned Batcher (WebSocket) |
| **Verification Method** | `verifyBatch()` | `verifyBatchesAligned()` |
| **Verifier Contract** | SP1Verifier/RISC0Verifier | AlignedProofAggregatorService |
| **Batch Verification** | One batch per tx | Multiple batches per tx |
| **Gas Cost** | Higher (per-proof verification) | Lower (amortized via aggregation) |
| **Additional Component** | None | L1ProofVerifier process |
| **Proof Tracking** | Via rollup store | Via Aligned SDK |

### Prover Differences

**Standard Mode**:
- Generates Groth16 proof (calldata format)
- Proof sent directly to `OnChainProposer.verifyBatch()`

**Aligned Mode**:
- Generates Compressed STARK proof (bytes format)
- Proof submitted to Aligned Batcher via SDK
- Must wait for Aligned aggregation before on-chain verification

### Verification Flow Differences

**Standard Mode**:
```
Prover → ProofCoordinator → L1ProofSender → OnChainProposer.verifyBatch()
                                                    │
                                                    ▼
                                          SP1Verifier/RISC0Verifier
```

**Aligned Mode**:
```
Prover → ProofCoordinator → L1ProofSender → Aligned Batcher
                                                    │
                                                    ▼
                                          Aligned Aggregation
                                                    │
                                                    ▼
L1ProofVerifier ← check_proof_verification ← AlignedProofAggregatorService
        │
        ▼
OnChainProposer.verifyBatchesAligned()
        │
        ▼
AlignedProofAggregatorService.verifyProofInclusion()
```

---

## Error Handling

### Proof Sender Errors

| Error | Description | Recovery |
|-------|-------------|----------|
| `AlignedGetNonceError` | Failed to get nonce from batcher | Retry with backoff |
| `AlignedFeeEstimateError` | Fee estimation failed | Retry all RPC URLs |
| `AlignedWrongProofFormat` | Proof is not compressed | Re-generate proof in Aligned mode |
| `InvalidProof` | Aligned rejected the proof | Delete proof, regenerate |

### Proof Verifier Errors

| Error | Description | Recovery |
|-------|-------------|----------|
| `MismatchedPublicInputs` | Proofs have different public inputs | Investigation required |
| `UnsupportedProverType` | Prover type not supported by Aligned | Use SP1 or RISC0 |
| `BeaconClient` | Beacon URL failed | Try next beacon URL |
| `EthereumProviderError` | RPC URL failed | Try next RPC URL |

---

## Monitoring

### Key Metrics

- `batch_verification_gas`: Gas used per batch verification
- `latest_sent_batch_proof`: Last batch proof submitted to Aligned
- `last_verified_batch`: Last batch verified on L1

### Log Messages

**Proof Sender**:
```
INFO ethrex_l2::sequencer::l1_proof_sender: Sending batch proof(s) to Aligned Layer batch_number=1
INFO ethrex_l2::sequencer::l1_proof_sender: Submitted proof to Aligned prover_type=SP1 batch_number=1
```

**Proof Verifier**:
```
INFO ethrex_l2::sequencer::l1_proof_verifier: Proof aggregated by Aligned batch_number=1 merkle_root=0x... commitment=0x...
INFO ethrex_l2::sequencer::l1_proof_verifier: Batches verified in OnChainProposer, with transaction hash 0x...
```

---

## References

- [Aligned Layer Documentation](https://docs.alignedlayer.com/)
- [Aligned SDK API Reference](https://docs.alignedlayer.com/guides/1.2_sdk_api_reference)
- [Aligned Contract Addresses](https://docs.alignedlayer.com/guides/7_contract_addresses)
- [ethrex L2 Deployment Guide](./overview.md)
- [ethrex Prover Documentation](../architecture/prover.md)
