use ethrex_l2_sdk::ContractCompilationError;

#[allow(clippy::enum_variant_names)]
#[derive(Debug, thiserror::Error)]
pub enum SystemContractsUpdaterError {
    #[error("Failed to compile contract: {0}")]
    FailedToCompileContract(#[from] ContractCompilationError),
    #[error("Failed to deploy contract: {0}")]
    FailedToDecodeRuntimeCode(#[from] hex::FromHexError),
    #[error("Failed to serialize modified genesis: {0}")]
    FailedToSerializeModifiedGenesis(#[from] serde_json::Error),
    #[error("Failed to write modified genesis file: {0}")]
    FailedToWriteModifiedGenesisFile(#[from] std::io::Error),
    #[error("Failed to download dependencies: {0}")]
    FailedToDownloadDependencies(String),
    #[error("Failed to read path: {0}")]
    InvalidPath(String),
}
