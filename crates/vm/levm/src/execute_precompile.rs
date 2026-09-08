//! EXECUTE precompile for Native Rollups — aligned with the l2beat spec.
//!
//! This is a thin wrapper around `verify_stateless_new_payload` (accessed via
//! the `StatelessValidator` trait). The precompile:
//! 1. Deserializes SSZ `StatelessInput` once (to read gas_limit and check L2 constraints)
//! 2. Validates L2-specific constraints (no blobs, no withdrawals, etc.)
//! 3. Charges gas equal to `gas_limit + calldata.len() * EXECUTE_GAS_PER_WITNESS_BYTE`
//! 4. Delegates to the `StatelessValidator` trait, passing the already-decoded input
//! 5. On success returns the SSZ-encoded `StatelessValidationResult`.
//!
//! Fail-closed semantics: both invalid input and a failed validation return an
//! `ExceptionalHalt` (CALL-level `success=false`), so a `require(success)` caller
//! is safe. Invalid input (undecodable SSZ or an L2-constraint violation) is
//! rejected in steps 1-2, *before* the step-3 gas charge — those checks are cheap
//! and already priced by the L1 calldata gas the attacker paid to submit the blob.
//! A *failed validation* (step 4) is rejected *after* the step-3 charge, so an
//! attacker who forces full re-execution still pays for it: that is the DoS bound.

use bytes::Bytes;

use ethrex_common::types::Fork;
use ethrex_crypto::Crypto;

use crate::errors::{InternalError, PrecompileError, VMError};
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
    use ethrex_common::types::stateless_ssz::{
        STATELESS_INPUT_SCHEMA_ID, STATELESS_INPUT_SCHEMA_ID_SIZE, SszStatelessInput,
    };
    use libssz::SszDecode;

    // Schema-prefixed `statelessInputBytes`: a 2-byte big-endian schema id, then
    // the SSZ body. Since execution-specs #3278 the prefix is the only thing
    // identifying the fork, so an unexpected id must be rejected. All of it is
    // attacker-controlled, hence CALL-level failures rather than invariants.
    let (schema_bytes, body) = calldata
        .split_first_chunk::<STATELESS_INPUT_SCHEMA_ID_SIZE>()
        .ok_or(VMError::from(PrecompileError::ExecuteInvalidInput))?;
    let schema_id = u16::from_be_bytes(*schema_bytes);
    if schema_id != STATELESS_INPUT_SCHEMA_ID {
        return Err(VMError::from(PrecompileError::ExecuteInvalidInput));
    }

    let input = SszStatelessInput::from_ssz_bytes(body)
        .map_err(|_| VMError::from(PrecompileError::ExecuteInvalidInput))?;

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

    // Hand the already-decoded `input` to the validator so it doesn't re-parse
    // the (potentially large) witness a second time.
    let result = validator.verify(&input)?;

    use ethrex_common::types::stateless_ssz::SszStatelessValidationResult;
    let parsed = SszStatelessValidationResult::from_ssz_bytes(&result).map_err(|e| {
        VMError::Internal(InternalError::Custom(format!(
            "EXECUTE: bad validation result: {e}"
        )))
    })?;
    if !parsed.successful_validation {
        // Attacker-controlled: validation failure must be a CALL-level failure so the tx is
        // includable and the attacker pays Task 1's gas charge. `PrecompileError` → `ExceptionalHalt`
        // → should_propagate()==false. Using `Internal` here would abort the tx, making it
        // non-includable and defeating the DoS bound.
        return Err(PrecompileError::ExecuteValidationFailed.into());
    }

    Ok(Bytes::from(result))
}

/// Validate L2-specific constraints on the ExecutionPayload.
fn validate_l2_constraints(
    input: &ethrex_common::types::stateless_ssz::SszStatelessInput,
) -> Result<(), VMError> {
    let payload = &input.new_payload_request.execution_payload;

    // All checks below are attacker-controllable input violations — CALL-level failures.
    if payload.blob_gas_used != 0 {
        return Err(PrecompileError::ExecuteInvalidInput.into());
    }
    if payload.excess_blob_gas != 0 {
        return Err(PrecompileError::ExecuteInvalidInput.into());
    }
    if !payload.withdrawals.is_empty() {
        return Err(PrecompileError::ExecuteInvalidInput.into());
    }
    let reqs = &input.new_payload_request.execution_requests;
    if !reqs.deposits.is_empty()
        || !reqs.withdrawals.is_empty()
        || !reqs.consolidations.is_empty()
        || !reqs.builder_deposits.is_empty()
        || !reqs.builder_exits.is_empty()
    {
        return Err(PrecompileError::ExecuteInvalidInput.into());
    }
    for tx_bytes in payload.transactions.iter() {
        // Rejected by leading EIP-2718 envelope byte: `0x03` (EIP-4844) because
        // L2 blocks carry no blobs, and `0x06` (frame) / `0x7e` (privileged)
        // because they carry an explicit sender and no signature — the stateless
        // input commits one public key per transaction, so admitting them would
        // make a well-formed block unrepresentable. Native rollups relay L1→L2
        // messages as signed EIP-1559 transactions, so this costs no function.
        if let Some(&(0x03 | 0x06 | 0x7e)) = tx_bytes.iter().next() {
            return Err(PrecompileError::ExecuteInvalidInput.into());
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
        BuilderDepositRequest, BuilderExitRequest, Bytes20, DepositRequest, ExecutionPayload,
        ExecutionRequests, NewPayloadRequest, SszExecutionWitness, SszStatelessInput,
        SszStatelessValidationResult, Withdrawal,
    };
    use libssz::SszEncode;

    /// Mock `StatelessValidator` that returns a successful SSZ-encoded
    /// `SszStatelessValidationResult` without performing any real execution.
    struct MockValidator;

    /// Mock `StatelessValidator` that returns `successful_validation: false`.
    struct MockInvalidValidator;

    impl crate::StatelessValidator for MockValidator {
        fn verify(&self, _input: &SszStatelessInput) -> Result<Vec<u8>, crate::errors::VMError> {
            let result = SszStatelessValidationResult {
                new_payload_request_root: [0u8; 32],
                successful_validation: true,
                chain_id: 1,
                schema_id: 0x1501,
            };
            let mut buf = Vec::new();
            result.ssz_append(&mut buf);
            Ok(buf)
        }
    }

    /// Encode an input as `statelessInputBytes`: the 2-byte big-endian schema id
    /// followed by the SSZ body. Every test must go through this — a bare body is
    /// rejected at the prefix check, which would silently turn each constraint
    /// test below into a test of the prefix check.
    fn calldata_for(input: &SszStatelessInput) -> Bytes {
        let mut buf = ethrex_common::types::stateless_ssz::STATELESS_INPUT_SCHEMA_ID
            .to_be_bytes()
            .to_vec();
        input.ssz_append(&mut buf);
        Bytes::from(buf)
    }

    /// Assert an error is the constraint rejection `ExecuteInvalidInput`, and is a
    /// CALL-level failure (includable, attacker pays) rather than a tx-abort.
    fn assert_invalid_input(err: &crate::errors::VMError, what: &str) {
        assert_eq!(
            err,
            &crate::errors::VMError::from(crate::errors::PrecompileError::ExecuteInvalidInput),
            "{what} must be rejected as ExecuteInvalidInput; got: {err:?}"
        );
        assert!(
            !err.should_propagate(),
            "{what} must be a CALL-level failure (non-propagating); got: {err:?}"
        );
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
                    transactions: Vec::new().into(),
                    withdrawals: Vec::new().into(),
                    blob_gas_used: 0,
                    excess_blob_gas: 0,
                    block_access_list: Vec::new().into(),
                    slot_number: 0,
                },
                versioned_hashes: Vec::new().into(),
                parent_beacon_block_root: [0u8; 32],
                execution_requests: ExecutionRequests {
                    deposits: Vec::new().into(),
                    withdrawals: Vec::new().into(),
                    consolidations: Vec::new().into(),
                    builder_deposits: Vec::new().into(),
                    builder_exits: Vec::new().into(),
                },
            },
            witness: SszExecutionWitness {
                state: vec![].try_into().expect("state"),
                codes: vec![].try_into().expect("codes"),
                headers: vec![].try_into().expect("headers"),
            },
            chain_id: 1,
            public_keys: vec![].try_into().expect("public_keys"),
        }
    }

    impl crate::StatelessValidator for MockInvalidValidator {
        fn verify(&self, _input: &SszStatelessInput) -> Result<Vec<u8>, crate::errors::VMError> {
            let result = SszStatelessValidationResult {
                new_payload_request_root: [0u8; 32],
                successful_validation: false,
                chain_id: 1,
                schema_id: 0x1501,
            };
            let mut buf = Vec::new();
            result.ssz_append(&mut buf);
            Ok(buf)
        }
    }

    /// Fail-closed: `run_execute` must return `Err` when `successful_validation` is `false`,
    /// and `Ok` when it is `true`.
    #[test]
    fn execute_fails_closed_on_invalid() {
        let input = l2_valid_input(0, 1_000_000);
        let calldata = calldata_for(&input);

        // Invalid result → must Err (fail-closed) with a NON-propagating error so the tx is
        // INCLUDABLE and the attacker pays Task 1's gas charge (I1×I13 regression guard).
        let mut gas = 100_000_000u64;
        let mock_invalid = MockInvalidValidator;
        let r = run_execute(&mock_invalid, &calldata, &mut gas);
        let err = r.expect_err("invalid validation must revert (fail-closed), not return Ok");
        assert!(
            !err.should_propagate(),
            "invalid EXECUTE must be a CALL-level failure (includable, attacker pays), not a tx-abort; got: {err:?}"
        );

        // Valid result → must Ok.
        let mut gas2 = 100_000_000u64;
        let mock_valid = MockValidator;
        let r2 = run_execute(&mock_valid, &calldata, &mut gas2);
        assert!(r2.is_ok(), "valid validation must return Ok(result)");
    }

    /// `gas_used=0, gas_limit=1_000_000`: the charge must be `gas_limit +
    /// calldata.len() * EXECUTE_GAS_PER_WITNESS_BYTE`, NOT 0 (the old
    /// `gas_used`-based charge). Proves the fix bounds verify()'s work.
    #[test]
    fn execute_charges_gas_limit_not_gas_used() {
        let input = l2_valid_input(0, 1_000_000);
        let calldata = calldata_for(&input);

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

    /// I1 regression (moved from `stateless.rs`) + ElFantasma malformed-SSZ ask:
    /// `run_execute` must fail closed at the CALL level (non-propagating) when
    /// the calldata is not a decodable SSZ `StatelessInput`, rejecting it at the
    /// up-front decode *before* the validator runs — so a garbage or
    /// bad-offset blob can never be turned into wrong proven values, and the
    /// error is includable rather than a tx-abort.
    #[test]
    fn execute_rejects_malformed_ssz_input() {
        // 4 bytes of 0xff: an out-of-range SSZ offset, undecodable as StatelessInput.
        let calldata = Bytes::from(vec![0xffu8, 0xff, 0xff, 0xff]);
        let mut gas = 100_000_000u64;
        let err = run_execute(&MockValidator, &calldata, &mut gas)
            .expect_err("malformed SSZ input must be rejected");
        assert_invalid_input(&err, "malformed SSZ input");
    }

    // ── Negative constraint tests (I11) ──────────────────────────────────────
    // Each test builds a fully valid `SszStatelessInput`, violates exactly ONE
    // `validate_l2_constraints` check, and asserts:
    //   (a) `run_execute` returns `Err`, and
    //   (b) the error is non-propagating (CALL-level failure, so the tx is
    //       includable rather than aborted). Note: a constraint violation is
    //       rejected *before* the precompile gas charge (cheap reject, already
    //       priced by L1 calldata gas), unlike a failed *validation*, which is
    //       charged first — see the module-level fail-closed note.

    /// Constraint 1: `blob_gas_used` must be zero.
    #[test]
    fn execute_rejects_blob_gas_used_nonzero() {
        let mut input = l2_valid_input(0, 1_000_000);
        input.new_payload_request.execution_payload.blob_gas_used = 1;
        let calldata = calldata_for(&input);
        let mut gas = 100_000_000u64;
        let err = run_execute(&MockValidator, &calldata, &mut gas)
            .expect_err("blob_gas_used != 0 must be rejected");
        assert_invalid_input(&err, "blob_gas_used != 0");
    }

    /// Constraint 2: `excess_blob_gas` must be zero.
    #[test]
    fn execute_rejects_excess_blob_gas_nonzero() {
        let mut input = l2_valid_input(0, 1_000_000);
        input.new_payload_request.execution_payload.excess_blob_gas = 1;
        let calldata = calldata_for(&input);
        let mut gas = 100_000_000u64;
        let err = run_execute(&MockValidator, &calldata, &mut gas)
            .expect_err("excess_blob_gas != 0 must be rejected");
        assert_invalid_input(&err, "excess_blob_gas != 0");
    }

    /// Constraint 3: `withdrawals` list must be empty.
    #[test]
    fn execute_rejects_nonempty_withdrawals() {
        let mut input = l2_valid_input(0, 1_000_000);
        input.new_payload_request.execution_payload.withdrawals = vec![Withdrawal {
            index: 0,
            validator_index: 0,
            address: Bytes20([0u8; 20]),
            amount: 1,
        }]
        .try_into()
        .expect("withdrawals");
        let calldata = calldata_for(&input);
        let mut gas = 100_000_000u64;
        let err = run_execute(&MockValidator, &calldata, &mut gas)
            .expect_err("non-empty withdrawals must be rejected");
        assert_invalid_input(&err, "non-empty withdrawals");
    }

    /// Constraint 4: `execution_requests` (deposits/withdrawals/consolidations) must all be empty.
    /// Here we add one deposit — the same check fires for any non-empty field.
    #[test]
    fn execute_rejects_nonempty_execution_requests() {
        let mut input = l2_valid_input(0, 1_000_000);
        input.new_payload_request.execution_requests.deposits = vec![DepositRequest {
            pubkey: [0u8; 48],
            withdrawal_credentials: [0u8; 32],
            amount: 1,
            signature: [0u8; 96],
            index: 0,
        }]
        .try_into()
        .expect("deposits");
        let calldata = calldata_for(&input);
        let mut gas = 100_000_000u64;
        let err = run_execute(&MockValidator, &calldata, &mut gas)
            .expect_err("non-empty execution_requests must be rejected");
        assert_invalid_input(&err, "non-empty execution_requests");
    }

    /// Constraint 4b: the EIP-8282 builder request lists must be empty too. They
    /// were added to `ExecutionRequests` after the original three, so this pins
    /// them against being reached by the constraint check only by accident.
    #[test]
    fn execute_rejects_nonempty_builder_requests() {
        let mut input = l2_valid_input(0, 1_000_000);
        input
            .new_payload_request
            .execution_requests
            .builder_deposits = vec![BuilderDepositRequest {
            pubkey: [0u8; 48],
            withdrawal_credentials: [0u8; 32],
            amount: 1,
            signature: [0u8; 96],
        }]
        .try_into()
        .expect("builder_deposits");
        let mut gas = 100_000_000u64;
        let err = run_execute(&MockValidator, &calldata_for(&input), &mut gas)
            .expect_err("non-empty builder_deposits must be rejected");
        assert_invalid_input(&err, "non-empty builder_deposits");

        let mut input = l2_valid_input(0, 1_000_000);
        input.new_payload_request.execution_requests.builder_exits = vec![BuilderExitRequest {
            source_address: Bytes20([0u8; 20]),
            pubkey: [0u8; 48],
        }]
        .try_into()
        .expect("builder_exits");
        let mut gas = 100_000_000u64;
        let err = run_execute(&MockValidator, &calldata_for(&input), &mut gas)
            .expect_err("non-empty builder_exits must be rejected");
        assert_invalid_input(&err, "non-empty builder_exits");
    }

    /// Constraint 5: no transaction may carry a rejected EIP-2718 type byte.
    ///
    /// `0x03` is a blob transaction. `0x06` (frame) and `0x7e` (privileged) carry
    /// an explicit sender and no signature, so no public key can be recovered for
    /// them — and the stateless input commits one key per transaction. Rejecting
    /// them here is what makes "every transaction in the payload is
    /// signature-bearing" an enforced invariant rather than a property of the
    /// current producer, which `build_ssz_stateless_input` relies on to populate
    /// `public_keys` without gaps.
    #[test]
    fn execute_rejects_unsupported_transaction_types() {
        for (type_byte, what) in [
            (0x03u8, "blob-typed transaction"),
            (0x06u8, "frame transaction"),
            (0x7eu8, "privileged transaction"),
        ] {
            let mut input = l2_valid_input(0, 1_000_000);
            let tx = vec![type_byte, 0x00, 0x00].try_into().expect("tx bytes");
            input.new_payload_request.execution_payload.transactions =
                vec![tx].try_into().expect("transactions");
            let calldata = calldata_for(&input);
            let mut gas = 100_000_000u64;
            let err = run_execute(&MockValidator, &calldata, &mut gas)
                .expect_err("unsupported transaction type must be rejected");
            assert_invalid_input(&err, what);
        }

        // A type byte just outside the rejected set must still be accepted, so the
        // test cannot pass by rejecting everything.
        let mut input = l2_valid_input(0, 1_000_000);
        let tx = vec![0x02u8, 0x00, 0x00].try_into().expect("tx bytes");
        input.new_payload_request.execution_payload.transactions =
            vec![tx].try_into().expect("transactions");
        let calldata = calldata_for(&input);
        let mut gas = 100_000_000u64;
        run_execute(&MockValidator, &calldata, &mut gas)
            .expect("an EIP-1559 transaction must be accepted");
    }

    /// The schema prefix is mandatory, and only `STATELESS_INPUT_SCHEMA_ID` is
    /// accepted. Since #3278 removed chain configuration from the wire, the prefix
    /// is the sole carrier of the fork, so an unexpected id must reject rather than
    /// be validated under Amsterdam rules by default.
    #[test]
    fn execute_requires_the_expected_schema_prefix() {
        let input = l2_valid_input(0, 1_000_000);
        let mut body = Vec::new();
        input.ssz_append(&mut body);

        // No prefix at all: the body's first bytes are an SSZ offset, not a schema id.
        let mut gas = 100_000_000u64;
        let err = run_execute(&MockValidator, &Bytes::from(body.clone()), &mut gas)
            .expect_err("unprefixed input must be rejected");
        assert_invalid_input(&err, "unprefixed input");

        // A well-formed but different schema id.
        let mut wrong = vec![0x15u8, 0x02];
        wrong.extend_from_slice(&body);
        let mut gas = 100_000_000u64;
        let err = run_execute(&MockValidator, &Bytes::from(wrong), &mut gas)
            .expect_err("unexpected schema id must be rejected");
        assert_invalid_input(&err, "unexpected schema id");

        // Too short to hold an id at all.
        let mut gas = 100_000_000u64;
        let err = run_execute(&MockValidator, &Bytes::from(vec![0x15u8]), &mut gas)
            .expect_err("truncated input must be rejected");
        assert_invalid_input(&err, "input too short for a schema id");

        // And the expected prefix is accepted, so the test is not vacuous.
        let mut gas = 100_000_000u64;
        run_execute(&MockValidator, &calldata_for(&input), &mut gas)
            .expect("the expected schema prefix must be accepted");
    }
}
