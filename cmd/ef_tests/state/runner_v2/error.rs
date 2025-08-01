use ethrex_levm::errors::VMError;

#[derive(Debug)]
pub enum RunnerError {
    FailedToGetAccountsUpdates(String),
    VMError(VMError),
    EIP7702ShouldNotBeCreateType,
    FailedToGetIndexValue(String),
}
