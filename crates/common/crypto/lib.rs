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
