//! Stateless block validation — shared between the EXECUTE precompile and zkVM guests.
//!
//! The core function mirrors `verify_stateless_new_payload` from execution-specs
//! (projects/zkevm branch). It is invoked through two entry points:
//! - the EXECUTE precompile (via the `StatelessValidator` trait), and
//! - the zkVM guest program.

use std::sync::Arc;

use ethrex_common::types::block_execution_witness::ExecutionWitness;
use ethrex_common::types::stateless_ssz::{
    NewPayloadRequest, SszChainConfig, SszStatelessInput, SszStatelessValidationResult,
};
use ethrex_crypto::Crypto;
use ethrex_guest_program::common::{ExecutionError, execute_blocks};
use ethrex_guest_program::l1::new_payload_request_to_block;
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
    use ethrex_common::types::ELASTICITY_MULTIPLIER;
    use ethrex_vm::Evm;

    let block = new_payload_request_to_block(new_payload_request, crypto.as_ref())
        .map_err(|e| ExecutionError::Internal(format!("payload conversion: {e}")))?;

    // Validate block_hash
    let computed_hash = block.hash();
    let expected_hash =
        ethrex_common::H256::from_slice(&new_payload_request.execution_payload.block_hash);
    if computed_hash != expected_hash {
        return Err(ExecutionError::Internal(format!(
            "block_hash mismatch: expected {expected_hash:?}, got {computed_hash:?}"
        )));
    }

    // Validate blob versioned hashes
    validate_versioned_hashes(&block, new_payload_request)?;

    // Execute statelessly
    let _result = execute_blocks(
        &[block],
        execution_witness,
        ELASTICITY_MULTIPLIER,
        |db, _| Ok(Evm::new_for_l1(db.clone(), crypto.clone())),
        crypto.clone(),
    )?;

    Ok(())
}

fn validate_versioned_hashes(
    block: &ethrex_common::types::Block,
    req: &NewPayloadRequest,
) -> Result<(), ExecutionError> {
    use ethrex_common::H256;

    let tx_hashes: Vec<H256> = block
        .body
        .transactions
        .iter()
        .flat_map(|tx| tx.blob_versioned_hashes())
        .collect();

    let req_hashes: Vec<H256> = req
        .versioned_hashes
        .iter()
        .map(|h| H256::from_slice(h))
        .collect();

    if tx_hashes != req_hashes {
        return Err(ExecutionError::Internal(
            "versioned hashes mismatch".to_string(),
        ));
    }

    Ok(())
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
/// implementation, and an `Arc<dyn StatelessValidator>` is injected into
/// `VM::new` from blockchain at runtime (see `blockchain.rs` VM construction
/// sites).
pub struct StatelessExecutor {
    pub crypto: Arc<dyn Crypto>,
}

impl ethrex_vm::StatelessValidator for StatelessExecutor {
    fn verify(&self, input: &[u8]) -> Result<Vec<u8>, ethrex_vm::VMError> {
        use ethrex_vm::{InternalError, VMError};
        use libssz::SszDecode;

        let stateless_input = SszStatelessInput::from_ssz_bytes(input)
            .map_err(|e| VMError::Internal(InternalError::Custom(format!("SSZ decode: {e}"))))?;

        let execution_witness = ExecutionWitness::from_ssz(&stateless_input).map_err(|e| {
            VMError::Internal(InternalError::Custom(format!("witness conversion: {e}")))
        })?;

        let result = verify_stateless_new_payload(
            &stateless_input.new_payload_request,
            execution_witness,
            &stateless_input.chain_config,
            self.crypto.clone(),
        );

        let mut buf = Vec::new();
        result.ssz_append(&mut buf);
        Ok(buf)
    }
}

