//! Crypto provider selection for the guest: zisk/sp1 route through the
//! zkvm-standards `zkvm-interface` syscalls, openvm through its guest libraries.
//! This keeps guest crypto decoupled from per-SDK patched-crate stacks: the SDK
//! versions ere pins, listed in `.github/scripts/zkvm-version.sh`, are not ones the
//! ethrex first-party providers in `ethrex_guest_program::crypto` target.

#[cfg(feature = "openvm")]
mod openvm;
#[cfg(feature = "zkvm-interface")]
mod zkvm_interface;

use std::sync::Arc;

use ethrex_crypto::Crypto;

/// Returns the [`Crypto`] implementation for the active zkVM feature.
#[allow(unreachable_code)]
pub fn crypto() -> Arc<dyn Crypto> {
    #[cfg(feature = "openvm")]
    return openvm::crypto();
    #[cfg(feature = "zkvm-interface")]
    return zkvm_interface::crypto();
    #[cfg(not(any(feature = "openvm", feature = "zkvm-interface")))]
    return Arc::new(ethrex_crypto::NativeCrypto);
}
