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

use ethrex_crypto::Crypto;
use std::sync::Arc;

pub fn get_crypto_provider() -> Arc<dyn Crypto> {
    #[cfg(feature = "sp1")]
    {
        return Arc::new(sp1::Sp1Crypto);
    }
    #[cfg(feature = "risc0")]
    {
        return Arc::new(risc0::Risc0Crypto);
    }
    #[cfg(feature = "zisk")]
    {
        return Arc::new(zisk::ZiskCrypto);
    }
    #[cfg(feature = "openvm")]
    {
        return Arc::new(openvm::OpenVmCrypto);
    }
    // When no zkVM feature is active (e.g. exec prover backend or workspace check),
    // fall back to native crypto.
    #[cfg(not(any(
        feature = "sp1",
        feature = "risc0",
        feature = "zisk",
        feature = "openvm"
    )))]
    {
        Arc::new(ethrex_crypto::NativeCrypto)
    }
}
