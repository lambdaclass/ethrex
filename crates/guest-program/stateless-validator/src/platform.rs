//! ere-platform entrypoint for the stateless-validator guest.
//!
//! Per-zkVM bins call [`entrypoint`] with their `Platform` implementation, which
//! supplies the input/output plumbing and cycle-count instrumentation; the crypto
//! provider is selected by cargo feature instead (see [`crate::crypto`]).
//!
//! Taking `ere-platform-core` rather than hand-mirroring each zkVM's read/write
//! convention makes the IO contract with `ere-server` structural. The scope names
//! below appear in ZisK profiling output, so keep them stable.

use ethrex_crypto::Crypto;
use libssz::SszEncode as _;
use std::sync::Arc;

pub use ere_platform_core::Platform;

use crate::{SszStatelessInput, SszStatelessValidationResult};

/// Runs the stateless guest on the [`Platform`].
pub fn entrypoint<P: Platform>() {
    let input_bytes = P::cycle_scope("read_input", || P::read_input());
    let output_bytes = run_stateless_guest::<P>(&input_bytes);
    P::cycle_scope("write_output", || P::write_output(&output_bytes));
}

/// Runs the stateless guest with serialized input and returns serialized output,
/// mirroring `run_stateless_guest` in the spec, with per-stage cycle scopes.
///
/// Structurally identical to [`crate::run_stateless_validation`] — the only
/// difference is the instrumentation. `tests/platform_parity.rs` asserts the two
/// produce byte-identical output, so the scopes cannot silently change behaviour.
pub fn run_stateless_guest<P: Platform>(input_bytes: &[u8]) -> Vec<u8> {
    let crypto = crate::crypto::crypto();

    let Ok(input) = P::cycle_scope("deserialize_input", || {
        crate::decode_stateless_input(input_bytes)
    }) else {
        let mut out = Vec::new();
        SszStatelessValidationResult::default().ssz_append(&mut out);
        return out;
    };

    let new_payload_request_root = P::cycle_scope("new_payload_request_root", || {
        new_payload_request_root(&input, crypto.clone())
    });
    let chain_id = input.chain_id;

    // No `validate_chain_config` scope: #3278 removed all chain configuration
    // from the wire, so there is nothing host-supplied left to validate. The fork
    // is fixed by the schema id and the config is derived inside the validation.
    let successful_validation = verify_stateless_new_payload::<P>(&input, crypto).is_ok();

    let output = SszStatelessValidationResult {
        new_payload_request_root,
        successful_validation,
        chain_id,
        schema_id: crate::STATELESS_INPUT_SCHEMA_ID,
    };

    P::cycle_scope("serialize_output", || {
        let mut out = Vec::new();
        output.ssz_append(&mut out);
        out
    })
}

/// Computes the payload-request root committed in the output.
fn new_payload_request_root(input: &SszStatelessInput, crypto: Arc<dyn Crypto>) -> [u8; 32] {
    use libssz_merkle::{HashTreeRoot, Sha256Hasher};

    struct CryptoHasher(Arc<dyn Crypto>);
    impl Sha256Hasher for CryptoHasher {
        fn hash(&self, data: &[u8]) -> [u8; 32] {
            self.0.sha256(data)
        }
    }

    input
        .new_payload_request
        .hash_tree_root(&CryptoHasher(crypto))
}

/// Statelessly validates the execution payload, with a cycle scope around the
/// expensive part.
fn verify_stateless_new_payload<P: Platform>(
    input: &SszStatelessInput,
    crypto: Arc<dyn Crypto>,
) -> Result<(), ethrex_guest_program::common::ExecutionError> {
    P::cycle_scope("run_validation", || {
        ethrex_guest_program::l1::validate_stateless_execution(input, crypto).map_err(|err| {
            P::print(&format!("Validation failed: {err}\n"));
            err
        })
    })
}
