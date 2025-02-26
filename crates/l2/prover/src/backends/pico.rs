use std::{env::temp_dir, path::PathBuf};

use ethrex_common::U256;
use ethrex_l2::utils::prover::proving_systems::{ProofCalldata, ProverType};
use ethrex_l2_sdk::calldata::Value;
use pico_sdk::vk_client::KoalaBearProveVKClient;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{info, warn};
use zkvm_interface::{io::ProgramInput, methods::ZKVM_PICO_PROGRAM_ELF};

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

pub fn prove(input: ProgramInput) -> Result<ProveOutput, Box<dyn std::error::Error>> {
    // TODO: Determine which field is better for our use case: KoalaBear or BabyBear
    let client = KoalaBearProveVKClient::new(ZKVM_PICO_PROGRAM_ELF);

    let stdin_builder = client.get_stdin_builder();
    stdin_builder.borrow_mut().write(&input);

    let output_dir = temp_dir();

    let proof = client.prove(output_dir.clone())?;

    // assumes setup (keypair generation) was done before
    client.prove_evm(false, output_dir.clone(), "kb")?;

    info!("Successfully generated Pico proof.");
    ProveOutput::new(output_dir)
}

pub fn execute(input: ProgramInput) -> Result<(), Box<dyn std::error::Error>> {
    // TODO: Determine which field is better for our use case: KoalaBear or BabyBear
    let client = KoalaBearProveVKClient::new(ZKVM_PICO_PROGRAM_ELF);

    let stdin_builder = client.get_stdin_builder();
    stdin_builder.borrow_mut().write(&input);

    warn!("Pico doesn't implement execution only so the backend's execute() function will generate a \"fast\" proof, without entering the recursion phase");
    let proof = client.prove_fast()?;

    info!("Successfully generated Pico fast proof.");
    Ok(())
}

pub fn verify(output: &ProveOutput) -> Result<(), Box<dyn std::error::Error>> {
    warn!("Pico backend's verify() does nothing, this is because Pico doesn't expose a verification function but will verify each phase during proving as a sanity check");
    Ok(())
}

pub fn to_calldata(output: ProveOutput) -> Result<ProofCalldata, Box<dyn std::error::Error>> {
    let ProveOutput {
        public_values,
        proof,
    } = output;

    // TODO: double check big endian is correct
    let proof = proof
        .chunks(32)
        .map(|integer| Value::Int(U256::from_big_endian(integer)))
        .collect();

    // bytes calldata publicValues,
    // uint256[8] calldata proof
    let calldata = vec![Value::Bytes(public_values.into()), Value::FixedArray(proof)];

    Ok(ProofCalldata {
        prover_type: ProverType::Pico,
        calldata,
    })
}

#[allow(unused)]
fn get_gas() -> Result<u64, Box<dyn std::error::Error>> {
    todo!()
}
