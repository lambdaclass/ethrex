use std::{fmt::Debug, sync::OnceLock};

use ethrex_l2_common::{
    calldata::Value,
    prover::{BatchProof, ProofBytes, ProofCalldata, ProverType},
};
use guest_program::input::ProgramInput;
use rkyv::rancor::Error;
use sp1_prover::components::CpuProverComponents;
#[cfg(not(feature = "gpu"))]
use sp1_sdk::CpuProver;
#[cfg(feature = "gpu")]
use sp1_sdk::cuda::builder::CudaProverBuilder;
use sp1_sdk::{
    HashableKey, Prover, SP1ProofMode, SP1ProofWithPublicValues, SP1ProvingKey, SP1Stdin,
    SP1VerifyingKey,
};
use std::time::Instant;
use tracing::info;
use url::Url;

pub static PROGRAM_ELF: &[u8] =
    include_bytes!("../guest_program/src/sp1/out/riscv32im-succinct-zkvm-elf");

pub struct ProverSetup {
    client: Box<dyn Prover<CpuProverComponents>>,
    pk: SP1ProvingKey,
    vk: SP1VerifyingKey,
}

pub static PROVER_SETUP: OnceLock<ProverSetup> = OnceLock::new();

pub fn init_prover_setup(_endpoint: Option<Url>) -> ProverSetup {
    #[cfg(feature = "gpu")]
    let client = {
        if let Some(endpoint) = _endpoint {
            CudaProverBuilder::default()
                .server(&endpoint.join("/twirp/"))
                .build()
        } else {
            CudaProverBuilder::default().local().build()
        }
    };
    #[cfg(not(feature = "gpu"))]
    let client = { CpuProver::new() };
    let (pk, vk) = client.setup(PROGRAM_ELF);

    ProverSetup {
        client: Box::new(client),
        pk,
        vk,
    }
}
pub struct ProveOutput {
    pub proof: SP1ProofWithPublicValues,
    pub vk: SP1VerifyingKey,
}

// TODO: Error enum

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
    let mut stdin = SP1Stdin::new();
    let bytes = rkyv::to_bytes::<Error>(&input)?;
    stdin.write_slice(bytes.as_slice());

    let setup = PROVER_SETUP.get_or_init(|| init_prover_setup(None));

    let now = Instant::now();
    setup.client.execute(PROGRAM_ELF, &stdin)?;
    let elapsed = now.elapsed();

    info!("Successfully executed SP1 program in {:.2?}", elapsed);
    Ok(())
}

pub fn prove(
    input: ProgramInput,
    aligned_mode: bool,
) -> Result<ProveOutput, Box<dyn std::error::Error>> {
    let mut stdin = SP1Stdin::new();
    let bytes = rkyv::to_bytes::<Error>(&input)?;
    stdin.write_slice(bytes.as_slice());

    let setup = PROVER_SETUP.get_or_init(|| init_prover_setup(None));

    // contains the receipt along with statistics about execution of the guest
    let proof = if aligned_mode {
        setup
            .client
            .prove(&setup.pk, &stdin, SP1ProofMode::Compressed)?
    } else {
        setup
            .client
            .prove(&setup.pk, &stdin, SP1ProofMode::Groth16)?
    };

    info!("Successfully generated SP1Proof.");
    Ok(ProveOutput::new(proof, setup.vk.clone()))
}

pub fn verify(output: &ProveOutput) -> Result<(), Box<dyn std::error::Error>> {
    let setup = PROVER_SETUP.get_or_init(|| init_prover_setup(None));
    setup.client.verify(&output.proof, &output.vk)?;

    Ok(())
}

pub fn to_batch_proof(
    proof: ProveOutput,
    aligned_mode: bool,
) -> Result<BatchProof, Box<dyn std::error::Error>> {
    let batch_proof = if aligned_mode {
        BatchProof::ProofBytes(ProofBytes {
            proof: bincode::serialize(&proof.proof)?,
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
