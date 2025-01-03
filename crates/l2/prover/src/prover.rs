use crate::errors::ProverError;
use ethrex_l2::utils::prover::proving_systems::{ProverType, ProvingOutput, Risc0Proof, Sp1Proof};
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

/// Creates a prover depending on the [ProverType]
pub fn create_prover(prover_type: ProverType) -> Box<dyn Prover> {
    match prover_type {
        ProverType::RISC0 => Box::new(Risc0Prover::new()),
        ProverType::SP1 => Box::new(Sp1Prover::new()),
    }
}

/// Trait in common with all proving systems, it can be thought as the common interface.
pub trait Prover {
    /// Generates the groth16 proof
    fn prove(&mut self, input: ProgramInput) -> Result<ProvingOutput, Box<dyn std::error::Error>>;
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
        let commitment = match proving_output {
            ProvingOutput::RISC0(proof) => proof.receipt.journal.decode()?,
            ProvingOutput::SP1(_) => return Err(Box::new(ProverError::IncorrectProverType)),
        };
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

    fn verify(&self, proving_output: &ProvingOutput) -> Result<(), Box<dyn std::error::Error>> {
        // Verify the proof.
        match proving_output {
            ProvingOutput::RISC0(proof) => proof.receipt.verify(self.id)?,
            ProvingOutput::SP1(_) => return Err(Box::new(ProverError::IncorrectProverType)),
        }

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
        let client = ProverClient::new();
        let (pk, vk) = client.setup(self.elf);

        // Proof information by proving the specified ELF binary.
        // This struct contains the receipt along with statistics about execution of the guest
        let proof = client.prove(&pk, stdin).groth16().run()?;
        // Wrap Proof and vk
        let sp1_proof = Sp1Proof::new(proof, vk);
        info!("Successfully generated SP1Proof.");
        Ok(ProvingOutput::SP1(sp1_proof))
    }

    fn verify(&self, proving_output: &ProvingOutput) -> Result<(), Box<dyn std::error::Error>> {
        // Verify the proof.
        match proving_output {
            ProvingOutput::SP1(complete_proof) => {
                let client = ProverClient::new();
                client.verify(&complete_proof.proof, &complete_proof.vk)?;
            }
            ProvingOutput::RISC0(_) => return Err(Box::new(ProverError::IncorrectProverType)),
        }

        Ok(())
    }

    fn get_gas(&self) -> Result<u64, Box<dyn std::error::Error>> {
        todo!()
    }
}
