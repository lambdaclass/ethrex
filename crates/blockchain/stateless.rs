//! Stateless block validation for the native-rollup EXECUTE precompile.
//!
//! The core function mirrors `verify_stateless_new_payload` from execution-specs
//! (projects/zkevm branch). It is invoked by the EXECUTE precompile via the
//! `StatelessValidator` trait (see `StatelessExecutor` below).
//!
//! Note: the zkVM guest binaries validate via the separate `validate_eip8025_*`
//! path, not this module.

use std::sync::Arc;

use ethrex_common::types::block_execution_witness::ExecutionWitness;
use ethrex_common::types::stateless_ssz::{
    NewPayloadRequest, SszChainConfig, SszStatelessInput, SszStatelessValidationResult,
};
use ethrex_crypto::Crypto;
use ethrex_guest_program::common::ExecutionError;
use ethrex_guest_program::l1::verify_stateless_block;
use libssz::SszEncode;
use libssz_merkle::{HashTreeRoot, Sha2Hasher};

/// Core stateless validation function matching the execution-specs definition.
///
/// Takes a `NewPayloadRequest`, `ExecutionWitness`, and `ChainConfig`, and:
/// 1. Computes `hash_tree_root` of the `NewPayloadRequest`
/// 2. Converts the payload to a `Block`
/// 3. Executes the block statelessly
/// 4. Returns the validation result
pub fn verify_stateless_new_payload(
    new_payload_request: &NewPayloadRequest,
    execution_witness: ExecutionWitness,
    chain_config: &SszChainConfig,
    crypto: Arc<dyn Crypto>,
) -> SszStatelessValidationResult {
    let request_root = new_payload_request.hash_tree_root(&Sha2Hasher);

    let successful = match verify_inner(new_payload_request, execution_witness, crypto) {
        Ok(()) => true,
        Err(e) => {
            tracing::error!("stateless validation failed: {e}");
            false
        }
    };

    SszStatelessValidationResult {
        new_payload_request_root: request_root,
        successful_validation: successful,
        chain_config: chain_config.clone(),
    }
}

fn verify_inner(
    new_payload_request: &NewPayloadRequest,
    execution_witness: ExecutionWitness,
    crypto: Arc<dyn Crypto>,
) -> Result<(), ExecutionError> {
    verify_stateless_block(new_payload_request, execution_witness, crypto)
}

/// Concrete `StatelessValidator` used by the EXECUTE precompile: deserializes
/// SSZ `StatelessInput`, calls `verify_stateless_new_payload`, and serializes
/// the result back to SSZ.
///
/// The `StatelessValidator` trait is defined in `ethrex-levm` and implemented
/// here rather than inline in the precompile because `verify_stateless_new_payload`
/// depends on `ethrex-vm` and `ethrex-guest-program`, which in turn depend on
/// `ethrex-levm`. A direct call would form a cycle. The trait breaks it via
/// dependency inversion: levm owns the interface, blockchain owns the
/// implementation, and a borrowed `&dyn StatelessValidator` (backed by an
/// `Arc<StatelessExecutor>` the blockchain holds) is passed to `VM::new` as its
/// `Option<&dyn StatelessValidator>` argument at runtime (see `blockchain.rs` VM
/// construction sites).
pub struct StatelessExecutor {
    pub crypto: Arc<dyn Crypto>,
}

impl ethrex_vm::StatelessValidator for StatelessExecutor {
    fn verify(&self, input: &SszStatelessInput) -> Result<Vec<u8>, ethrex_vm::VMError> {
        use ethrex_vm::{PrecompileError, VMError};

        // `input` was decoded by the precompile from attacker-controlled
        // calldata. Rebuilding the witness can still fail on a malformed or
        // un-rebuildable witness; that is a CALL-level failure, NOT a node
        // invariant, so map it to `PrecompileError` (the precompile halts
        // exceptionally: tx included, gas charged — the DoS bound holds) rather
        // than `VMError::Internal`, which would abort the whole tx and let the
        // attacker walk away uncharged after we already did the trie rebuild.
        let execution_witness = ExecutionWitness::from_ssz(input)
            .map_err(|_| VMError::from(PrecompileError::ExecuteInvalidInput))?;

        let result = verify_stateless_new_payload(
            &input.new_payload_request,
            execution_witness,
            &input.chain_config,
            self.crypto.clone(),
        );

        let mut buf = Vec::new();
        result.ssz_append(&mut buf);
        Ok(buf)
    }
}

// Note: the malformed/attacker-controlled-input fail-closed regression (I1) now
// lives with the SSZ decode in the EXECUTE precompile itself — see
// `execute_rejects_malformed_ssz_input` in `crates/vm/levm/src/execute_precompile.rs`.
// `verify` here receives an already-decoded `SszStatelessInput`, so it can no
// longer be exercised with raw malformed bytes.
