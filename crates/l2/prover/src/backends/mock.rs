use ethrex_l2::utils::prover::proving_systems::{ProofCalldata, ProverType};
use tracing::warn;
use zkvm_interface::io::ProgramInput;

pub fn execute(input: ProgramInput) -> Result<(), Box<dyn std::error::Error>> {
    warn!("Executing with mock prover does nothing. Use a real prover by enabling the corresponding feature flag");
    Ok(())
}

pub fn prove(input: ProgramInput) -> Result<(), Box<dyn std::error::Error>> {
    warn!("Proving with mock prover does nothing. Use a real prover by enabling the corresponding feature flag");
    Ok(())
}

pub fn verify(proof: &()) -> Result<(), Box<dyn std::error::Error>> {
    warn!("Verifying with mock prover does nothing. Use a real prover by enabling the corresponding feature flag");
    Ok(())
}

pub fn to_calldata(proof: ()) -> Result<ProofCalldata, Box<dyn std::error::Error>> {
    warn!("Calldata of a mock prover is empty. Use a real prover by enabling the corresponding feature flag");
    Ok(ProofCalldata {
        prover_type: ProverType::RISC0,
        calldata: Vec::new(),
    })
}
