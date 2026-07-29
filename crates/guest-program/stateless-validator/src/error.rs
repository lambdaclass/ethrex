//! Errors for the stateless-validator guest.

use ethrex_common::types::block_execution_witness::GuestProgramStateError;
use ethrex_guest_program::common::ExecutionError;
use thiserror::Error;

/// Errors for the stateless-validator guest. Each variant tags the point at
/// which conversion or validation fails rather than carrying diagnostic
/// detail.
#[derive(Debug, Error)]
pub enum Error {
    /// Shared guest validation failed.
    #[error(transparent)]
    Common(#[from] stateless_validator_common::guest::Error),
    /// The blob target exceeded the ethrex `u32` bound.
    #[error("blob target out of bounds")]
    BlobTargetOutOfBounds,
    /// The blob max exceeded the ethrex `u32` bound.
    #[error("blob max out of bounds")]
    BlobMaxOutOfBounds,
    /// The payload variant has no ethrex execution path.
    #[error("unsupported payload")]
    UnsupportedPayload,
    /// The witness did not build into the state tries.
    #[error(transparent)]
    GuestProgramStateError(#[from] GuestProgramStateError),
    /// The ethrex execution path rejected the payload.
    #[error(transparent)]
    ExecutionError(#[from] ExecutionError),
}
