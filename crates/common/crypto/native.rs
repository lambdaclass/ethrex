use crate::provider::Crypto;

/// A crypto provider that uses the native implementations
/// (default trait methods in `Crypto`). Only available with the `std` feature.
#[derive(Debug)]
pub struct NativeCrypto;

impl Crypto for NativeCrypto {}
