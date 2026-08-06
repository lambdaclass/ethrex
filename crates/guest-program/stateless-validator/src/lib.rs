//! Ethrex stateless-validator guest.
//!
//! Implements the wire contract of the zkEVM stateless-validation spec: decode
//! `statelessInputBytes` (a schema-prefixed SSZ `SszStatelessInput`), run ethrex
//! stateless validation, and emit `statelessOutputBytes` (an SSZ
//! `SszStatelessValidationResult`).
//!
//! Ported from the `feat/stateless-validator-mirror` spike, with one deliberate
//! difference: the spike took eth-act's `stateless-validator-common` as a git
//! dependency for the wire types, whereas ethrex owns them natively in
//! `ethrex_common::types::stateless_ssz`. That also removes the spike's
//! `convert.rs` entirely — its 369 lines existed only to map between two mirror
//! type hierarchies, and with one hierarchy every one of those conversions is the
//! identity.
//!
//! What this crate adds on top of `ethrex_guest_program::l1` is the per-zkVM
//! machinery: a `Crypto` provider per target (see [`crypto`]) and, behind the
//! `ere` feature, the ere-platform entrypoint (see [`platform`]).
//!
//! Target: execution-specs `3c3b6f4af315b268a61e20d5a4da8aa4f24c91f0`
//! (#3248 progressive SSZ + #3278 `ChainConfig` removal).

// The zkVM crypto providers are written against `alloc` rather than `std`,
// because a guest may be built without std. Declaring the crate here makes those
// imports resolve in either configuration; it is a no-op for host builds.
extern crate alloc;

#[cfg(any(feature = "ere", feature = "zkvm-interface", feature = "openvm"))]
pub mod crypto;
#[cfg(feature = "ere")]
pub mod platform;

use std::sync::Arc;

use ethrex_crypto::Crypto;

pub use ethrex_common::types::stateless_ssz::{
    STATELESS_INPUT_SCHEMA_ID, SszStatelessInput, SszStatelessValidationResult,
};
pub use ethrex_guest_program::l1::{StatelessInputDecodeError, decode_stateless_input};

/// Run stateless validation over serialized input and return serialized output.
///
/// A thin wrapper over [`ethrex_guest_program::l1::run_stateless_guest`], which is
/// the single implementation shared by this guest, the `ExecBackend`, and the
/// ef_tests conformance comparison. Keeping one implementation is the point: a
/// second copy is what let the public-key check go missing on the EXECUTE path.
///
/// Never panics and never returns an error. A decode failure commits the all-zero
/// default result; a decodable input commits the real payload-request root,
/// `chain_id` and `schema_id` even when validation fails.
pub fn run_stateless_validation(input_bytes: &[u8], crypto: Arc<dyn Crypto>) -> Vec<u8> {
    ethrex_guest_program::l1::run_stateless_guest(input_bytes, crypto)
}
