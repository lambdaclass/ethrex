use bytes::Bytes;
use derive_more::derive::Display;
use ethrex_common::{Address, U256, types::Log};
use serde::{Deserialize, Serialize};
use thiserror;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize, Display)]
pub enum VMError {
    /// Errors that break execution, they shouldn't ever happen. Contains subcategory `DatabaseError`.
    Internal(#[from] InternalError),
    /// Returned when a transaction doesn't pass all validations before executing.
    TxValidation(#[from] TxValidationError),
    /// Errors contemplated by the EVM, they revert and consume all gas of the current context.
    ExceptionalHalt(#[from] ExceptionalHalt),
    /// Revert Opcode called. It behaves like ExceptionalHalt, except it doesn't consume all gas left.
    RevertOpcode,
}

impl VMError {
    /// These errors are unexpected and indicate critical issues.
    /// They should not cause a transaction to revert silently but instead fail loudly, propagating the error.
    pub fn should_propagate(&self) -> bool {
        matches!(self, VMError::Internal(_))
    }

    /// Error triggered by revert opcode. This error doesn't consume all gas left in context.
    pub fn is_revert_opcode(&self) -> bool {
        matches!(self, VMError::RevertOpcode)
    }
}

impl From<DatabaseError> for VMError {
    fn from(err: DatabaseError) -> Self {
        VMError::Internal(InternalError::Database(err))
    }
}

impl From<PrecompileError> for VMError {
    fn from(err: PrecompileError) -> Self {
        VMError::ExceptionalHalt(ExceptionalHalt::Precompile(err))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum ExceptionalHalt {
    #[error("Stack Underflow")]
    StackUnderflow,
    #[error("Stack Overflow")]
    StackOverflow,
    #[error("Invalid Jump")]
    InvalidJump,
    #[error("Opcode Not Allowed In Static Context")]
    OpcodeNotAllowedInStaticContext,
    #[error("Invalid Contract Prefix")]
    InvalidContractPrefix,
    #[error("Very Large Number")]
    VeryLargeNumber,
    #[error("Invalid Opcode")]
    InvalidOpcode,
    #[error("Address Already Occupied")]
    AddressAlreadyOccupied,
    #[error("Contract Output Too Big")]
    ContractOutputTooBig,
    #[error("Offset out of bounds")]
    OutOfBounds,
    #[error("Out Of Gas")]
    OutOfGas,
    #[error("Precompile execution error: {0}")]
    Precompile(#[from] PrecompileError),
}

// Error strings are attached to execution-spec-tests mapping https://github.com/ethereum/execution-spec-tests
// If any change is made here without changing the mapper it will break some hive tests.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum TxValidationError {
    #[error("Sender account {0} shouldn't be a contract")]
    SenderNotEOA(Address),
    #[error("Insufficient account funds")]
    InsufficientAccountFunds,
    #[error("Nonce is max")]
    NonceIsMax,
    #[error("Nonce mismatch: expected {expected}, got {actual}")]
    NonceMismatch { expected: u64, actual: u64 },
    #[error("Initcode size exceeded, max size: {max_size}, actual size: {actual_size}")]
    InitcodeSizeExceeded { max_size: usize, actual_size: usize },
    #[error("Priority fee {priority_fee} is greater than max fee per gas {max_fee_per_gas}")]
    PriorityGreaterThanMaxFeePerGas {
        priority_fee: U256,
        max_fee_per_gas: U256,
    },
    #[error("Intrinsic gas too low")]
    IntrinsicGasTooLow,
    #[error(
        "Gas allowance exceeded. Block gas limit: {block_gas_limit}, transaction gas limit: {tx_gas_limit}"
    )]
    GasAllowanceExceeded {
        block_gas_limit: u64,
        tx_gas_limit: u64,
    },
    #[error("Insufficient max fee per gas")]
    InsufficientMaxFeePerGas,
    #[error(
        "Insufficient max fee per blob gas. Expected at least {base_fee_per_blob_gas}, got: {tx_max_fee_per_blob_gas}"
    )]
    InsufficientMaxFeePerBlobGas {
        base_fee_per_blob_gas: U256,
        tx_max_fee_per_blob_gas: U256,
    },
    #[error("Type 3 transactions are not supported before the Cancun fork")]
    Type3TxPreFork,
    #[error("Type 3 transaction without blobs")]
    Type3TxZeroBlobs,
    #[error("Invalid blob versioned hash")]
    Type3TxInvalidBlobVersionedHash,
    #[error(
        "Blob count exceeded. Max blob count: {max_blob_count}, actual blob count: {actual_blob_count}"
    )]
    Type3TxBlobCountExceeded {
        max_blob_count: usize,
        actual_blob_count: usize,
    },
    #[error("Contract creation in blob transaction")]
    Type3TxContractCreation,
    #[error("Type 4 transactions are not supported before the Prague fork")]
    Type4TxPreFork,
    #[error("Empty authorization list in type 4 transaction")]
    Type4TxAuthorizationListIsEmpty,
    #[error("Contract creation in type 4 transaction")]
    Type4TxContractCreation,
    #[error("Gas limit price product overflow")]
    GasLimitPriceProductOverflow,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum InternalError {
    #[error("Arithmetic operation overflowed")]
    Overflow,
    #[error("Arithmetic operation underflowed")]
    Underflow,
    #[error("Cannot divide by zero")]
    DivisionByZero,
    #[error("Tried to convert one type to another")]
    TypeConversion,
    #[error("CallFrame not found")]
    CallFrame,
    #[error("Tried to slice non-existing data")]
    Slicing,
    #[error("Account not found when it should've been in the cache.")]
    AccountNotFound,
    #[error("Invalid precompile address. Tried to execute a precompile that does not exist.")]
    InvalidPrecompileAddress,
    #[error("Invalid Fork")]
    InvalidFork,
    #[error("Account should had been delegated")]
    AccountNotDelegated,
    #[error("No recipient found for privileged transaction")]
    RecipientNotFoundForPrivilegedTransaction,
    #[error("Memory Size Sverflow")]
    MemorySizeOverflow,
    #[error("Custom error: {0}")]
    Custom(String),
    /// Unexpected error when accessing the database, used in trait `Database`.
    #[error("Database access error: {0}")]
    Database(#[from] DatabaseError),
}

impl InternalError {
    pub fn msg(msg: &'static str) -> Self {
        Self::Custom(msg.to_owned())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum PrecompileError {
    #[error("Error while parsing the calldata")]
    ParsingInputError,
    #[error("Error while increasing consumed gas")]
    GasConsumedOverflow,
    #[error("There is not enough gas to execute precompiled contract")]
    NotEnoughGas,
    #[error("There was an error evaluating the point")]
    EvaluationError,
    #[error("This is a default error")]
    DefaultError,
    #[error("The G1 point is not in the curve")]
    BLS12381G1PointNotInCurve,
    #[error("The G2 point is not in the curve")]
    BLS12381G2PointNotInCurve,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum DatabaseError {
    #[error("{0}")]
    Custom(String),
}

#[derive(Debug, Clone)]
/// Note: "Halt" does not mean "Error during execution" it simply
/// means that the execution stopped. It's not called "Stop" because
/// "Stop" is an Opcode
pub enum OpcodeResult {
    Continue { pc_increment: usize },
    Halt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TxResult {
    Success,
    Revert(VMError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionReport {
    pub result: TxResult,
    pub gas_used: u64,
    pub gas_refunded: u64,
    pub output: Bytes,
    pub logs: Vec<Log>,
}

impl ExecutionReport {
    pub fn is_success(&self) -> bool {
        matches!(self.result, TxResult::Success)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextResult {
    pub result: TxResult,
    pub gas_used: u64,
    pub output: Bytes,
}

impl ContextResult {
    pub fn is_success(&self) -> bool {
        matches!(self.result, TxResult::Success)
    }
}
