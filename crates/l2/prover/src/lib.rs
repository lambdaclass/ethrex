pub mod backend;
pub mod prover;

pub mod config;
use config::ProverConfig;
use ethrex_l2_common::prover::BatchProof;
use guest_program::input::ProgramInput;
use tracing::warn;

use crate::backend::{Backend, ProveOutput};

pub async fn init_client(config: ProverConfig) {
    prover::start_prover(config).await;
    warn!("Prover finished!");
}

/// Execute a program using the specified backend with panic handling.
/// 
/// This function wraps backend execution calls with panic catching to ensure
/// that panics from zkVM backends are converted to proper error results.
pub fn execute(backend: Backend, input: ProgramInput) -> Result<(), Box<dyn std::error::Error>> {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    
    // Catch any panics that might occur during backend execution
    let result = catch_unwind(AssertUnwindSafe(|| {
        match backend {
            Backend::Exec => backend::exec::execute(input),
            #[cfg(feature = "sp1")]
            Backend::SP1 => backend::sp1::execute(input),
            #[cfg(feature = "risc0")]
            Backend::RISC0 => backend::risc0::execute(input),
        }
    }));
    
    match result {
        Ok(exec_result) => exec_result,
        Err(panic_info) => {
            // Extract meaningful panic message
            let panic_msg = extract_panic_message(&panic_info);
            
            Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Backend {:?} panicked during execution: {}", backend, panic_msg)
            )))
        }
    }
}

/// Generate a proof using the specified backend with panic handling.
/// 
/// This function wraps backend proving calls with panic catching to ensure
/// that panics from zkVM backends are converted to proper error results.
pub fn prove(
    backend: Backend,
    input: ProgramInput,
    aligned_mode: bool,
) -> Result<ProveOutput, Box<dyn std::error::Error>> {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    
    // Catch any panics that might occur during backend proving
    let result = catch_unwind(AssertUnwindSafe(|| {
        match backend {
            Backend::Exec => backend::exec::prove(input, aligned_mode).map(ProveOutput::Exec),
            #[cfg(feature = "sp1")]
            Backend::SP1 => backend::sp1::prove(input, aligned_mode).map(ProveOutput::SP1),
            #[cfg(feature = "risc0")]
            Backend::RISC0 => backend::risc0::prove(input, aligned_mode).map(ProveOutput::RISC0),
        }
    }));
    
    match result {
        Ok(prove_result) => prove_result,
        Err(panic_info) => {
            // Extract meaningful panic message
            let panic_msg = extract_panic_message(&panic_info);
            
            Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Backend {:?} panicked during proving: {}", backend, panic_msg)
            )))
        }
    }
}

pub fn to_batch_proof(
    proof: ProveOutput,
    aligned_mode: bool,
) -> Result<BatchProof, Box<dyn std::error::Error>> {
    match proof {
        ProveOutput::Exec(proof) => backend::exec::to_batch_proof(proof, aligned_mode),
        #[cfg(feature = "sp1")]
        ProveOutput::SP1(proof) => backend::sp1::to_batch_proof(proof, aligned_mode),
        #[cfg(feature = "risc0")]
        ProveOutput::RISC0(receipt) => backend::risc0::to_batch_proof(receipt, aligned_mode),
    }
}

/// Extract a meaningful error message from panic information.
/// 
/// This helper function attempts to downcast the panic payload to common types
/// and returns a readable error message.
pub fn extract_panic_message(panic_info: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = panic_info.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = panic_info.downcast_ref::<&str>() {
        s.to_string()
    } else {
        "Unknown panic occurred".to_string()
    }
}
