//! ere-platform entrypoint for the stateless-validator guest.
//!
//! Mirrors the ere-guests guest entrypoint: per-zkVM bins call
//! [`entrypoint`] with their `Platform` implementation, which supplies
//! input/output plumbing and cycle-count instrumentation, while the crypto
//! provider is selected by cargo feature (see [`crate::crypto`]).

use std::sync::Arc;

pub use ere_platform_core::Platform;
use ethrex_crypto::Crypto;
use stateless_validator_common::SszEncode as _;

use crate::{Error, ProtocolFork, StatelessInput, StatelessValidationResult};

/// Runs the stateless guest on the [`Platform`].
pub fn entrypoint<P: Platform>() {
    let input_bytes = P::cycle_scope("read_input", || P::read_input());
    let output_bytes = run_stateless_guest::<P>(&input_bytes);
    P::cycle_scope("write_output", || P::write_output(&output_bytes));
}

/// Runs the stateless guest with serialized input and returns serialized
/// output, mirroring `run_stateless_guest` in the spec.
pub fn run_stateless_guest<P: Platform>(input_bytes: &[u8]) -> Vec<u8> {
    let crypto = crate::crypto::crypto();

    let Ok((fork, input)) =
        P::cycle_scope("deserialize_input", || crate::decode_input(input_bytes))
    else {
        return StatelessValidationResult::default().to_ssz();
    };

    let new_payload_request_root = P::cycle_scope("new_payload_request_root", || {
        crate::new_payload_request_root(&input, crypto.clone())
    });
    let chain_config = input.chain_config.clone();

    let successful_validation =
        verify_stateless_new_payload::<P>(fork, input, crypto).is_ok();

    let output = StatelessValidationResult::new(
        new_payload_request_root,
        successful_validation,
        chain_config,
    );

    P::cycle_scope("serialize_output", || output.to_ssz())
}

/// Statelessly validates the execution payload, mirroring
/// `verify_stateless_new_payload` in the spec, with per-stage cycle scopes.
fn verify_stateless_new_payload<P: Platform>(
    fork: ProtocolFork,
    input: StatelessInput,
    crypto: Arc<dyn Crypto>,
) -> Result<(), Error> {
    P::cycle_scope("validate_chain_config", || {
        input.chain_config.validate(&input.new_payload_request)
    })?;

    #[cfg(debug_assertions)]
    let new_payload_request_root = crate::new_payload_request_root(&input, crypto.clone());

    let ethrex_input = P::cycle_scope("to_ethrex_input", || {
        crate::convert::to_ethrex_input(fork, input, crypto.as_ref()).map_err(|err| {
            P::print(&format!("Input conversion failed: {err}\n"));
            err
        })
    })?;

    #[cfg(debug_assertions)]
    if fork == ProtocolFork::Amsterdam {
        let ethrex_root = crate::ethrex_new_payload_request_root(&ethrex_input, crypto.clone());
        assert_eq!(ethrex_root, new_payload_request_root);
    }

    P::cycle_scope("run_validation", || {
        crate::run_validation(ethrex_input, crypto).map_err(|err| {
            P::print(&format!("Validation failed: {err}\n"));
            err
        })
    })?;

    Ok(())
}
