#[cfg(feature = "openvm-crypto")]
pub mod openvm;
#[cfg(feature = "risc0-crypto")]
pub mod risc0;
#[cfg(any(
    feature = "sp1-crypto",
    feature = "risc0-crypto",
    feature = "zisk-crypto",
    feature = "openvm-crypto"
))]
mod shared;
#[cfg(feature = "sp1-crypto")]
pub mod sp1;
#[cfg(feature = "zisk-crypto")]
pub mod zisk;

// Re-export core crypto types so consumers don't need a direct ethrex-crypto dependency.
pub use ethrex_crypto::{Crypto, NativeCrypto};
