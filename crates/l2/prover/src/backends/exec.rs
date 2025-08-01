use std::time::Instant;
use tracing::{info, warn};
use zkvm_interface::io::{ProgramInput, ProgramOutput};

use ethrex_l2_common::{
    calldata::Value,
    prover::{BatchProof, ProofCalldata, ProverType},
};

pub fn execute(input: ProgramInput) -> Result<(), Box<dyn std::error::Error>> {
    let now = Instant::now();
    execution_program(input)?;
    let elapsed = now.elapsed();

    info!("Successfully executed program in {:.2?}", elapsed);
    Ok(())
}

pub fn prove(
    input: ProgramInput,
    _aligned_mode: bool,
) -> Result<ProgramOutput, Box<dyn std::error::Error>> {
    warn!("\"exec\" prover backend generates no proof, only executes");
    let output = execution_program(input)?;
    Ok(output)
}

pub fn verify(_proof: &ProgramOutput) -> Result<(), Box<dyn std::error::Error>> {
    warn!("\"exec\" prover backend generates no proof, verification always succeeds");
    Ok(())
}

fn to_calldata(proof: ProgramOutput) -> ProofCalldata {
    let public_inputs = proof.encode();
    ProofCalldata {
        prover_type: ProverType::Exec,
        calldata: vec![Value::Bytes(public_inputs.into())],
    }
}

pub fn to_batch_proof(
    proof: ProgramOutput,
    _aligned_mode: bool,
) -> Result<BatchProof, Box<dyn std::error::Error>> {
    Ok(BatchProof::ProofCalldata(to_calldata(proof)))
}

pub fn execution_program(input: ProgramInput) -> Result<ProgramOutput, Box<dyn std::error::Error>> {
    zkvm_interface::execution::execution_program(input).map_err(|e| e.into())
}
