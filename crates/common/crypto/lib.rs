#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

pub mod blake2f;
#[cfg(feature = "blst")]
mod bls_blst;
pub mod keccak;
pub mod kzg;
pub mod native;
pub mod provider;
pub use native::NativeCrypto;
pub use provider::{Crypto, CryptoError};

/// `true` when `NativeCrypto` routes BLS12-381 through the native blst backend;
/// `false` when it falls back to the portable `bls12_381` trait default (e.g.
/// zkVM guest builds). Differential tests assert this so they fail loudly
/// instead of silently comparing the pure-Rust backend to itself.
pub const NATIVE_BLS_BACKEND: bool = cfg!(feature = "blst");
