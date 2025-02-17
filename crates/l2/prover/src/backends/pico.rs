#[derive(Serialize, Deserialize, Clone)]
pub struct PicoProof {
    pub constraints: Vec<u8>,
    pub groth16_witness: Vec<u8>,
}

impl Debug for PicoProof {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PicoProof")
            .field("constraints", &self.constraints)
            .field("groth16_witness", &self.groth16_witness)
            .finish()
    }
}

impl PicoProof {
    pub fn new(
        proof: sp1_sdk::SP1ProofWithPublicValues,
        verifying_key: sp1_sdk::SP1VerifyingKey,
    ) -> Self {
        PicoProof {
            constraints: Vec::new(),
            groth16_witness: Vec::new(),
        }
    }
}

fn prove(input: ProgramInput) -> Result<ProvingOutput, Box<dyn std::error::Error>> {
    let client = DefaultProverClient::new(self.elf);

    let stdin_builder = client.get_stdin_builder();
    stdin_builder.borrow_mut().write(&input);

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
    input: ProgramInput,
) -> Result<ExecuteOutput, Box<dyn std::error::Error>> {
    todo!()
}

fn verify(proving_output: &ProvingOutput) -> Result<(), Box<dyn std::error::Error>> {
    todo!()
}

fn get_gas() -> Result<u64, Box<dyn std::error::Error>> {
    todo!()
}
