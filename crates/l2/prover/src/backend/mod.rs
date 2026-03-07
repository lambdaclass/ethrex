// Re-export everything from the shared prover backend crate.
pub use ethrex_prover_backend::backend::*;

// Re-export backend submodules so that paths like `crate::backend::sp1::*` still work.
pub use ethrex_prover_backend::backend::exec;

#[cfg(feature = "risc0")]
pub use ethrex_prover_backend::backend::risc0;

#[cfg(feature = "sp1")]
pub use ethrex_prover_backend::backend::sp1;

#[cfg(feature = "zisk")]
pub use ethrex_prover_backend::backend::zisk;

#[cfg(feature = "openvm")]
pub use ethrex_prover_backend::backend::openvm;

pub use ethrex_prover_backend::backend::error;
