#[derive(Debug)]
pub enum RunnerError {
    FailedToGetAccountsUpdates(String),
    VMExecutionError(String),
    EIP7702ShouldNotBeCreateType,
    FailedToGetIndexValue(String),
}

