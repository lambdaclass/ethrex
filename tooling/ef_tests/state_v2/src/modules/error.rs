use ethrex_levm::errors::VMError;

#[derive(Debug)]
pub enum RunnerError {
    FailedToGetAccountsUpdates(String),
    VMError(VMError),
    EIP7702ShouldNotBeCreateType,
    EIP4844ShouldNotBeCreateType,
    FailedToGetIndexValue(String),
    Custom(String),
}
