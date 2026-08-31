use bytes::Bytes;
use derive_more::derive::Display;
use ethrex_common::{
    Address, H256, U256,
    types::{FakeExponentialError, Log},
};
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

/// Useful to use ? in try_into, specially when slicing with known bounds to fixed size arrays,
/// which is a error that never really happens.
impl From<std::array::TryFromSliceError> for VMError {
    fn from(_: std::array::TryFromSliceError) -> Self {
        VMError::Internal(InternalError::TypeConversion)
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
    #[error("Transaction gas limit lower than the minimum gas cost to execute the transaction")]
    IntrinsicGasTooLow,
    #[error("Transaction gas limit lower than the gas cost floor for calldata tokens")]
    IntrinsicGasBelowFloorGasCost,
    /// L2-only. Raised by the L2 hook's `reserve_l1_gas` when the gas limit cannot
    /// cover the L1 data-availability gas reserved on top of the intrinsic cost or
    /// the calldata floor. Distinct from [`TxValidationError::IntrinsicGasTooLow`]
    /// on purpose: the reserved amount is derived from the L1 fee config and the
    /// block's gas price, so this failure is TRANSIENT — the same transaction can
    /// succeed in a later block once L1 fees drop. Consumers that classify errors
    /// as permanent-vs-transient (the payload builders' `is_deterministic_invalid`)
    /// rely on the two not being conflated.
    #[error("Transaction gas limit cannot cover the reserved L1 data availability gas")]
    L1GasReservationTooLow,
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
    #[error("Frame transactions (EIP-8141) are not supported before the Hegota fork")]
    FrameTxPreFork,
    #[error("Gas limit price product overflow")]
    GasLimitPriceProductOverflow,
    #[error(
        "Transaction gas limit exceeds maximum. Transaction hash: {tx_hash}, transaction gas limit: {tx_gas_limit}"
    )]
    TxMaxGasLimitExceeded { tx_hash: H256, tx_gas_limit: u64 },
    #[error("Invalid frame transaction: VERIFY frame did not call APPROVE or payer not approved")]
    InvalidFrameTransaction,
    /// EIP-8272: a reference whose slot is not yet referenceable. A root written
    /// in slot `S` becomes referenceable in slot `S + 1`, so this resolves on its
    /// own as the chain advances and must not be treated as intrinsic invalidity.
    #[error("EIP-8272 recent-root reference is not yet referenceable at this slot")]
    FrameTxRecentRootNotReferenceable,
    /// EIP-8272: a reference outside the usable window, or one whose entry is not
    /// committed under `RECENT_ROOT_ADDRESS` in the transaction pre-state.
    #[error("EIP-8272 recent-root reference is expired or not committed")]
    FrameTxRecentRootInvalid,
    /// EIP-8312: a spend whose input cannot be proven YET — its creation block's
    /// openings root is written at that block's end (so a UTXO is never spendable
    /// in its own creation block), or its batch is not sealed yet. Resolves on its
    /// own as the chain advances, so the builder keeps the transaction pooled
    /// rather than evicting a valid spend. Distinct from every other UTXO failure,
    /// which is permanent.
    #[error("EIP-8312 UTXO input is not yet spendable at this block")]
    UtxoNotYetSpendable,
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
    #[error("{0}")]
    FakeExponentialError(#[from] FakeExponentialError),
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
    #[error("There is not enough gas to execute precompiled contract")]
    NotEnoughGas,
    #[error("Invalid point")]
    InvalidPoint,
    #[error("The point is not in the subgroup")]
    PointNotInSubgroup,
    #[error("The G1 point is not in the curve")]
    BLS12381G1PointNotInCurve,
    #[error("The G2 point is not in the curve")]
    BLS12381G2PointNotInCurve,
    #[error("Mod-exp base length is too large")]
    ModExpBaseTooLarge,
    #[error("Mod-exp exponent length is too large")]
    ModExpExpTooLarge,
    #[error("Mod-exp modulus length is too large")]
    ModExpModulusTooLarge,
    #[error("Coordinate Exceeds Field Modulus")]
    CoordinateExceedsFieldModulus,
    /// EXECUTE precompile: the L1-submitted block failed stateless validation.
    /// Attacker-controllable: tx continues, forwarded gas is consumed, tx is includable.
    #[error("EXECUTE: stateless validation failed")]
    ExecuteValidationFailed,
    /// EXECUTE precompile: the calldata is malformed or violates L2 constraints.
    /// Attacker-controllable input: treat as CALL-level failure, not an internal invariant.
    #[error("EXECUTE: invalid input for native-rollup execution")]
    ExecuteInvalidInput,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum DatabaseError {
    #[error("{0}")]
    Custom(String),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum OpcodeResult {
    Continue,
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
    /// Gas used before refunds (for block-level accounting).
    /// Pre-EIP-7778: This is the post-refund gas.
    /// Post-EIP-7778: This is the pre-refund gas.
    pub gas_used: u64,
    /// Gas spent after refunds (what the user actually pays).
    /// This is always the post-refund gas value.
    /// Pre-EIP-7778: Same as gas_used.
    /// Post-EIP-7778: gas_used - refunds (capped).
    pub gas_spent: u64,
    pub gas_refunded: u64,
    /// EIP-8037: State gas portion of gas_used (Amsterdam+).
    /// Block gas_used = max(sum(regular_gas), sum(state_gas)).
    pub state_gas_used: u64,
    pub output: Bytes,
    pub logs: Vec<Log>,
    /// For frame transactions: the address that paid for gas
    pub payer_address: Option<Address>,
    /// For frame transactions: one entry per frame, in frame order.
    pub frame_results: Option<Vec<FrameResult>>,
}

/// One frame's outcome inside an EIP-8141 frame transaction, as reported in the
/// transaction's receipt.
///
/// `gas_used` and `state_gas_used` are the two halves of v2's two-dimensional
/// `gas_used = [execution, state]`. They are separate budgets that never mix, so
/// they are named rather than positional: both are `u64`, and swapping them would
/// misreport every frame without failing to compile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameResult {
    /// A `FRAME_RECEIPT_STATUS_*` code (0 = failure, 1 = success, 3 = skipped
    /// due to a failed atomic batch, per the EIP-8141 receipt encoding).
    pub status: u8,
    /// Execution gas drawn from the frame's `limits.execution`.
    pub gas_used: u64,
    /// State gas drawn from the frame's `limits.state`. Zero for a frame that
    /// reverted or halted exceptionally, and subject to reduction by a later
    /// frame's cross-frame refill.
    pub state_gas_used: u64,
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
    /// Gas used before refunds (for block-level accounting).
    pub gas_used: u64,
    /// Gas spent after refunds (what the user actually pays).
    pub gas_spent: u64,
    pub output: Bytes,
}

impl ContextResult {
    pub fn is_success(&self) -> bool {
        matches!(self.result, TxResult::Success)
    }

    /// Returns true if this result is an address collision error.
    pub fn is_collision(&self) -> bool {
        matches!(
            self.result,
            TxResult::Revert(VMError::ExceptionalHalt(
                ExceptionalHalt::AddressAlreadyOccupied
            ))
        )
    }

    /// True if the failure was caused by the REVERT opcode (intentional revert).
    /// Used to gate behaviour that differs between REVERT and ExceptionalHalt —
    /// e.g. return data is propagated to the parent on REVERT only.
    pub fn is_revert_opcode(&self) -> bool {
        matches!(self.result, TxResult::Revert(VMError::RevertOpcode))
    }
}
