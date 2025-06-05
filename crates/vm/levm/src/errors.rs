use bytes::Bytes;
use derive_more::derive::Display;
use ethrex_common::types::Log;
use serde::{Deserialize, Serialize};
use thiserror;

use crate::db::error::DatabaseError;

//TODO: Rename to VMError afterwards
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize, Display)]
pub enum VMError {
    /// Errors that break execution, they shouldn't ever happen. Contains subcategory `DatabaseError`.
    Internal(#[from] InternalError),
    /// Returned when a transaction doesn't pass all validations before executing.
    TxValidation(#[from] TxValidationError),
    /// Errors contemplated by the EVM, they revert and consume all gas of the current context.
    ExceptionalHalt(#[from] ExceptionalHalt),
    /// Revert Opcode called. Unlike exceptional gas, this doesn't consume all gas of the context.
    RevertOpcode,
}

impl VMError {
    /// These errors are unexpected and indicate critical issues.
    /// They should not cause a transaction to revert silently but instead fail loudly, propagating the error.
    pub fn should_propagate(&self) -> bool {
        matches!(self, VMError::Internal(_))
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
    #[error("Invalid Transaction")]
    InvalidTransaction,
    #[error("Invalid Opcode")]
    InvalidOpcode,
    #[error("Gas price is lower than base fee")]
    GasPriceIsLowerThanBaseFee,
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

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum TxValidationError {
    #[error("Sender account shouldn't be a contract")]
    SenderNotEOA,
    #[error("Insufficient account founds")]
    InsufficientAccountFunds,
    #[error("Nonce is max")]
    NonceIsMax,
    #[error("Nonce mismatch: expected {expected}, got {actual}")]
    NonceMismatch { expected: u64, actual: u64 },
    #[error("Initcode size exceeded")]
    InitcodeSizeExceeded,
    #[error("Priority fee is greater than max fee per gas")]
    PriorityGreaterThanMaxFeePerGas,
    #[error("Intrinsic gas too low")]
    IntrinsicGasTooLow,
    #[error("Gas allowance exceeded")]
    GasAllowanceExceeded,
    #[error("Insufficient max fee per gas")]
    InsufficientMaxFeePerGas,
    #[error("Insufficient max fee per blob gas")]
    InsufficientMaxFeePerBlobGas,
    #[error("Type 3 transactions are not supported before the Cancun fork")]
    Type3TxPreFork,
    #[error("Type 3 transaction without blobs")]
    Type3TxZeroBlobs,
    #[error("Invalid blob versioned hash")]
    Type3TxInvalidBlobVersionedHash,
    #[error("Blob count exceeded")]
    Type3TxBlobCountExceeded,
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
    SlicingError,
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
