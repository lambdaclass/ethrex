#[cfg(feature = "airbender")]
#[cfg(target_arch = "riscv32")]
#[path = "airbender.rs"]
pub mod airbender;

#[cfg(feature = "airbender")]
#[cfg(not(target_arch = "riscv32"))]
pub mod airbender {
    /// Stub for host-side compilation. The real implementation uses
    /// `airbender-crypto` which requires nightly + riscv32 target.
    #[derive(Debug)]
    pub struct AirbenderCrypto;
}
#[cfg(feature = "openvm")]
pub mod openvm;
#[cfg(feature = "risc0")]
pub mod risc0;
#[cfg(any(
    feature = "sp1",
    feature = "risc0",
    feature = "zisk",
    feature = "openvm"
))]
mod shared;
#[cfg(feature = "sp1")]
pub mod sp1;
#[cfg(feature = "zisk")]
pub mod zisk;

// Re-export core crypto types so consumers don't need a direct ethrex-crypto dependency.
pub use ethrex_crypto::{Crypto, NativeCrypto};
