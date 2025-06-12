use ethrex_l2_common::{calldata::Value, prover::{BatchProof, ProofBytes, ProofCalldata, ProverType}};
use tracing::warn;

use zkvm_interface::io::{ProgramInput, ProgramOutput};

pub struct ProveOutput(pub ProgramOutput);

pub fn execute(input: ProgramInput) -> Result<(), Box<dyn std::error::Error>> {
    execution_program(input)?;
    Ok(())
}

pub fn prove(
    input: ProgramInput,
    _aligned_mode: bool,
) -> Result<ProveOutput, Box<dyn std::error::Error>> {
    warn!("\"exec\" prover backend generates no proof, only executes");
    let output = execution_program(input)?;
    Ok(ProveOutput(output))
}

pub fn verify(_proof: &ProveOutput) -> Result<(), Box<dyn std::error::Error>> {
    warn!("\"exec\" prover backend generates no proof, verification always succeeds");
    Ok(())
}

fn to_calldata(proof: ProveOutput) -> ProofCalldata {
    let public_inputs = proof.0.encode();
    ProofCalldata {
        prover_type: ProverType::Exec,
        calldata: vec![Value::Bytes(public_inputs.into())],
    }
}

pub fn to_batch_proof(
    proof: ProveOutput,
    _aligned_mode: bool,
) -> Result<BatchProof, Box<dyn std::error::Error>> {
    Ok(BatchProof::ProofCalldata(to_calldata(proof)))
}

pub fn execution_program(input: ProgramInput) -> Result<ProgramOutput, Box<dyn std::error::Error>> {
    zkvm_interface::execution::execution_program(input).map_err(|e| e.into())
}
