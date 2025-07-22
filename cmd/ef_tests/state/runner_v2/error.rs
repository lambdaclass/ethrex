use ethrex_levm::errors::VMError;

#[derive(Debug)]
pub enum RunnerError {
    RootMismatch,
    FailedToGetAccountsUpdates,
    VMExecutionError(VMError),
    TxSucceededAndExceptionWasExpected,
    DifferentExceptionWasExpected,
    EIP7702ShouldNotBeCreateType,
    FailedToGetIndexValue(String),
}
