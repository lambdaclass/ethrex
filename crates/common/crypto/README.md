# ethrex-crypto

Cryptographic primitives for the ethrex Ethereum client.

## Overview

This crate provides optimized implementations of cryptographic functions required by the Ethereum protocol, including hash functions and polynomial commitment schemes. It supports multiple backends and automatically selects optimal implementations based on the target platform.

## Features

- **Platform-optimized hashing**: Assembly implementations for x86_64 and ARM64, with pure Rust fallbacks
- **Multi-backend KZG**: Support for c-kzg, kzg-rs, and OpenVM backends
- **zkVM compatibility**: Portable implementations for RISC0 and OpenVM proving environments
- **Zero-cost abstractions**: Runtime CPU feature detection with single initialization

## Modules

### blake2f

BLAKE2b compression function implementation per [RFC 7693](https://tools.ietf.org/html/rfc7693).

```rust
use ethrex_crypto::blake2f::blake2b_f;

let mut state = [0u64; 8];
let message = [0u64; 16];
let offset = [0u64; 2];

blake2b_f(12, &mut state, &message, &offset, true);
```

Used by the EVM BLAKE2F precompile (address `0x09`).

**Platform support:**
- x86_64: AVX2 assembly (auto-detected)
- ARM64: NEON intrinsics with SHA3 extension
- Other: Pure Rust implementation

### keccak

Keccak-256 hash function used throughout Ethereum.

```rust
use ethrex_crypto::keccak::{keccak_hash, Keccak256};

// Single-shot hashing
let hash = keccak_hash(b"hello");

// Streaming (incremental) hashing
let hash = Keccak256::new()
    .update(b"hello")
    .update(b" world")
    .finalize();
```

Used by the EVM `KECCAK256` opcode and for address derivation, storage keys, and Merkle tree construction.

**Platform support:**
- x86_64/ARM64: Assembly implementation
- Other: Falls back to `tiny-keccak` crate

### kzg

KZG (Kate-Zaverucha-Goldberg) polynomial commitment scheme for EIP-4844 blob transactions.

```rust
use ethrex_crypto::kzg::{
    warm_up_trusted_setup,
    verify_blob_kzg_proof,
    verify_kzg_proof,
};

// Load trusted setup on startup (runs in background thread)
warm_up_trusted_setup();

// Verify a blob commitment
let valid = verify_blob_kzg_proof(blob, commitment, proof)?;

// Verify a point evaluation
let valid = verify_kzg_proof(commitment, z, y, proof)?;
```

**c-kzg only functions:**
- `verify_kzg_proof_batch` - Batch verification
- `blob_to_kzg_commitment_and_proof` - Generate commitment and proof from blob
- `blob_to_commitment_and_cell_proofs` - Generate commitment and cell proofs (Dencun)

## Feature Flags

| Feature | Description | Default |
|---------|-------------|---------|
| `kzg-rs` | Pure Rust KZG implementation | Yes |
| `c-kzg` | C bindings to BLST library (faster, more complete API) | No |
| `openvm-kzg` | OpenVM-compatible KZG for zkVM proving | No |
| `risc0` | RISC0 zkVM compatibility (portable c-kzg) | No |
| `openvm` | Alias for `openvm-kzg` | No |

### Backend Selection

The KZG module selects backends at compile time:

1. **c-kzg** (if enabled): Full API with batch operations
2. **kzg-rs** (default): Basic verification only
3. **openvm-kzg** (if enabled): For OpenVM proving environment

Note: `c-kzg` and `openvm-kzg` are mutually exclusive.

## Constants

### KZG Constants (EIP-4844)

| Constant | Value | Description |
|----------|-------|-------------|
| `BYTES_PER_FIELD_ELEMENT` | 32 | Size of a BLS12-381 scalar |
| `FIELD_ELEMENTS_PER_BLOB` | 4096 | Elements in a blob |
| `BYTES_PER_BLOB` | 131,072 | Total blob size (128 KiB) |
| `FIELD_ELEMENTS_PER_CELL` | 64 | Elements per cell (Dencun) |
| `BYTES_PER_CELL` | 2,048 | Cell size (2 KiB) |
| `CELLS_PER_EXT_BLOB` | 128 | Cells in extended blob |

## Error Handling

KZG operations return `Result<T, KzgError>`:

```rust
pub enum KzgError {
    CKzg(c_kzg::Error),           // c-kzg backend error
    KzgRs(kzg_rs::KzgError),      // kzg-rs backend error
    OpenvmKzg(openvm_kzg::KzgError), // OpenVM backend error
    NotSupportedWithoutCKZG(String), // Operation requires c-kzg
    Unimplemented(String),        // Operation not available
}
```

## Performance

### Blake2f
- **x86_64 (AVX2)**: ~2x faster than portable
- **ARM64 (NEON)**: ~1.5x faster than portable
- Runtime detection ensures optimal path without recompilation

### Keccak
- Assembly implementations provide significant speedup for large inputs
- Streaming API avoids memory allocation for incremental hashing

### KZG
- `warm_up_trusted_setup()` loads the 50MB trusted setup in a background thread
- c-kzg with precomputation (`KZG_PRECOMPUTE=8`) optimizes repeated verifications
- RISC0 uses `KZG_PRECOMPUTE=0` for memory-constrained proving

## Usage in ethrex

This crate is used by:
- **ethrex-levm**: EVM opcode and precompile implementations
- **ethrex-blockchain**: Block and transaction validation
- **ethrex-trie**: Merkle Patricia Trie operations
- **ethrex-prover**: zkVM guest programs
