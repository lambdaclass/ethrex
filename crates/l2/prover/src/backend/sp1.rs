use std::fmt::Debug;

use ethrex_l2_common::{
    calldata::Value,
    prover::{BatchProof, ProofBytes, ProofCalldata, ProofFormat, ProverType},
};
use guest_program::input::ProgramInput;
use rkyv::rancor::Error;
use sp1_sdk::{
    HashableKey, Prover, ProverClient, SP1ProofWithPublicValues, SP1Stdin, SP1VerifyingKey,
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

    let elapsed = if cfg!(feature = "gpu") {
        let client = ProverClient::builder().cuda().build();
        let now = Instant::now();
        client.execute(PROGRAM_ELF, &stdin).run()?;
        now.elapsed()
    } else {
        let client = ProverClient::builder().cpu().build();
        let now = Instant::now();
        client.execute(PROGRAM_ELF, &stdin).run()?;
        now.elapsed()
    };

    info!("Successfully executed SP1 program in {:.2?}", elapsed);
    Ok(())
}

pub fn prove(
    input: ProgramInput,
    format: ProofFormat,
) -> Result<ProveOutput, Box<dyn std::error::Error>> {
    let mut stdin = SP1Stdin::new();
    let bytes = rkyv::to_bytes::<Error>(&input)?;
    stdin.write_slice(bytes.as_slice());

    let (proof, vk) = if cfg!(feature = "gpu") {
        let client = ProverClient::builder().cuda().build();
        let (pk, vk) = client.setup(PROGRAM_ELF);
        let proof = match format {
            ProofFormat::Compressed => client.prove(&pk, &stdin).compressed().run()?,
            ProofFormat::Groth16 => client.prove(&pk, &stdin).groth16().run()?,
        };
        (proof, vk)
    } else {
        let client = ProverClient::builder().cpu().build();
        let (pk, vk) = client.setup(PROGRAM_ELF);
        let proof = match format {
            ProofFormat::Compressed => client.prove(&pk, &stdin).compressed().run()?,
            ProofFormat::Groth16 => client.prove(&pk, &stdin).groth16().run()?,
        };
        (proof, vk)
    };

    info!("Successfully generated SP1Proof.");
    Ok(ProveOutput::new(proof, vk))
}

pub fn verify(output: &ProveOutput) -> Result<(), Box<dyn std::error::Error>> {
    if cfg!(feature = "gpu") {
        let client = ProverClient::builder().cuda().build();
        client.verify(&output.proof, &output.vk)?;
    } else {
        let client = ProverClient::builder().cpu().build();
        client.verify(&output.proof, &output.vk)?;
    };

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
