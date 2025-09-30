use std::{fmt::Debug, sync::LazyLock};

use ethrex_l2_common::{
    calldata::Value,
    prover::{BatchProof, ProofBytes, ProofCalldata, ProverType},
};
use guest_program::input::ProgramInput;
use rkyv::rancor::Error as RkyvError;
use sp1_sdk::{
    EnvProver, HashableKey, ProverClient, SP1ProofWithPublicValues, SP1ProvingKey, SP1Stdin,
    SP1VerifyingKey,
};
use std::time::Instant;
use tracing::info;

#[cfg(not(clippy))]
static PROGRAM_ELF: &[u8] =
    include_bytes!("../guest_program/src/sp1/out/riscv32im-succinct-zkvm-elf");

// If we're running clippy, the file isn't generated.
// To avoid compilation errors, we override it with an empty slice.
#[cfg(clippy)]
static PROGRAM_ELF: &[u8] = &[];

struct ProverSetup {
    client: EnvProver,
    pk: SP1ProvingKey,
    vk: SP1VerifyingKey,
}

static PROVER_SETUP: LazyLock<ProverSetup> = LazyLock::new(|| {
    let client = ProverClient::from_env();
    let (pk, vk) = client.setup(PROGRAM_ELF);
    ProverSetup { client, pk, vk }
});

pub struct ProveOutput {
    pub proof: SP1ProofWithPublicValues,
    pub vk: SP1VerifyingKey,
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("failed to serialize input: {0}")]
    Rkyv(#[from] RkyvError),
    #[error("bincode serialization failed: {0}")]
    Bincode(#[from] bincode::Error),
    #[error("zkvm dynamic error: {0}")]
    ZkvmDyn(#[from] anyhow::Error),
    #[error("program ELF is missing")]
    MissingProgramElf,
}

impl Debug for ProveOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sp1Proof")
            .field("proof", &self.proof)
            .field("vk", &self.vk.bytes32())
            .finish()
    }
}

impl ProveOutput {
    pub fn new(proof: SP1ProofWithPublicValues, verifying_key: SP1VerifyingKey) -> Self {
        ProveOutput {
            proof,
            vk: verifying_key,
        }
    }
}

pub fn execute(input: ProgramInput) -> Result<(), Box<dyn std::error::Error>> {
    if PROGRAM_ELF.is_empty() {
        return Err(Box::new(Error::MissingProgramElf));
    }

    let mut stdin = SP1Stdin::new();
    let bytes = rkyv::to_bytes::<RkyvError>(&input)
        .map_err(|e| Box::<dyn std::error::Error>::from(Error::from(e)))?;
    stdin.write_slice(bytes.as_slice());

    let setup = &*PROVER_SETUP;

    let now = Instant::now();
    setup
        .client
        .execute(PROGRAM_ELF, &stdin)
        .run()
        .map_err(|e| Box::<dyn std::error::Error>::from(Error::from(e)))?;
    let elapsed = now.elapsed();

    info!("Successfully executed SP1 program in {:.2?}", elapsed);
    Ok(())
}

pub fn prove(
    input: ProgramInput,
    aligned_mode: bool,
) -> Result<ProveOutput, Box<dyn std::error::Error>> {
    if PROGRAM_ELF.is_empty() {
        return Err(Box::new(Error::MissingProgramElf));
    }

    let mut stdin = SP1Stdin::new();
    let bytes = rkyv::to_bytes::<RkyvError>(&input)
        .map_err(|e| Box::<dyn std::error::Error>::from(Error::from(e)))?;
    stdin.write_slice(bytes.as_slice());

    let setup = &*PROVER_SETUP;

    // contains the receipt along with statistics about execution of the guest
    let proof = if aligned_mode {
        setup
            .client
            .prove(&setup.pk, &stdin)
            .compressed()
            .run()
            .map_err(|e| Box::<dyn std::error::Error>::from(Error::from(e)))?
    } else {
        setup
            .client
            .prove(&setup.pk, &stdin)
            .groth16()
            .run()
            .map_err(|e| Box::<dyn std::error::Error>::from(Error::from(e)))?
    };

    info!("Successfully generated SP1Proof.");
    Ok(ProveOutput::new(proof, setup.vk.clone()))
}

pub fn verify(output: &ProveOutput) -> Result<(), Box<dyn std::error::Error>> {
    if PROGRAM_ELF.is_empty() {
        return Err(Box::new(Error::MissingProgramElf));
    }

    let setup = &*PROVER_SETUP;
    setup
        .client
        .verify(&output.proof, &output.vk)
        .map_err(|e| Box::<dyn std::error::Error>::from(Error::from(e)))?;

    Ok(())
}

pub fn to_batch_proof(
    proof: ProveOutput,
    aligned_mode: bool,
) -> Result<BatchProof, Box<dyn std::error::Error>> {
    let batch_proof = if aligned_mode {
        BatchProof::ProofBytes(ProofBytes {
            proof: bincode::serialize(&proof.proof)
                .map_err(|e| Box::<dyn std::error::Error>::from(Error::from(e)))?,
            public_values: proof.proof.public_values.to_vec(),
        })
    } else {
        BatchProof::ProofCalldata(to_calldata(proof))
    };

    Ok(batch_proof)
}

fn to_calldata(proof: ProveOutput) -> ProofCalldata {
    // bytes calldata publicValues,
    // bytes calldata proofBytes
    let calldata = vec![
        Value::Bytes(proof.proof.public_values.to_vec().into()),
        Value::Bytes(proof.proof.bytes().into()),
    ];

    ProofCalldata {
        prover_type: ProverType::SP1,
        calldata,
    }
}
