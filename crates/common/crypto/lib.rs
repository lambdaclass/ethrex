#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

pub mod blake2f;
#[cfg(feature = "blst")]
mod bls_blst;
pub mod keccak;
pub mod kzg;
pub mod native;
#[cfg(feature = "aws-lc-rs")]
mod p256_awslc;
pub mod provider;
pub use native::NativeCrypto;
pub use provider::{Crypto, CryptoError};

/// `true` when `NativeCrypto` routes `secp256r1_verify` through the native
/// aws-lc-rs backend; `false` when it falls back to the portable `p256` trait
/// default (e.g. zkVM guest builds). Differential tests assert this so they
/// fail loudly instead of silently comparing the pure-Rust backend to itself.
pub const NATIVE_P256_BACKEND: bool = cfg!(feature = "aws-lc-rs");

/// `true` when `NativeCrypto` routes BLS12-381 through the native blst backend;
/// `false` when it falls back to the portable `bls12_381` trait default (e.g.
/// zkVM guest builds). Differential tests assert this so they fail loudly
/// instead of silently comparing the pure-Rust backend to itself.
pub const NATIVE_BLS_BACKEND: bool = cfg!(feature = "blst");
