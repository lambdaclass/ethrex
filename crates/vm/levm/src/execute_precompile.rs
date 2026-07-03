//! EXECUTE precompile for Native Rollups — aligned with the l2beat spec.
//!
//! This is a thin wrapper around `verify_stateless_new_payload` (accessed via
//! the `StatelessValidator` trait). The precompile:
//! 1. Deserializes SSZ `StatelessInput` (to extract gas_limit and validate L2 constraints)
//! 2. Charges gas equal to `gas_limit + calldata.len() * EXECUTE_GAS_PER_WITNESS_BYTE`
//! 3. Validates L2-specific constraints (no blobs, no withdrawals, etc.)
//! 4. Delegates to the `StatelessValidator` trait for actual execution
//! 5. Returns SSZ-encoded `StatelessValidationResult`

use bytes::Bytes;

use ethrex_common::types::Fork;
use ethrex_crypto::Crypto;

use crate::errors::{InternalError, VMError};
use crate::precompiles::increase_precompile_consumed_gas;

/// WIP DA/decode cost — the SSZ witness decode + trie rebuild scales with input size;
/// 16 mirrors calldata non-zero byte cost. Tune when EIP-8079 pricing is finalized.
pub const EXECUTE_GAS_PER_WITNESS_BYTE: u64 = 16;

/// EXECUTE precompile entrypoint.
///
/// Input: SSZ-encoded `StatelessInput`.
/// Output: SSZ-encoded `StatelessValidationResult`.
///
/// This wrapper exists to mimic the standard precompile signature
/// (`fn(&Bytes, &mut u64, Fork, &dyn Crypto) -> Result<Bytes, VMError>`) plus
/// the extra `stateless_validator` slot threaded through the dispatcher. It
/// also converts the `Option` into an error, so `run_execute` can work with a
/// guaranteed-`Some` validator.
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

/// Validate L2 constraints, charge gas, delegate.
fn run_execute(
    validator: &dyn crate::StatelessValidator,
    calldata: &Bytes,
    gas_remaining: &mut u64,
) -> Result<Bytes, VMError> {
    use ethrex_common::types::stateless_ssz::SszStatelessInput;
    use libssz::SszDecode;

    let input = SszStatelessInput::from_ssz_bytes(calldata).map_err(|e| {
        VMError::Internal(InternalError::Custom(format!(
            "EXECUTE: failed to decode SSZ StatelessInput: {e}"
        )))
    })?;

    validate_l2_constraints(&input)?;

    // `gas_used` is attacker-controlled and does NOT bound verify()'s work:
    // a malicious sequencer can set gas_used=0 while verify() still does full
    // native re-execution (bounded by gas_limit), witness decode, and trie rebuild.
    // `gas_limit` IS the true upper bound on EVM re-execution (execute_block
    // enforces the block gas limit). The per-byte term bounds witness decode and
    // trie rebuild costs that scale with input size.
    let gas_limit = input.new_payload_request.execution_payload.gas_limit;
    #[expect(
        clippy::as_conversions,
        reason = "usize to u64: safe on 64-bit targets; calldata len cannot exceed u64::MAX"
    )]
    let witness_charge = (calldata.len() as u64).saturating_mul(EXECUTE_GAS_PER_WITNESS_BYTE);
    let charge = gas_limit.saturating_add(witness_charge);
    increase_precompile_consumed_gas(charge, gas_remaining)?;

    let result = validator.verify(calldata)?;
    Ok(Bytes::from(result))
}

/// Validate L2-specific constraints on the ExecutionPayload.
fn validate_l2_constraints(
    input: &ethrex_common::types::stateless_ssz::SszStatelessInput,
) -> Result<(), VMError> {
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
    let reqs = &input.new_payload_request.execution_requests;
    if !reqs.deposits.is_empty() || !reqs.withdrawals.is_empty() || !reqs.consolidations.is_empty()
    {
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

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    clippy::as_conversions
)]
mod tests {
    use super::{EXECUTE_GAS_PER_WITNESS_BYTE, run_execute};
    use bytes::Bytes;
    use ethrex_common::types::stateless_ssz::{
        Bytes20, ExecutionPayload, ExecutionRequests, NewPayloadRequest, SszChainConfig,
        SszExecutionWitness, SszForkActivation, SszForkConfig, SszStatelessInput,
        SszStatelessValidationResult,
    };
    use libssz::SszEncode;

    /// Mock `StatelessValidator` that returns a successful SSZ-encoded
    /// `SszStatelessValidationResult` without performing any real execution.
    struct MockValidator;

    impl crate::StatelessValidator for MockValidator {
        fn verify(&self, _input: &[u8]) -> Result<Vec<u8>, crate::errors::VMError> {
            let result = SszStatelessValidationResult {
                new_payload_request_root: [0u8; 32],
                successful_validation: true,
                chain_config: SszChainConfig {
                    chain_id: 1,
                    active_fork: SszForkConfig {
                        fork: 0,
                        activation: SszForkActivation {
                            block_number: vec![].try_into().expect("empty block_number"),
                            timestamp: vec![].try_into().expect("empty timestamp"),
                        },
                        blob_schedule: vec![].try_into().expect("empty blob_schedule"),
                    },
                },
            };
            let mut buf = Vec::new();
            result.ssz_append(&mut buf);
            Ok(buf)
        }
    }

    /// Build a minimal L2-valid `SszStatelessInput` with the given gas fields.
    /// No blobs, no withdrawals, no execution requests — satisfies all
    /// `validate_l2_constraints` checks.
    fn l2_valid_input(gas_used: u64, gas_limit: u64) -> SszStatelessInput {
        SszStatelessInput {
            new_payload_request: NewPayloadRequest {
                execution_payload: ExecutionPayload {
                    parent_hash: [0u8; 32],
                    fee_recipient: Bytes20([0u8; 20]),
                    state_root: [0u8; 32],
                    receipts_root: [0u8; 32],
                    logs_bloom: vec![0u8; 256].try_into().expect("logs_bloom"),
                    prev_randao: [0u8; 32],
                    block_number: 1,
                    gas_limit,
                    gas_used,
                    timestamp: 1_000_000,
                    extra_data: vec![].try_into().expect("extra_data"),
                    base_fee_per_gas: [0u8; 32],
                    block_hash: [0u8; 32],
                    transactions: vec![].try_into().expect("transactions"),
                    withdrawals: vec![].try_into().expect("withdrawals"),
                    blob_gas_used: 0,
                    excess_blob_gas: 0,
                    block_access_list: vec![].try_into().expect("block_access_list"),
                },
                versioned_hashes: vec![].try_into().expect("versioned_hashes"),
                parent_beacon_block_root: [0u8; 32],
                execution_requests: ExecutionRequests {
                    deposits: vec![].try_into().expect("deposits"),
                    withdrawals: vec![].try_into().expect("withdrawals"),
                    consolidations: vec![].try_into().expect("consolidations"),
                },
            },
            witness: SszExecutionWitness {
                state: vec![].try_into().expect("state"),
                codes: vec![].try_into().expect("codes"),
                headers: vec![].try_into().expect("headers"),
            },
            chain_config: SszChainConfig {
                chain_id: 1,
                active_fork: SszForkConfig {
                    fork: 0,
                    activation: SszForkActivation {
                        block_number: vec![].try_into().expect("block_number"),
                        timestamp: vec![].try_into().expect("timestamp"),
                    },
                    blob_schedule: vec![].try_into().expect("blob_schedule"),
                },
            },
            public_keys: vec![].try_into().expect("public_keys"),
        }
    }

    /// `gas_used=0, gas_limit=1_000_000`: the charge must be `gas_limit +
    /// calldata.len() * EXECUTE_GAS_PER_WITNESS_BYTE`, NOT 0 (the old
    /// `gas_used`-based charge). Proves the fix bounds verify()'s work.
    #[test]
    fn execute_charges_gas_limit_not_gas_used() {
        let input = l2_valid_input(0, 1_000_000);
        let mut calldata_buf = Vec::new();
        input.ssz_append(&mut calldata_buf);
        let calldata = Bytes::from(calldata_buf);

        let start_gas = 100_000_000u64;
        let mut gas_remaining = start_gas;
        let mock = MockValidator;
        let _ = run_execute(&mock, &calldata, &mut gas_remaining);

        let charged = start_gas.saturating_sub(gas_remaining);
        let expected = 1_000_000u64.saturating_add(
            u64::try_from(calldata.len())
                .expect("calldata len fits u64")
                .saturating_mul(EXECUTE_GAS_PER_WITNESS_BYTE),
        );

        // Must charge at least gas_limit, NOT 0 (old gas_used-based charge).
        assert!(
            charged >= 1_000_000,
            "must charge at least gas_limit, got {charged}"
        );
        // Must equal gas_limit + per-byte witness charge exactly.
        assert_eq!(
            charged, expected,
            "charge must be gas_limit + calldata.len() * EXECUTE_GAS_PER_WITNESS_BYTE"
        );
    }
}
