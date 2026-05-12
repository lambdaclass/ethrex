//! Maps LEVM [`VMError`] variants to the error strings emitted by geth's EVM.
//!
//! Strings are taken from `go-ethereum/core/vm/errors.go`. Keeping them
//! identical allows goevmlab diff tools to match traces across implementations.

use ethrex_levm::errors::{ExceptionalHalt, TxValidationError, VMError};

/// Returns the geth-compatible error string for a LEVM [`VMError`].
///
/// For variants whose [`Display`] impl already matches the geth string exactly
/// (notably `StackUnderflow` and `StackOverflow` which were made geth-compatible
/// in Phase 4a), the Display output is used directly.
pub fn vm_error_to_geth_string(err: &VMError) -> String {
    match err {
        VMError::RevertOpcode => "execution reverted".to_owned(),
        VMError::ExceptionalHalt(halt) => exceptional_halt_to_geth_string(halt),
        VMError::TxValidation(tv) => tx_validation_to_geth_string(tv),
        VMError::Internal(internal) => internal.to_string(),
    }
}

/// Maps `TxValidationError` variants to the strings geth emits from
/// `core/types/transaction.go` and `core/state_transition.go`. Variants without
/// a clear geth analog fall through to LEVM Display.
fn tx_validation_to_geth_string(tv: &TxValidationError) -> String {
    match tv {
        TxValidationError::IntrinsicGasTooLow
        | TxValidationError::IntrinsicGasBelowFloorGasCost => "intrinsic gas too low".to_owned(),
        TxValidationError::NonceMismatch { actual, expected } if actual < expected => {
            "nonce too low".to_owned()
        }
        TxValidationError::NonceMismatch { .. } => "nonce too high".to_owned(),
        TxValidationError::NonceIsMax => "nonce has max value".to_owned(),
        TxValidationError::InsufficientAccountFunds => {
            "insufficient funds for gas * price + value".to_owned()
        }
        TxValidationError::InsufficientMaxFeePerGas => {
            "max fee per gas less than block base fee".to_owned()
        }
        TxValidationError::PriorityGreaterThanMaxFeePerGas { .. } => {
            "max priority fee per gas higher than max fee per gas".to_owned()
        }
        TxValidationError::InsufficientMaxFeePerBlobGas { .. } => {
            "max fee per blob gas less than block blob gas fee".to_owned()
        }
        TxValidationError::Type3TxPreFork => "blob tx used before Cancun".to_owned(),
        TxValidationError::Type3TxZeroBlobs => "blobless blob transaction".to_owned(),
        TxValidationError::Type3TxInvalidBlobVersionedHash => "invalid versioned hash".to_owned(),
        TxValidationError::Type3TxBlobCountExceeded { .. } => "too many blobs".to_owned(),
        TxValidationError::Type3TxContractCreation => {
            "blob transaction is a contract creation".to_owned()
        }
        TxValidationError::Type4TxPreFork => "setcode tx used before Prague".to_owned(),
        TxValidationError::Type4TxAuthorizationListIsEmpty => {
            "EIP-7702 transaction with empty auth list".to_owned()
        }
        TxValidationError::Type4TxContractCreation => {
            "setcode tx is a contract creation".to_owned()
        }
        TxValidationError::InitcodeSizeExceeded { .. } => "max initcode size exceeded".to_owned(),
        TxValidationError::GasAllowanceExceeded { .. } => "gas limit reached".to_owned(),
        TxValidationError::SenderNotEOA(_) => "sender not an eoa".to_owned(),
        // Fall through to LEVM Display for variants without a clean geth analog.
        TxValidationError::GasLimitPriceProductOverflow
        | TxValidationError::TxMaxGasLimitExceeded { .. } => tv.to_string(),
    }
}

fn exceptional_halt_to_geth_string(halt: &ExceptionalHalt) -> String {
    match halt {
        // Phase 4a gave these variants a geth-compatible Display: use it directly.
        ExceptionalHalt::StackUnderflow { .. } => halt.to_string(),
        ExceptionalHalt::StackOverflow { .. } => halt.to_string(),

        ExceptionalHalt::OutOfGas => "out of gas".to_owned(),
        ExceptionalHalt::InvalidJump => "invalid jump destination".to_owned(),
        ExceptionalHalt::OpcodeNotAllowedInStaticContext => "write protection".to_owned(),
        ExceptionalHalt::InvalidContractPrefix => {
            "invalid code: must not begin with 0xef".to_owned()
        }
        // geth emits "invalid opcode: OPCODE_NAME"; without opcode-name info we
        // emit the shorter form which is still valid per geth's error.go.
        ExceptionalHalt::InvalidOpcode => "invalid opcode".to_owned(),
        ExceptionalHalt::AddressAlreadyOccupied => "contract address collision".to_owned(),
        ExceptionalHalt::ContractOutputTooBig => "max code size exceeded".to_owned(),
        ExceptionalHalt::OutOfBounds => "return data out of bounds".to_owned(),
        ExceptionalHalt::VeryLargeNumber => "gas uint64 overflow".to_owned(),
        // Precompile errors are not a top-level geth error string; fall through
        // to LEVM Display which includes the precompile-specific message.
        ExceptionalHalt::Precompile(_) => halt.to_string(),
    }
}
