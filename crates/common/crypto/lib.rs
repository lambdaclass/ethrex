#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

pub mod blake2f;
pub mod keccak;
pub mod kzg;
#[cfg(feature = "std")]
pub mod native;
pub mod provider;
#[cfg(feature = "std")]
pub use native::NativeCrypto;
pub use provider::{Crypto, CryptoError};
