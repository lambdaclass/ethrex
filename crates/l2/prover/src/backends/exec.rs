use ethrex_l2::utils::prover::proving_systems::{ProofCalldata, ProverType};
use ethrex_l2_sdk::calldata::Value;
use tracing::warn;

use zkvm_interface::io::{ProgramInput, ProgramOutput};

#[cfg(feature = "l2")]
use ethrex_common::types::blobs_bundle::{blob_from_bytes, kzg_commitment_to_versioned_hash};
#[cfg(feature = "l2")]
use ethrex_l2_common::{
    compute_deposit_logs_hash, compute_withdrawals_merkle_root, get_block_deposits,
    get_block_withdrawal_hashes,
};

pub struct ProveOutput(pub ProgramOutput);

pub fn execute(input: ProgramInput) -> Result<(), Box<dyn std::error::Error>> {
    execution_program(input)?;
    Ok(())
}

pub fn prove(input: ProgramInput) -> Result<ProveOutput, Box<dyn std::error::Error>> {
    warn!("\"exec\" prover backend generates no proof, only executes");
    let output = execution_program(input)?;
    Ok(ProveOutput(output))
}

pub fn verify(_proof: &ProveOutput) -> Result<(), Box<dyn std::error::Error>> {
    warn!("\"exec\" prover backend generates no proof, verification always succeeds");
    Ok(())
}

pub fn to_calldata(proof: ProveOutput) -> Result<ProofCalldata, Box<dyn std::error::Error>> {
    let public_inputs = proof.0.encode();
    Ok(ProofCalldata {
        prover_type: ProverType::Exec,
        calldata: vec![Value::Bytes(public_inputs.into())],
    })
}

pub fn execution_program(input: ProgramInput) -> Result<ProgramOutput, Box<dyn std::error::Error>> {
    zkvm_interface::execution::execution_program(input).map_err(|e| e.into())
}
