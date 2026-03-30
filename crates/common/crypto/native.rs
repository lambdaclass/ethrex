#[cfg(feature = "std")]
use crate::provider::Crypto;

/// A crypto provider that uses the native implementations
/// (default trait methods in `Crypto`). Only available with the `std` feature.
#[cfg(feature = "std")]
#[derive(Debug)]
pub struct NativeCrypto;

#[cfg(feature = "std")]
impl Crypto for NativeCrypto {}
