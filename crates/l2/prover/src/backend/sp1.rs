use ethrex_l2_common::{
    calldata::Value,
    prover::{BatchProof, ProofBytes, ProofCalldata, ProofFormat, ProverType},
};
use guest_program::{ZKVM_SP1_PROGRAM_ELF, input::ProgramInput};
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
use std::{
    fmt::Debug,
    sync::OnceLock,
    time::{Duration, Instant},
};
use tracing::info;
use url::Url;

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
                .server(
                    #[expect(clippy::expect_used)]
                    endpoint
                        .join("/twirp/")
                        .expect("Failed to parse moongate server url")
                        .as_ref(),
                )
                .build()
        } else {
            CudaProverBuilder::default().local().build()
        }
    };
    #[cfg(not(feature = "gpu"))]
    let client = { CpuProver::new() };
    let (pk, vk) = client.setup(ZKVM_SP1_PROGRAM_ELF);

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

    setup.client.execute(ZKVM_SP1_PROGRAM_ELF, &stdin)?;

    Ok(())
}

pub fn execute_timed(input: ProgramInput) -> Result<Duration, Box<dyn std::error::Error>> {
    let mut stdin = SP1Stdin::new();
    let bytes = rkyv::to_bytes::<Error>(&input)?;
    stdin.write_slice(bytes.as_slice());

    let setup = PROVER_SETUP.get_or_init(|| init_prover_setup(None));

    let start = Instant::now();
    setup.client.execute(ZKVM_SP1_PROGRAM_ELF, &stdin)?;
    let duration = start.elapsed();

    Ok(duration)
}

pub fn prove(
    input: ProgramInput,
    format: ProofFormat,
) -> Result<ProveOutput, Box<dyn std::error::Error>> {
    let mut stdin = SP1Stdin::new();
    let bytes = rkyv::to_bytes::<Error>(&input)?;
    stdin.write_slice(bytes.as_slice());

    let setup = PROVER_SETUP.get_or_init(|| init_prover_setup(None));

    let format = match format {
        ProofFormat::Compressed => SP1ProofMode::Compressed,
        ProofFormat::Groth16 => SP1ProofMode::Groth16,
    };

    let proof = setup.client.prove(&setup.pk, &stdin, format)?;

    Ok(ProveOutput::new(proof, setup.vk.clone()))
}

pub fn prove_timed(
    input: ProgramInput,
    format: ProofFormat,
) -> Result<(ProveOutput, Duration), Box<dyn std::error::Error>> {
    let mut stdin = SP1Stdin::new();
    let bytes = rkyv::to_bytes::<Error>(&input)?;
    stdin.write_slice(bytes.as_slice());

    let setup = PROVER_SETUP.get_or_init(|| init_prover_setup(None));

    let format = match format {
        ProofFormat::Compressed => SP1ProofMode::Compressed,
        ProofFormat::Groth16 => SP1ProofMode::Groth16,
    };

    let start = Instant::now();
    let proof = setup.client.prove(&setup.pk, &stdin, format)?;
    let duration = start.elapsed();

    Ok((ProveOutput::new(proof, setup.vk.clone()), duration))
}

pub fn verify(output: &ProveOutput) -> Result<(), Box<dyn std::error::Error>> {
    let setup = PROVER_SETUP.get_or_init(|| init_prover_setup(None));
    setup.client.verify(&output.proof, &output.vk)?;

    Ok(())
}

pub fn to_batch_proof(
    proof: ProveOutput,
    format: ProofFormat,
) -> Result<BatchProof, Box<dyn std::error::Error>> {
    let batch_proof = match format {
        ProofFormat::Compressed => BatchProof::ProofBytes(ProofBytes {
            prover_type: ProverType::SP1,
            proof: bincode::serialize(&proof.proof)?,
            public_values: proof.proof.public_values.to_vec(),
        }),
        ProofFormat::Groth16 => BatchProof::ProofCalldata(to_calldata(proof)),
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
