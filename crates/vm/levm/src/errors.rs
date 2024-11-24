use crate::account::Account;
use bytes::Bytes;
use ethrex_core::{types::Log, Address};
use std::collections::HashMap;
use thiserror;

/// Errors that halt the program
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VMError {
    #[error("Stack Underflow")]
    StackUnderflow,
    #[error("Stack Overflow")]
    StackOverflow,
    #[error("Invalid Jump")]
    InvalidJump,
    #[error("Opcode Not Allowed In Static Context")]
    OpcodeNotAllowedInStaticContext,
    #[error("Opcode Not Found")]
    OpcodeNotFound,
    #[error("Invalid Bytecode")]
    InvalidBytecode,
    #[error("Very Large Number")]
    VeryLargeNumber,
    #[error("Fatal Error")]
    FatalError,
    #[error("Invalid Transaction")]
    InvalidTransaction,
    #[error("Revert Opcode")]
    RevertOpcode,
    #[error("Invalid Opcode")]
    InvalidOpcode,
    #[error("Missing Blob Hashes")]
    MissingBlobHashes,
    #[error("Blob Hash Index Out Of Bounds")]
    BlobHashIndexOutOfBounds,
    #[error("Sender Account Does Not Exist")]
    SenderAccountDoesNotExist,
    #[error("Address Does Not Match An Account")]
    AddressDoesNotMatchAnAccount,
    #[error("Sender account should not have bytecode")]
    SenderNotEOA,
    #[error("Insufficient account founds")]
    InsufficientAccountFunds,
    #[error("Gas price is lower than base fee")]
    GasPriceIsLowerThanBaseFee,
    #[error("Address Already Occupied")]
    AddressAlreadyOccupied,
    #[error("Contract Output Too Big")]
    ContractOutputTooBig,
    #[error("Invalid Initial Byte")]
    InvalidInitialByte,
    #[error("Nonce is max (overflow)")]
    NonceIsMax,
    #[error("Memory load out of bounds")]
    MemoryLoadOutOfBounds,
    #[error("Memory Store Out Of Bounds")]
    MemoryStoreOutOfBounds,
    #[error("Gas limit price product overflow")]
    GasLimitPriceProductOverflow,
    #[error("Balance Overflow")]
    BalanceOverflow,
    #[error("Balance Underflow")]
    BalanceUnderflow,
    #[error("Gas refunds underflow")]
    GasRefundsUnderflow,
    #[error("Gas refunds overflow")]
    GasRefundsOverflow,
    #[error("Initcode size exceeded")]
    InitcodeSizeExceeded,
    #[error("Priority fee greater than max fee per gas")]
    PriorityGreaterThanMaxFeePerGas,
    #[error("Intrinsic gas too low")]
    IntrinsicGasTooLow,
    #[error("Gas allowance exceeded")]
    GasAllowanceExceeded,
    #[error("Insufficient max fee per gas")]
    InsufficientMaxFeePerGas,
    #[error("Insufficient max fee per blob gas")]
    InsufficientMaxFeePerBlobGas,
    #[error("Memory size overflows")]
    MemorySizeOverflow,
    #[error("Type3TxZeroBlobs")]
    Type3TxZeroBlobs,
    #[error("Type3TxInvalidBlobVersionedHash")]
    Type3TxInvalidBlobVersionedHash,
    #[error("Type3TxBlobCountExceeded")]
    Type3TxBlobCountExceeded,
    #[error("Type3TxContractCreation")]
    Type3TxContractCreation,
    #[error("Revert Create")]
    RevertCreate,
    // OutOfGas
    #[error("Out Of Gas")]
    OutOfGas(#[from] OutOfGasError),
    // Internal
    #[error("Internal error: {0}")]
    Internal(#[from] InternalError),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, thiserror::Error)]
pub enum OutOfGasError {
    #[error("Gas Cost Overflow")]
    GasCostOverflow,
    #[error("Gas Used Overflow")]
    GasUsedOverflow,
    #[error("Creation Cost Is Too High")]
    CreationCostIsTooHigh,
    #[error("Consumed Gas Overflow")]
    ConsumedGasOverflow,
    #[error("Max Gas Limit Exceeded")]
    MaxGasLimitExceeded,
    #[error("Arithmetic operation divided by zero in gas calculation")]
    ArithmeticOperationDividedByZero,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InternalError {
    #[error("Overflowed when incrementing nonce")]
    NonceOverflowed,
    #[error("Underflowed when incrementing nonce")]
    NonceUnderflowed,
    #[error("Overflowed when incrementing program counter")]
    PCOverflowed,
    #[error("Underflowed when decrementing program counter")]
    PCUnderflowed,
    #[error("Arithmetic operation overflowed")]
    ArithmeticOperationOverflow,
    #[error("Arithmetic operation underflowed")]
    ArithmeticOperationUnderflow,
    #[error("Arithmetic operation divided by zero")]
    ArithmeticOperationDividedByZero,
    #[error("Accound should have been cached")]
    AccountShouldHaveBeenCached,
    #[error("Tried to convert one type to another")]
    ConversionError,
    #[error("Division error")]
    DivisionError,
    #[error("Tried to access last call frame but found none")]
    CouldNotAccessLastCallframe, // Last callframe before execution is the same as the first, but after execution the last callframe is actually the initial CF
    #[error("Tried to read from empty code")]
    TriedToIndexEmptyCode,
    #[error("Failed computing CREATE address")]
    CouldNotComputeCreateAddress,
    #[error("Failed computing CREATE2 address")]
    CouldNotComputeCreate2Address,
    #[error("Tried to slice non-existing data")]
    SlicingError,
    #[error("Could not pop callframe")]
    CouldNotPopCallframe,
    #[error("Account not found")]
    AccountNotFound,
    #[error("ExcessBlobGas should not be None")]
    ExcessBlobGasShouldNotBeNone,
    #[error("Error in utils file")]
    UtilsError,
    #[error("Overflow error")]
    OperationOverflow,
    #[error("Undefined state")]
    UndefinedState(i32), // This error is temporarily for things that cause an undefined state.
}

impl VMError {
    pub fn is_internal(&self) -> bool {
        matches!(self, VMError::Internal(_))
    }
}

#[derive(Debug, Clone)]
pub enum OpcodeSuccess {
    Continue,
    Result(ResultReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResultReason {
    Stop,
    Revert,
    Return,
    SelfDestruct,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TxResult {
    Success,
    Revert(VMError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionReport {
    pub result: TxResult,
    pub new_state: HashMap<Address, Account>,
    pub gas_used: u64,
    pub gas_refunded: u64,
    pub output: Bytes,
    pub logs: Vec<Log>,
    // This only applies to create transactions. It's fundamentally ambiguous since
    // a transaction could create multiple new contracts, but whatever.
    pub created_address: Option<Address>,
}

impl TransactionReport {
    /// Function to add gas to report, if it exceeds max gas limit it should return OutOfGas error. Only used for adding gas after execution.
    pub fn add_gas_with_max(&mut self, gas: u64, max: u64) -> Result<(), VMError> {
        self.gas_used = self
            .gas_used
            .checked_add(gas)
            .filter(|&total| total <= max)
            .ok_or(VMError::OutOfGas(OutOfGasError::MaxGasLimitExceeded))?;
        Ok(())
    }

    pub fn is_success(&self) -> bool {
        matches!(self.result, TxResult::Success)
    }
}
