use tracing::info;
use zkvm_interface::io::ProgramInput;
use sp1_sdk::{SP1Stdin, ProverClient};

static PROGRAM_ELF: &'static [u8] = include_bytes!("../../zkvm/interface/sp1/elf/riscv32im-succinct-zkvm-elf");

pub struct Proof {
    pub proof: Box<sp1_sdk::SP1ProofWithPublicValues>,
    pub vk: sp1_sdk::SP1VerifyingKey,
}

pub struct ContractData {
    pub public_values: Vec<u8>,
    pub vk: Vec<u8>,
    pub proof_bytes: Vec<u8>,
}

impl Debug for Sp1Proof {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sp1Proof")
            .field("proof", &self.proof)
            .field("vk", &self.vk.bytes32())
            .finish()
    }
}

impl Proof {
    pub fn new(
        proof: sp1_sdk::SP1ProofWithPublicValues,
        verifying_key: sp1_sdk::SP1VerifyingKey,
    ) -> Self {
        Sp1Proof {
            proof: Box::new(proof),
            vk: verifying_key,
        }
    }

    pub fn contract_data(&self) -> Result<ContractData, ProverServerError> {
        let vk = self
            .vk
            .bytes32()
            .strip_prefix("0x")
            .ok_or(ProverServerError::Custom(
                "Failed to strip_prefix of sp1 vk".to_owned(),
            ))?
            .to_string();
        let vk_bytes = hex::decode(&vk)
            .map_err(|_| ProverServerError::Custom("Failed hex::decode(&vk)".to_owned()))?;

        Ok(ContractData {
            public_values: self.proof.public_values.to_vec(),
            vk: vk_bytes,
            proof_bytes: self.proof.bytes(),
        })
    }
}

pub fn execute(input: ProgramInput) -> Result<ExecutionOutput, Box<dyn std::error::Error>> {
    let mut stdin = SP1Stdin::new();
    stdin.write(&input);

    // Generate the ProverClient
    let client = ProverClient::new();

    let output = client.execute(*PROGRAM_ELF, &stdin).run()?;

    info!("Successfully executed SP1 program.");
    Ok(ExecuteOutput::SP1(output))
}

pub fn prove(input: ProgramInput) -> Result<ProvingOutput, Box<dyn std::error::Error>> {
    let mut stdin = SP1Stdin::new();
    stdin.write(&input);

    // Generate the ProverClient
    let client = ProverClient::from_env();
    let (pk, vk) = client.setup(*PROGRAM_ELF);

    // Proof information by proving the specified ELF binary.
    // This struct contains the receipt along with statistics about execution of the guest
    let proof = client.prove(&pk, &stdin).groth16().run()?;
    // Wrap Proof and vk
    let sp1_proof = Sp1Proof::new(proof, vk);
    info!("Successfully generated SP1Proof.");
    Ok(ProvingOutput::SP1(sp1_proof))
}

pub fn verify(input: ProgramInput) -> Result<bool, Box<dyn std::error::Error>> {
    // Verify the proof.
    let ProvingOutput::SP1(complete_proof) = proving_output else {
        return Err(Box::new(ProverError::IncorrectProverType));
    };
    let client = ProverClient::from_env();
    client.verify(&complete_proof.proof, &complete_proof.vk)?;

    Ok(())
}

fn get_gas() -> Result<u64, Box<dyn std::error::Error>> {
    todo!()
}
