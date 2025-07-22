use ethrex_levm::errors::VMError;

#[derive(Debug)]
pub enum RunnerError {
    FailedToGetAccountsUpdates(String),
    VMExecutionError(VMError),
    EIP7702ShouldNotBeCreateType,
    FailedToGetIndexValue(String),
}
