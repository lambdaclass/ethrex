// SP1/Risc0/OpenVM/Zisk backends serialize `ProgramInput` via rkyv, which the
// `eip-8025` variant of `ProgramInput` doesn't implement. Combining them would
// fail to compile in `serialize_input`; reject it explicitly.
#[cfg(all(
    feature = "eip-8025",
    any(
        feature = "sp1",
        feature = "risc0",
        feature = "openvm",
        feature = "zisk"
    )
))]
compile_error!("feature `eip-8025` cannot be combined with `sp1`, `risc0`, `openvm`, or `zisk`");

pub mod backend;
pub mod protocol;
pub mod prover;

pub use crate::backend::{BackendError, BackendType, ExecBackend, ProverBackend};
pub use crate::protocol::ProofData;
pub use crate::prover::{Prover, ProverPullConfig};

// Re-export prover types so downstream crates (e.g. proof_coordinator) can import from
// ethrex_prover without depending on ethrex_common directly.
pub use ethrex_common::types::prover::{ProofBytes, ProofFormat, ProverOutput, ProverType};

#[cfg(feature = "sp1")]
pub use crate::backend::Sp1Backend;

#[cfg(feature = "risc0")]
pub use crate::backend::Risc0Backend;

#[cfg(feature = "zisk")]
pub use crate::backend::ZiskBackend;

#[cfg(feature = "openvm")]
pub use crate::backend::OpenVmBackend;
