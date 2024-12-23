#[derive(Debug, thiserror::Error)]
pub enum ProverError {
    #[error("Incorrect ProverType")]
    IncorrectProverType,
    #[error("{0}")]
    Custom(String),
}
