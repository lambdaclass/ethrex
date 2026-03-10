use crate::provider::Crypto;

/// Native crypto implementation using system libraries.
///
/// All method bodies live as defaults on the [`Crypto`] trait itself.
/// This struct exists so callers outside zkVM contexts have a concrete
/// type to instantiate.
#[derive(Debug)]
pub struct NativeCrypto;

impl Crypto for NativeCrypto {}
