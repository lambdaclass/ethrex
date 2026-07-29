//! Ethrex stateless-validator guest logic.
//!
//! Implements the stateless-validator wire contract: decode
//! `statelessInputBytes` (schema-prefixed SSZ [`StatelessInput`]), run ethrex
//! stateless validation, and encode `statelessOutputBytes` (SSZ
//! [`StatelessValidationResult`]). The wire types are owned by
//! `stateless-validator-common`; this crate converts them into the ethrex
//! EIP-8025 containers and drives the validation in
//! `ethrex_guest_program::l1`.

pub use stateless_validator_common::guest::{StatelessInput, StatelessValidationResult};
