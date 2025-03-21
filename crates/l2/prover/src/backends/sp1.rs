use std::{fmt::Debug, sync::LazyLock};

use ethrex_l2::utils::prover::proving_systems::{ProofCalldata, ProverType};
use ethrex_l2_sdk::calldata::Value;
use sp1_sdk::{
    EnvProver, HashableKey, ProverClient, SP1ProofWithPublicValues, SP1ProvingKey, SP1Stdin,
    SP1VerifyingKey,
};
use tracing::info;
use zkvm_interface::io::ProgramInput;

static PROGRAM_ELF: &[u8] =
    include_bytes!("../../zkvm/interface/sp1/elf/riscv32im-succinct-zkvm-elf");

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

pub fn prove(input: ProgramInput) -> Result<ProveOutput, Box<dyn std::error::Error>> {
    let mut stdin = SP1Stdin::new();
    stdin.write(&input);

    let setup = &*PROVER_SETUP;

    // contains the receipt along with statistics about execution of the guest
    let proof = setup.client.prove(&setup.pk, &stdin).groth16().run()?;
    info!("Successfully generated SP1Proof.");
    Ok(ProveOutput::new(proof, setup.vk.clone()))
}

pub fn verify(output: &ProveOutput) -> Result<bool, Box<dyn std::error::Error>> {
    let setup = &*PROVER_SETUP;
    setup.client.verify(&output.proof, &output.vk)?;

    Ok(true)
}

pub fn to_calldata(proof: ProveOutput) -> Result<ProofCalldata, Box<dyn std::error::Error>> {
    // bytes32 programVKey,
    // bytes calldata publicValues,
    // bytes calldata proofBytes
    let calldata = vec![
        Value::FixedBytes(bytes::Bytes::from_owner(proof.vk.bytes32_raw())),
        Value::Bytes(proof.proof.public_values.to_vec().into()),
        Value::Bytes(proof.proof.bytes().into()),
    ];

    Ok(ProofCalldata {
        prover_type: ProverType::SP1,
        calldata,
    })
}
