use std::path::PathBuf;

use ethrex_levm::errors::VMError;

#[derive(Debug)]
pub enum RunnerError {
    FailedToGetAccountsUpdates(String),
    VMError(VMError),
    EIP7702ShouldNotBeCreateType,
    FailedToGetIndexValue(String),
    /// Wraps an I/O or serde error encountered while parsing a fixture.
    /// Holds the offending path and the underlying error message.
    ParseFixture {
        path: PathBuf,
        source: String,
    },
    Custom(String),
}
