use crate::provider::Crypto;

/// Native crypto implementation using system libraries.
///
/// All method bodies live as defaults on the [`Crypto`] trait itself. On the
/// host the BLS12-381 (EIP-2537) defaults route through the assembly-optimized
/// `blst` backend (the `blst` feature, default-on); zkVM guest builds compile
/// `blst` out and use their own `Crypto` providers instead of this type. This
/// struct exists so callers outside zkVM contexts have a concrete type to
/// instantiate.
#[derive(Debug)]
pub struct NativeCrypto;

impl Crypto for NativeCrypto {}
