use ethrex_l2_sdk::DeployError;
use ethrex_rpc::clients::{EthClientError, eth::errors::CalldataEncodeError};

#[derive(Debug, thiserror::Error)]
pub enum DeployerError {
    #[error("The path is not a valid utf-8 string")]
    FailedToGetStringFromPath,
    #[error("Deployer setup error: {0} not set")]
    ConfigValueNotSet(String),
    #[error("Deployer EthClient error: {0}")]
    EthClientError(#[from] EthClientError),
    #[error("Deployer decoding error: {0}")]
    DecodingError(String),
    #[error("Failed to encode calldata: {0}")]
    CalldataEncodeError(#[from] CalldataEncodeError),
    #[error("Failed to deploy contract: {0}")]
    FailedToDeployContract(#[from] DeployError),
    #[error("Deployment subtask failed: {0}")]
    DeploymentSubtaskFailed(String),
    #[error("Internal error: {0}")]
    InternalError(String),
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
    #[error(
        "Contract bytecode not found. Make sure to compile the deployer with `COMPILE_CONTRACTS` set."
    )]
    BytecodeNotFound,
}
