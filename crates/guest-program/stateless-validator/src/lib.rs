//! Ethrex stateless-validator guest logic.
//!
//! Implements the stateless-validator wire contract: decode
//! `statelessInputBytes` (schema-prefixed SSZ [`StatelessInput`]), run ethrex
//! stateless validation, and encode `statelessOutputBytes` (SSZ
//! [`StatelessValidationResult`]). The wire types are owned by
//! `stateless-validator-common`; this crate converts them into the ethrex
//! EIP-8025 containers and drives the validation in
//! `ethrex_guest_program::l1`.
//!
//! The crate is platform-agnostic: zkVM entrypoints provide input bytes and a
//! [`Crypto`] implementation, and commit the returned output bytes. The
//! staged functions ([`decode_input`], [`new_payload_request_root`],
//! [`verify_stateless_new_payload`]) are public so entrypoints can wrap each
//! stage with their own cycle-count instrumentation without changing the
//! output.

use std::sync::Arc;

use ethrex_crypto::Crypto;
use ethrex_guest_program::common::ExecutionError;
use ethrex_guest_program::l1::{
    DecodedEip8025, validate_eip8025_canonical_execution, validate_eip8025_execution,
};
use stateless_validator_common::{HashTreeRoot, Sha256Hasher, SszEncode as _};

pub use stateless_validator_common::guest::{
    StatelessInput, StatelessValidationResult, input::ProtocolFork,
};

mod convert;
mod error;

pub use error::Error;

/// Runs the stateless validation over serialized input and returns serialized
/// output, mirroring `run_stateless_guest` in the spec.
pub fn run_stateless_validation(input_bytes: &[u8], crypto: Arc<dyn Crypto>) -> Vec<u8> {
    let Ok((fork, input)) = decode_input(input_bytes) else {
        return StatelessValidationResult::default().to_ssz();
    };

    let new_payload_request_root = new_payload_request_root(&input, crypto.clone());
    let chain_config = input.chain_config.clone();

    let successful_validation = verify_stateless_new_payload(fork, input, crypto).is_ok();

    StatelessValidationResult::new(
        new_payload_request_root,
        successful_validation,
        chain_config,
    )
    .to_ssz()
}

/// Decodes the schema-prefixed SSZ `statelessInputBytes`.
pub fn decode_input(
    input_bytes: &[u8],
) -> Result<(ProtocolFork, StatelessInput), stateless_validator_common::guest::Error> {
    StatelessInput::from_schema_prefixed_ssz(input_bytes)
}

/// Computes the `NewPayloadRequest` hash tree root committed in the output.
pub fn new_payload_request_root(input: &StatelessInput, crypto: Arc<dyn Crypto>) -> [u8; 32] {
    input
        .new_payload_request
        .hash_tree_root(&CryptoSha256Hasher(crypto))
}

/// Statelessly validates the execution payload, mirroring
/// `verify_stateless_new_payload` in the spec.
pub fn verify_stateless_new_payload(
    fork: ProtocolFork,
    input: StatelessInput,
    crypto: Arc<dyn Crypto>,
) -> Result<(), Error> {
    input.chain_config.validate(&input.new_payload_request)?;

    #[cfg(debug_assertions)]
    let new_payload_request_root = new_payload_request_root(&input, crypto.clone());

    let ethrex_input = convert::to_ethrex_input(fork, input, crypto.as_ref())?;

    #[cfg(debug_assertions)]
    if fork == ProtocolFork::Amsterdam {
        let ethrex_root = ethrex_new_payload_request_root(&ethrex_input, crypto.clone());
        assert_eq!(ethrex_root, new_payload_request_root);
    }

    run_validation(ethrex_input, crypto)?;

    Ok(())
}

/// Validates the decoded payload through its canonical or legacy execution
/// path, reporting a rejected payload as an error.
fn run_validation(
    ethrex_input: DecodedEip8025,
    crypto: Arc<dyn Crypto>,
) -> Result<(), ExecutionError> {
    match ethrex_input {
        DecodedEip8025::Legacy {
            new_payload_request,
            execution_witness,
        } => validate_eip8025_execution(&new_payload_request, execution_witness, crypto),
        DecodedEip8025::Canonical {
            stateless_input,
            chain_config,
        } => validate_eip8025_canonical_execution(stateless_input, chain_config, crypto),
    }
}

#[cfg(debug_assertions)]
fn ethrex_new_payload_request_root(
    ethrex_input: &DecodedEip8025,
    crypto: Arc<dyn Crypto>,
) -> [u8; 32] {
    let hasher = CryptoSha256Hasher(crypto);
    match ethrex_input {
        DecodedEip8025::Legacy {
            new_payload_request,
            ..
        } => new_payload_request.hash_tree_root(&hasher),
        DecodedEip8025::Canonical {
            stateless_input, ..
        } => stateless_input.new_payload_request.hash_tree_root(&hasher),
    }
}

/// Bridges [`Crypto`] to the [`Sha256Hasher`] used by `hash_tree_root`, so
/// SSZ merkleization runs through the zkVM's accelerated sha256 when one is
/// active.
struct CryptoSha256Hasher(Arc<dyn Crypto>);

impl Sha256Hasher for CryptoSha256Hasher {
    fn hash(&self, data: &[u8]) -> [u8; 32] {
        self.0.sha256(data)
    }
}
