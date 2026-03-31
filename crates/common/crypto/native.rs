use crate::provider::Crypto;

/// A crypto provider that uses the native implementations
/// (default trait methods in `Crypto`).
#[derive(Debug)]
pub struct NativeCrypto;

impl Crypto for NativeCrypto {}
