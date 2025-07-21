#[derive(Debug)]
pub enum RunnerError {
    RootMismatch,
    FailedToGetAccountsUpdates,
    VMExecutionError(String),
    TxSucceededAndExceptionWasExpected,
    DifferentExceptionWasExpected,
    EIP7702ShouldNotBeCreateType,
    FailedToGetIndexValue(String),
}
