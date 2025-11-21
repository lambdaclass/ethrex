#[derive(thiserror::Error, Debug)]
pub enum EcdsaError {
    #[cfg(all(
        not(feature = "zisk"),
        not(feature = "risc0"),
        not(feature = "sp1"),
        feature = "secp256k1"
    ))]
    #[error("secp256k1 error: {0}")]
    Secp256k1(#[from] secp256k1::Error),
    #[cfg(any(
        feature = "zisk",
        feature = "risc0",
        feature = "sp1",
        not(feature = "secp256k1")
    ))]
    #[error("k256 error: {0}")]
    K256(#[from] k256::ecdsa::Error),
}
