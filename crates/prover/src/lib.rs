pub mod backend;
pub mod protocol;
pub mod prover;

pub use crate::backend::{BackendError, BackendType, ExecBackend, ProverBackend};
pub use crate::protocol::ProofData;
pub use crate::prover::{InMessage as ProverInMessage, Prover, ProverPullConfig};

// Re-export prover types so downstream crates (e.g. proof_coordinator) can import from
// ethrex_prover without depending on ethrex_l2_common directly.
pub use ethrex_common::types::prover::{ProofBytes, ProofFormat, ProverType};
pub use ethrex_l2_common::prover::BatchProof;

#[cfg(feature = "sp1")]
pub use crate::backend::Sp1Backend;

#[cfg(feature = "risc0")]
pub use crate::backend::Risc0Backend;

#[cfg(feature = "zisk")]
pub use crate::backend::ZiskBackend;

#[cfg(feature = "openvm")]
pub use crate::backend::OpenVmBackend;
