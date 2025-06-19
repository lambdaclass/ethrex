use ethrex_l2::utils::prover::proving_systems::{
    BatchProof, ProofBytes, ProofCalldata, ProverType,
};
use ethrex_l2_sdk::calldata::Value;
use sp1_sdk::{
    EnvProver, HashableKey, ProverClient, SP1ProofWithPublicValues, SP1ProvingKey, SP1Stdin,
    SP1VerifyingKey,
};
use std::{fmt::Debug, sync::LazyLock};
use tracing::info;
use zkvm_interface::io::ProgramInput;

static PROGRAM_ELF: &[u8] =
    include_bytes!("../../zkvm/interface/sp1/out/riscv32im-succinct-zkvm-elf");

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
    stdin.write(&input);

    let setup = &*PROVER_SETUP;

    setup.client.execute(PROGRAM_ELF, &stdin).run()?;

    info!("Successfully executed SP1 program.");
    Ok(())
}

pub fn prove(
    input: ProgramInput,
    aligned_mode: bool,
) -> Result<ProveOutput, Box<dyn std::error::Error>> {
    let mut stdin = SP1Stdin::new();
    stdin.write(&input);

    let setup = &*PROVER_SETUP;

    // contains the receipt along with statistics about execution of the guest
    let proof = if aligned_mode {
        setup.client.prove(&setup.pk, &stdin).compressed().run()?
    } else {
        setup.client.prove(&setup.pk, &stdin).groth16().run()?
    };

    info!("Successfully generated SP1Proof.");
    Ok(ProveOutput::new(proof, setup.vk.clone()))
}

pub fn verify(output: &ProveOutput) -> Result<(), Box<dyn std::error::Error>> {
    let setup = &*PROVER_SETUP;
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
