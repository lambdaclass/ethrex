mod db;
mod errors;
mod execution_result;
pub mod tracing;
mod witness_db;

pub mod backends;

/// EIP-8037 (Amsterdam+, PR #2703) per-tx 2D inclusion check. Re-exported so the
/// payload builder can enforce it with identical semantics to the validator.
pub use backends::levm::check_2d_gas_allowance;
pub use backends::{BlockExecutionResult, Evm};
pub use db::{DynVmDatabase, VmDatabase};
pub use errors::EvmError;
pub use ethrex_levm::precompiles::{PrecompileCache, precompiles_for_fork};
/// EIP-8037 intrinsic gas split `(regular, state)` for a transaction.
/// Re-exported for mempool / payload-builder use.
pub use ethrex_levm::utils::intrinsic_gas_dimensions;
pub use execution_result::ExecutionResult;
pub use witness_db::GuestProgramStateWrapper;
pub mod system_contracts;
