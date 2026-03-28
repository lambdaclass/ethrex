//! EXECUTE precompile for Native Rollups — aligned with the l2beat spec.
//!
//! This is a thin wrapper around `verify_stateless_new_payload` (accessed via
//! the `StatelessValidator` trait). The precompile:
//! 1. Deserializes SSZ `StatelessInput` (to extract gas_used and validate L2 constraints)
//! 2. Charges gas equal to `execution_payload.gas_used`
//! 3. Validates L2-specific constraints (no blobs, no withdrawals, etc.)
//! 4. Delegates to the `StatelessValidator` trait for actual execution
//! 5. Returns SSZ-encoded `StatelessValidationResult`

use bytes::Bytes;

use ethrex_common::types::Fork;
use ethrex_crypto::Crypto;

use crate::errors::{InternalError, VMError};
use crate::precompiles::increase_precompile_consumed_gas;

/// EXECUTE precompile entrypoint.
///
/// Input: SSZ-encoded `StatelessInput`.
/// Output: SSZ-encoded `StatelessValidationResult`.
pub fn execute_precompile(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
    _crypto: &dyn Crypto,
    stateless_validator: Option<&dyn crate::StatelessValidator>,
) -> Result<Bytes, VMError> {
    let validator = stateless_validator.ok_or_else(|| {
        VMError::Internal(InternalError::Custom(
            "EXECUTE precompile requires a StatelessValidator but none was provided".to_string(),
        ))
    })?;
    run_execute(validator, calldata, gas_remaining)
}

/// Core execution logic: charge gas, validate L2 constraints, delegate.
fn run_execute(
    validator: &dyn crate::StatelessValidator,
    calldata: &Bytes,
    gas_remaining: &mut u64,
) -> Result<Bytes, VMError> {
    // Charge gas based on the L2 block's gas_used
    let gas_to_charge = extract_gas_used(calldata)?;
    increase_precompile_consumed_gas(gas_to_charge, gas_remaining)?;

    // Validate L2-specific constraints
    validate_l2_constraints(calldata)?;

    // Delegate to verify_stateless_new_payload via the trait
    let result = validator.verify(calldata)?;
    Ok(Bytes::from(result))
}

/// Extract `execution_payload.gas_used` from SSZ-encoded `StatelessInput`.
fn extract_gas_used(calldata: &[u8]) -> Result<u64, VMError> {
    use ethrex_common::types::stateless_ssz::SszStatelessInput;
    use ssz::SszDecode;

    let input = SszStatelessInput::from_ssz_bytes(calldata).map_err(|e| {
        VMError::Internal(InternalError::Custom(format!(
            "EXECUTE: failed to decode SSZ StatelessInput: {e}"
        )))
    })?;

    Ok(input.new_payload_request.execution_payload.gas_used)
}

/// Validate L2-specific constraints on the ExecutionPayload.
fn validate_l2_constraints(calldata: &[u8]) -> Result<(), VMError> {
    use ethrex_common::types::stateless_ssz::SszStatelessInput;
    use ssz::SszDecode;

    let input = SszStatelessInput::from_ssz_bytes(calldata).map_err(|e| {
        VMError::Internal(InternalError::Custom(format!(
            "EXECUTE: SSZ decode for L2 validation: {e}"
        )))
    })?;

    let payload = &input.new_payload_request.execution_payload;

    if payload.blob_gas_used != 0 {
        return Err(VMError::Internal(InternalError::Custom(
            "EXECUTE: L2 blocks must have blob_gas_used == 0".to_string(),
        )));
    }
    if payload.excess_blob_gas != 0 {
        return Err(VMError::Internal(InternalError::Custom(
            "EXECUTE: L2 blocks must have excess_blob_gas == 0".to_string(),
        )));
    }
    if !payload.withdrawals.is_empty() {
        return Err(VMError::Internal(InternalError::Custom(
            "EXECUTE: L2 blocks must have empty withdrawals".to_string(),
        )));
    }
    if !input.new_payload_request.execution_requests.is_empty() {
        return Err(VMError::Internal(InternalError::Custom(
            "EXECUTE: L2 blocks must have empty execution_requests".to_string(),
        )));
    }
    for tx_bytes in payload.transactions.iter() {
        if let Some(&0x03) = tx_bytes.iter().next() {
            return Err(VMError::Internal(InternalError::Custom(
                "EXECUTE: L2 blocks must not contain blob transactions".to_string(),
            )));
        }
    }

    Ok(())
}
