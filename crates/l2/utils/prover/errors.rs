#[derive(Debug, thiserror::Error)]
pub enum SaveStateError {
    #[error("Failed to create data dir")]
    FailedToCrateDataDir,
    #[error("Failed to interact with IO: {0}")]
    IOError(#[from] std::io::Error),
    #[error("Failed to de/serialize: {0}")]
    SerdeJson(#[from] serde_json::Error),
    #[error("Failed to parse block_number_from_path: {0}")]
    ParseIntError(#[from] std::num::ParseIntError),
    #[error("{0}")]
    Custom(String),
}
