use ethrex_l2::utils::eth_client::errors::EthClientError;

#[derive(Debug, thiserror::Error)]
pub enum DeployerError {
    #[error("Deployer setup error: {0} not set")]
    ConfigValueNotSet(String),
    #[error("Deployer setup parse error: {0}")]
    ParseError(String),
    #[error("Deployer dependency error: {0}")]
    DependencyError(String),
    #[error("Deployer compilation error: {0}")]
    CompilationError(String),
    #[error("Deployer EthClient error: {0}")]
    EthClientError(#[from] EthClientError),
    #[error("Deployer decoding error: {0}")]
    DecodingError(String),
}
