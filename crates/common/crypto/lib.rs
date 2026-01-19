//! # ethrex-crypto
//!
//! Cryptographic primitives for the ethrex Ethereum client.
//!
//! This crate provides optimized implementations of hash functions and polynomial
//! commitment schemes required by the Ethereum protocol. It automatically selects
//! the best implementation for the target platform.
//!
//! ## Modules
//!
//! - [`blake2f`]: BLAKE2b compression function (EVM precompile 0x09)
//! - [`keccak`]: Keccak-256 hash function (EVM KECCAK256 opcode)
//! - [`kzg`]: KZG polynomial commitments for EIP-4844 blobs
//!
//! ## Quick Start
//!
//! ### Keccak Hashing
//!
//! ```rust
//! use ethrex_crypto::keccak::{keccak_hash, Keccak256};
//!
//! // Single-shot
//! let hash = keccak_hash(b"hello");
//!
//! // Streaming
//! let hash = Keccak256::new()
//!     .update(b"hello")
//!     .update(b" world")
//!     .finalize();
//! ```
//!
//! ### BLAKE2b Compression
//!
//! ```rust
//! use ethrex_crypto::blake2f::blake2b_f;
//!
//! let mut state = [0u64; 8];
//! let message = [0u64; 16];
//! let offset = [0u64; 2];
//! blake2b_f(12, &mut state, &message, &offset, true);
//! ```
//!
//! ### KZG Blob Verification
//!
//! ```rust,ignore
//! use ethrex_crypto::kzg::{warm_up_trusted_setup, verify_blob_kzg_proof};
//!
//! // Load trusted setup on startup
//! warm_up_trusted_setup();
//!
//! // Verify blob commitment
//! let valid = verify_blob_kzg_proof(blob, commitment, proof)?;
//! ```
//!
//! ## Feature Flags
//!
//! | Feature | Description |
//! |---------|-------------|
//! | `kzg-rs` | Pure Rust KZG (default) |
//! | `c-kzg` | C bindings to BLST (faster, full API) |
//! | `openvm-kzg` | OpenVM zkVM compatibility |
//! | `risc0` | RISC0 zkVM compatibility |
//!
//! ## Platform Support
//!
//! - **x86_64**: AVX2 assembly for BLAKE2f, assembly for Keccak
//! - **ARM64**: NEON intrinsics for BLAKE2f, assembly for Keccak
//! - **Other**: Pure Rust fallbacks (tiny-keccak for Keccak)

pub mod blake2f;
pub mod keccak;
pub mod kzg;
