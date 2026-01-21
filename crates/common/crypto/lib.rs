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
//! The [`blake2f::blake2b_f`] function implements the BLAKE2b compression function
//! (EVM precompile 0x09). It takes a mutable state array, message block, offset
//! counter, and finalization flag.
//!
//! ### KZG Trusted Setup
//!
//! ```rust
//! use ethrex_crypto::kzg::warm_up_trusted_setup;
//!
//! // Load trusted setup on startup (runs in background thread)
//! warm_up_trusted_setup();
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
