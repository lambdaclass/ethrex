// FIXME
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[cfg(feature = "secp256k1")]
    #[error("")]
    Secp256k1(#[from] secp256k1::Error),
    #[cfg(feature = "k256")]
    #[error("")]
    K256,
}
