#[cfg(feature = "openvm")]
pub mod openvm;
#[cfg(feature = "risc0")]
pub mod risc0;
#[cfg(any(feature = "sp1", feature = "risc0", feature = "openvm"))]
mod shared;
#[cfg(feature = "sp1")]
pub mod sp1;
#[cfg(feature = "zisk")]
pub mod zisk;

// Re-export core crypto types so consumers don't need a direct ethrex-crypto dependency.
pub use ethrex_crypto::{Crypto, NativeCrypto};
