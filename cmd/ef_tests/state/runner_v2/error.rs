#[derive(Debug)]
pub enum RunnerError {
    RootMismatch,
    FailedToGetAccountsUpdates,
    VMExecutionError(String),
    CurrentBaseFeeMissing,
    MaxPriorityFeePerGasMissing,
    MaxFeePerGasMissing,
    TxSucceededAndExceptionWasExpected,
    DifferentExceptionWasExpected,
    EIP7702ShouldNotBeCreateType
}
