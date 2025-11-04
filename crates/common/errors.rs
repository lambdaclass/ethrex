#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[cfg(all(not(feature = "risc0"), not(feature = "sp1")))]
    #[error("secp256k1 error: {0}")]
    Secp256k1(#[from] secp256k1::Error),
    #[cfg(any(feature = "risc0", feature = "sp1"))]
    #[error("k256 error: {0}")]
    K256(#[from] k256::ecdsa::Error),
}
