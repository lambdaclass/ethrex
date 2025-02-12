use std::{env::temp_dir, fs::read_to_string};

use crate::errors::ProverError;
use ethrex_l2::utils::prover::proving_systems::{
    ExecuteOutput, PicoProof, ProverType, ProvingOutput, Risc0Proof, Sp1Proof,
};
use pico_sdk::vk_client::KoalaBearProveVKClient;
use tracing::info;

// risc0
use risc0_zkvm::{default_prover, ExecutorEnv, ProverOpts};
use zkvm_interface::{
    io::{ProgramInput, ProgramOutput},
    methods::ZKVM_SP1_PROGRAM_ELF,
    methods::{ZKVM_RISC0_PROGRAM_ELF, ZKVM_RISC0_PROGRAM_ID},
};

// sp1
use sp1_sdk::{ProverClient, SP1Stdin};

// pico
#[cfg(feature = "build_pico")]
use pico_sdk::client::DefaultProverClient;

/// Structure that wraps all the needed components for the RISC0 proving system
pub struct Risc0Prover<'a> {
    elf: &'a [u8],
    pub id: [u32; 8],
    pub stdout: Vec<u8>,
}

impl<'a> Default for Risc0Prover<'a> {
    fn default() -> Self {
        Self::new()
    }
}

/// Structure that wraps all the needed components for the SP1 proving system
pub struct Sp1Prover<'a> {
    elf: &'a [u8],
}

impl<'a> Default for Sp1Prover<'a> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "build_pico")]
/// Structure that wraps all the needed components for the SP1 proving system
pub struct PicoProver<'a> {
    elf: &'a [u8],
}

#[cfg(feature = "build_pico")]
impl<'a> Default for PicoProver<'a> {
    fn default() -> Self {
        Self::new()
    }
}

/// Creates a prover depending on the [ProverType]
pub fn create_prover(prover_type: ProverType) -> Box<dyn Prover> {
    match prover_type {
        ProverType::RISC0 => Box::new(Risc0Prover::new()),
        ProverType::SP1 => Box::new(Sp1Prover::new()),
        #[cfg(feature = "build_pico")]
        ProverType::Pico => Box::new(PicoProver::new()),
    }
}

/// Trait in common with all proving systems, it can be thought as the common interface.
pub trait Prover {
    /// Generates the groth16 proof
    fn prove(&mut self, input: ProgramInput) -> Result<ProvingOutput, Box<dyn std::error::Error>>;
    /// Executes without proving
    fn execute(&mut self, input: ProgramInput)
        -> Result<ExecuteOutput, Box<dyn std::error::Error>>;
    /// Verifies the proof
    fn verify(&self, proving_output: &ProvingOutput) -> Result<(), Box<dyn std::error::Error>>;
    /// Gets the EVM gas consumed by the verified block
    fn get_gas(&self) -> Result<u64, Box<dyn std::error::Error>>;
}

impl<'a> Risc0Prover<'a> {
    pub fn new() -> Self {
        Self {
            elf: ZKVM_RISC0_PROGRAM_ELF,
            id: ZKVM_RISC0_PROGRAM_ID,
            stdout: Vec::new(),
        }
    }

    pub fn get_commitment(
        &self,
        proving_output: &ProvingOutput,
    ) -> Result<ProgramOutput, Box<dyn std::error::Error>> {
        let ProvingOutput::RISC0(proof) = proving_output else {
            return Err(Box::new(ProverError::IncorrectProverType));
        };
        let commitment = proof.receipt.journal.decode()?;
        Ok(commitment)
    }
}

impl<'a> Prover for Risc0Prover<'a> {
    fn prove(&mut self, input: ProgramInput) -> Result<ProvingOutput, Box<dyn std::error::Error>> {
        let env = ExecutorEnv::builder()
            .stdout(&mut self.stdout)
            .write(&input)?
            .build()?;

        // Generate the Receipt
        let prover = default_prover();

        // Proof information by proving the specified ELF binary.
        // This struct contains the receipt along with statistics about execution of the guest
        let prove_info = prover.prove_with_opts(env, self.elf, &ProverOpts::groth16())?;

        // Extract the receipt.
        let receipt = prove_info.receipt;

        info!("Successfully generated execution receipt.");
        Ok(ProvingOutput::RISC0(Risc0Proof::new(
            receipt,
            self.id.to_vec(),
        )))
    }

    fn execute(
        &mut self,
        input: ProgramInput,
    ) -> Result<ExecuteOutput, Box<dyn std::error::Error>> {
        todo!()
    }

    fn verify(&self, proving_output: &ProvingOutput) -> Result<(), Box<dyn std::error::Error>> {
        // Verify the proof.
        let ProvingOutput::RISC0(proof) = proving_output else {
            return Err(Box::new(ProverError::IncorrectProverType));
        };
        proof.receipt.verify(self.id)?;
        Ok(())
    }

    fn get_gas(&self) -> Result<u64, Box<dyn std::error::Error>> {
        Ok(risc0_zkvm::serde::from_slice(
            self.stdout.get(..8).unwrap_or_default(), // first 8 bytes
        )?)
    }
}

impl<'a> Sp1Prover<'a> {
    pub fn new() -> Self {
        Self {
            elf: ZKVM_SP1_PROGRAM_ELF,
        }
    }
}

impl<'a> Prover for Sp1Prover<'a> {
    fn prove(&mut self, input: ProgramInput) -> Result<ProvingOutput, Box<dyn std::error::Error>> {
        let mut stdin = SP1Stdin::new();
        stdin.write(&input);

        // Generate the ProverClient
        let client = ProverClient::from_env();
        let (pk, vk) = client.setup(self.elf);

        // Proof information by proving the specified ELF binary.
        // This struct contains the receipt along with statistics about execution of the guest
        let proof = client.prove(&pk, &stdin).groth16().run()?;
        // Wrap Proof and vk
        let sp1_proof = Sp1Proof::new(proof, vk);
        info!("Successfully generated SP1Proof.");
        Ok(ProvingOutput::SP1(sp1_proof))
    }

    fn execute(
        &mut self,
        input: ProgramInput,
    ) -> Result<ExecuteOutput, Box<dyn std::error::Error>> {
        let mut stdin = SP1Stdin::new();
        stdin.write(&input);

        // Generate the ProverClient
        let client = ProverClient::new();
        let (pk, vk) = client.setup(self.elf);

        let output = client.execute(self.elf, &stdin).run()?;

        info!("Successfully executed SP1 program.");
        Ok(ExecuteOutput::SP1(output))
    }

    fn verify(&self, proving_output: &ProvingOutput) -> Result<(), Box<dyn std::error::Error>> {
        // Verify the proof.
        let ProvingOutput::SP1(complete_proof) = proving_output else {
            return Err(Box::new(ProverError::IncorrectProverType));
        };
        let client = ProverClient::from_env();
        client.verify(&complete_proof.proof, &complete_proof.vk)?;

        Ok(())
    }

    fn get_gas(&self) -> Result<u64, Box<dyn std::error::Error>> {
        todo!()
    }
}

#[cfg(feature = "build_pico")]
impl<'a> PicoProver<'a> {
    pub fn new() -> Self {
        Self {
            elf: ZKVM_RISC0_PROGRAM_ELF,
        }
    }
}

#[cfg(feature = "build_pico")]
impl<'a> Prover for PicoProver<'a> {
    fn prove(&mut self, input: ProgramInput) -> Result<ProvingOutput, Box<dyn std::error::Error>> {
        let client = DefaultProverClient::new(self.elf);
        let stdin_builder = client.get_stdin_builder();

        let output_dir = temp_dir();
        let constraints_path = output_dir.join("constraints.json");
        let groth16_witness_path = output_dir.join("groth16_witness.json");

        let proof = client.prove(output_dir)?;

        let constraints_json: serde_json::Value =
            serde_json::from_str(&read_to_string(constraints_path)?)?;
        let groth16_witness_json: serde_json::Value =
            serde_json::from_str(&read_to_string(groth16_witness_path)?)?;

        let mut constraints = Vec::new();
        let mut groth16_witness = Vec::new();

        serde_json::to_writer(&mut constraints, &constraints_json)?;
        serde_json::to_writer(&mut groth16_witness, &groth16_witness_json)?;

        info!("Successfully generated PicoProof.");
        Ok(ProvingOutput::Pico(PicoProof {
            constraints,
            groth16_witness,
        }))
    }

    fn execute(
        &mut self,
        input: ProgramInput,
    ) -> Result<ExecuteOutput, Box<dyn std::error::Error>> {
        todo!()
    }

    fn verify(&self, proving_output: &ProvingOutput) -> Result<(), Box<dyn std::error::Error>> {
        todo!()
    }

    fn get_gas(&self) -> Result<u64, Box<dyn std::error::Error>> {
        todo!()
    }
}
