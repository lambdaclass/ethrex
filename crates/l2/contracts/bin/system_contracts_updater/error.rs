#[allow(clippy::enum_variant_names)]
#[derive(Debug, thiserror::Error)]
pub enum SystemContractsUpdaterError {
    #[error("Failed to deploy contract: {0}")]
    FailedToDecodeRuntimeCode(#[from] hex::FromHexError),
    #[error("Failed to serialize modified genesis: {0}")]
    FailedToSerializeModifiedGenesis(#[from] serde_json::Error),
    #[error("Failed to write modified genesis file: {0}")]
    FailedToWriteModifiedGenesisFile(#[from] std::io::Error),
    #[error("Failed to read path: {0}")]
    InvalidPath(String),
    #[error(
        "Contract bytecode not found. Make sure to compile the updater with `COMPILE_CONTRACTS` set."
    )]
    BytecodeNotFound,
}
