//! Ethrex stateless-validator guest.
//!
//! Implements the wire contract of the zkEVM stateless-validation spec: decode
//! `statelessInputBytes` (a schema-prefixed SSZ `SszStatelessInput`), run ethrex
//! stateless validation, and emit `statelessOutputBytes` (an SSZ
//! `SszStatelessValidationResult`).
//!
//! The validation itself lives in `ethrex_guest_program::l1`, shared with the
//! `ExecBackend` and the ef_tests conformance comparison — a second copy is what
//! let the public-key check go missing on the EXECUTE path. What this crate adds
//! is the per-zkVM machinery: a `Crypto` provider per target (see [`crypto`])
//! and, behind the `ere` feature, the ere-platform entrypoint (see [`platform`]).
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

pub use ethrex_common::types::stateless_ssz::{
    STATELESS_INPUT_SCHEMA_ID, SszStatelessInput, SszStatelessValidationResult,
};
pub use ethrex_guest_program::l1::{
    StatelessInputDecodeError, decode_stateless_input,
    run_stateless_guest as run_stateless_validation,
};
