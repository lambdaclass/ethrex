//! Global injectable keccak via `OnceLock`.
//!
//! Guest programs call [`init_global_crypto`] at startup to install a
//! zkVM-accelerated [`Crypto`] provider.  Native code never needs to
//! call init — [`global_keccak`] falls back to the native CRYPTOGAMS
//! implementation automatically via `get_or_init`.
//!
//! This avoids threading a `&dyn Crypto` through every function that
//! needs keccak, keeping all existing signatures unchanged.

use std::sync::{Arc, OnceLock};

use ethereum_types::H256;

use crate::keccak::keccak_hash;
use crate::provider::Crypto;

/// The global crypto provider. `None` means "use native default".
static GLOBAL_CRYPTO: OnceLock<Arc<dyn Crypto>> = OnceLock::new();

/// Install a [`Crypto`] provider for the global keccak functions.
///
/// Must be called **once**, before any code calls [`global_keccak`].
/// Typically called at the top of a zkVM guest program's `main`.
///
/// # Panics
///
/// Panics if called more than once (the provider is already set).
pub fn init_global_crypto(crypto: Arc<dyn Crypto>) {
    GLOBAL_CRYPTO
        .set(crypto)
        .expect("init_global_crypto called more than once");
}

/// Keccak-256 returning raw bytes.
///
/// If a global [`Crypto`] provider was installed via
/// [`init_global_crypto`], delegates to its `keccak256` method.
/// Otherwise falls back to the native CRYPTOGAMS/tiny-keccak
/// implementation, so native code and tests work without any setup.
#[inline]
pub fn global_keccak(data: impl AsRef<[u8]>) -> [u8; 32] {
    let data = data.as_ref();
    match GLOBAL_CRYPTO.get() {
        Some(crypto) => crypto.keccak256(data),
        None => keccak_hash(data),
    }
}

/// Keccak-256 returning `H256`. Convenience wrapper around
/// [`global_keccak`].
#[inline]
pub fn global_keccak_hash(data: impl AsRef<[u8]>) -> H256 {
    H256(global_keccak(data.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_matches_native() {
        let data = b"hello world";
        assert_eq!(global_keccak(data), keccak_hash(data));
    }

    #[test]
    fn h256_wrapper_matches() {
        let data = b"test data";
        assert_eq!(global_keccak_hash(data), H256(keccak_hash(data)));
    }
}
