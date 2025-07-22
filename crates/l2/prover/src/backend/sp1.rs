use std::{fmt::Debug, sync::LazyLock};

use ethrex_l2_common::{
    calldata::Value,
    prover::{BatchProof, ProofBytes, ProofCalldata, ProverType},
};
use sp1_sdk::{
    EnvProver, HashableKey, ProverClient, SP1ProofWithPublicValues, SP1ProvingKey, SP1Stdin,
    SP1VerificationError, SP1VerifyingKey,
};
use tracing::info;

use crate::{
    backend::ProverBackend,
    guest_program::input::{JSONProgramInput, ProgramInput},
};

static PROGRAM_ELF: &[u8] = &[0u8; 0]; // Placeholder for the actual ELF file content
// include_bytes!("../../zkvm/interface/sp1/out/riscv32im-succinct-zkvm-elf");

static PROVER_SETUP: LazyLock<SP1ProverSetup> = LazyLock::new(|| {
    let client = ProverClient::from_env();

    let (pk, vk) = client.setup(PROGRAM_ELF);

    SP1ProverSetup { client, pk, vk }
});

struct SP1ProverSetup {
    client: EnvProver,
    pk: SP1ProvingKey,
    vk: SP1VerifyingKey,
}

#[derive(Debug, thiserror::Error)]
pub enum SP1BackendError {
    #[error("Failed to execute SP1 program: {0}")]
    ProgramExecutionError(String),
    #[error("Failed to generate SP1 program output: {0}")]
    ProgramOutputError(String),
    #[error("Failed to verify SP1 program output: {0}")]
    VerificationError(#[from] SP1VerificationError),
    #[error("Failed to convert SP1 program output to batch proof: {0}")]
    ProgramOutputSerializationError(#[source] bincode::Error),
}

pub struct SP1ProgramOutput {
    pub proof: SP1ProofWithPublicValues,
    pub vk: SP1VerifyingKey,
}

impl SP1ProgramOutput {
    pub fn new(proof: SP1ProofWithPublicValues, verifying_key: SP1VerifyingKey) -> Self {
        SP1ProgramOutput {
            proof,
            vk: verifying_key,
        }
    }
}

impl Debug for SP1ProgramOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sp1Proof")
            .field("proof", &self.proof)
            .field("vk", &self.vk.bytes32())
            .finish()
    }
}

pub struct SP1Backend;

impl ProverBackend for SP1Backend {
    type ProgramOutput = SP1ProgramOutput;

    type Error = SP1BackendError;

    fn r#type(&self) -> ProverType {
        ProverType::SP1
    }

    fn execute(&self, input: ProgramInput) -> Result<(), Self::Error> {
        let mut stdin = SP1Stdin::new();

        stdin.write(&JSONProgramInput(input));

        let setup = &*PROVER_SETUP;

        setup
            .client
            .execute(PROGRAM_ELF, &stdin)
            .run()
            .map_err(|err| Self::Error::ProgramExecutionError(err.to_string()))?;

        info!("Successfully executed SP1 program");

        Ok(())
    }

    fn prove(
        &self,
        input: ProgramInput,
        aligned_mode: bool,
    ) -> Result<Self::ProgramOutput, Self::Error> {
        let mut stdin = SP1Stdin::new();

        stdin.write(&JSONProgramInput(input));

        let setup = &*PROVER_SETUP;

        // contains the receipt along with statistics about execution of the guest
        let proof = if aligned_mode {
            setup.client.prove(&setup.pk, &stdin).compressed().run()
        } else {
            setup.client.prove(&setup.pk, &stdin).groth16().run()
        }
        .map_err(|err| Self::Error::ProgramOutputError(err.to_string()))?;

        info!("Successfully generated SP1Proof");

        let proof = Self::ProgramOutput::new(proof, setup.vk.clone());

        Ok(proof)
    }

    fn verify(&self, output: Self::ProgramOutput) -> Result<(), Self::Error> {
        let setup = &*PROVER_SETUP;

        setup
            .client
            .verify(&output.proof, &output.vk)
            .map_err(Self::Error::VerificationError)
    }

    fn to_batch_proof(
        &self,
        output: Self::ProgramOutput,
        aligned_mode: bool,
    ) -> Result<BatchProof, Self::Error> {
        let batch_proof = if aligned_mode {
            BatchProof::ProofBytes(ProofBytes {
                proof: bincode::serialize(&output.proof)
                    .map_err(Self::Error::ProgramOutputSerializationError)?,
                public_values: output.proof.public_values.to_vec(),
            })
        } else {
            BatchProof::ProofCalldata(self.to_calldata(output)?)
        };

        Ok(batch_proof)
    }

    fn to_calldata(&self, output: Self::ProgramOutput) -> Result<ProofCalldata, Self::Error> {
        let calldata = ProofCalldata {
            prover_type: self.r#type(),
            // bytes calldata publicValues,
            // bytes calldata proofBytes
            calldata: vec![
                Value::Bytes(output.proof.public_values.to_vec().into()),
                Value::Bytes(output.proof.bytes().into()),
            ],
        };

        Ok(calldata)
    }
}
