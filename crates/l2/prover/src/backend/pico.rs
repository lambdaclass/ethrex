use std::{env::temp_dir, path::PathBuf};

use ethrex_common::U256;
use rkyv::rancor::Error;
use ethrex_l2_common::{
    calldata::Value,
    prover::{BatchProof, ProofBytes, ProofCalldata, ProofFormat, ProverType},
};
use pico_sdk::client::DefaultProverClient;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{info, warn};
use guest_program::{ZKVM_PICO_PROGRAM_ELF, input::ProgramInput};

#[derive(Debug, Error)]
pub enum PicoBackendError {
    #[error("proof byte count ({0}) isn't the expected (256)")]
    ProofLen(usize),
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ProveOutput {
    pub public_values: Vec<u8>,
    pub proof: Vec<u8>,
}

impl ProveOutput {
    pub fn new(output_dir: PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let public_values = std::fs::read(output_dir.join("pv_file"))?;
        let proof = std::fs::read(output_dir.join("proof.data"))?;

        // uint256[8]
        if proof.len() != 256 {
            return Err(Box::new(PicoBackendError::ProofLen(proof.len())));
        }

        Ok(ProveOutput {
            public_values,
            proof,
        })
    }
}

pub fn prove(
    input: ProgramInput,
    _format: ProofFormat,
) -> Result<ProveOutput, Box<dyn std::error::Error>> {
    // TODO: Determine which field is better for our use case: KoalaBear or BabyBear
    let client = DefaultProverClient::new(ZKVM_PICO_PROGRAM_ELF);

    let mut stdin = client.new_stdin_builder();
    let input_bytes = rkyv::to_bytes::<Error>(&input)?;
    stdin.write_slice(&input_bytes);

    let output_dir = temp_dir();

    client.prove(stdin)?;

    ProveOutput::new(output_dir)
}

pub fn execute(input: ProgramInput) -> Result<(), Box<dyn std::error::Error>> {
    // TODO: Determine which field is better for our use case: KoalaBear or BabyBear
    let client = DefaultProverClient::new(ZKVM_PICO_PROGRAM_ELF);

    let mut stdin = client.new_stdin_builder();
    let input_bytes = rkyv::to_bytes::<Error>(&input)?;
    stdin.write_slice(&input_bytes);

    client.emulate(stdin);
    Ok(())
}

pub fn verify(_output: &ProveOutput) -> Result<(), Box<dyn std::error::Error>> {
    warn!("Pico backend's verify() does nothing, this is because Pico doesn't expose a verification function but will verify each phase during proving as a sanity check");
    Ok(())
}

pub fn to_batch_proof(
    proof: ProveOutput,
    _format: ProofFormat,
) -> Result<BatchProof, Box<dyn std::error::Error>> {
    Ok(BatchProof::ProofCalldata(to_calldata(proof)))
}

fn to_calldata(output: ProveOutput) -> ProofCalldata {
    unimplemented!();
    // let ProveOutput {
    //     public_values,
    //     proof,
    // } = output;

    // // TODO: double check big endian is correct
    // let proof = proof
    //     .chunks(32)
    //     .map(|integer| Value::Int(U256::from_big_endian(integer)))
    //     .collect();

    // // bytes calldata publicValues,
    // // uint256[8] calldata proof
    // let calldata = vec![Value::Bytes(public_values.into()), Value::FixedArray(proof)];

    // ProofCalldata {
    //     prover_type: ProverType::Pico,
    //     calldata,
    // }
}
