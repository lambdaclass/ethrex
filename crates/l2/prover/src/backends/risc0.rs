

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Proof {
    pub receipt: Box<risc0_zkvm::Receipt>,
    pub prover_id: Vec<u32>,
}

pub struct ContractData {
    pub block_proof: Vec<u8>,
    pub image_id: Vec<u8>,
    pub journal_digest: Vec<u8>,
}

impl Proof {
    // 8 times u32
    const IMAGE_ID_SIZE: usize = 8;
    // 4 times u8
    const SELECTOR_SIZE: usize = 4;
    pub fn new(receipt: risc0_zkvm::Receipt, prover_id: Vec<u32>) -> Self {
        Risc0Proof {
            receipt: Box::new(receipt),
            prover_id,
        }
    }

    pub fn contract_data(&self) -> Result<ContractData, ProverServerError> {
        // If we run the prover_client with RISC0_DEV_MODE=0 we will have a groth16 proof
        // Else, we will have a fake proof.
        //
        // The RISC0_DEV_MODE=1 should only be used with DEPLOYER_CONTRACT_VERIFIER=0xAA
        let block_proof = match self.receipt.inner.groth16() {
            Ok(inner) => {
                // The SELECTOR is used to perform an extra check inside the groth16 verifier contract.
                let mut selector = hex::encode(
                    inner
                        .verifier_parameters
                        .as_bytes()
                        .get(..Self::SELECTOR_SIZE)
                        .ok_or(ProverServerError::Custom(
                            "Failed to get verify_proof_selector in send_proof()".to_owned(),
                        ))?,
                );
                let seal = hex::encode(inner.clone().seal);
                selector.push_str(&seal);
                hex::decode(selector).map_err(|e| {
                    ProverServerError::Custom(format!("Failed to hex::decode(selector): {e}"))
                })?
            }
            Err(_) => vec![0u8; 4],
        };

        let mut image_id = [0_u32; Self::IMAGE_ID_SIZE];
        for (i, b) in image_id.iter_mut().enumerate() {
            *b = *self.prover_id.get(i).ok_or(ProverServerError::Custom(
                "Failed to get image_id in handle_proof_submission()".to_owned(),
            ))?;
        }

        let image_id: risc0_zkvm::sha::Digest = image_id.into();
        let image_id = image_id.as_bytes().to_vec();

        let journal_digest = Digestible::digest(&self.receipt.journal)
            .as_bytes()
            .to_vec();

        Ok(ContractData {
            block_proof,
            image_id,
            journal_digest,
        })
    }
}

fn execute(
    &mut self,
    input: ProgramInput,
) -> Result<ExecuteOutput, Box<dyn std::error::Error>> {
    todo!()
}

fn prove(input: ProgramInput) -> Result<ProvingOutput, Box<dyn std::error::Error>> {
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

fn verify(proving_output: &ProvingOutput) -> Result<(), Box<dyn std::error::Error>> {
    // Verify the proof.
    let ProvingOutput::RISC0(proof) = proving_output else {
        return Err(Box::new(ProverError::IncorrectProverType));
    };
    proof.receipt.verify(self.id)?;
    Ok(())
}

fn get_gas(stdout: &[u8]) -> Result<u64, Box<dyn std::error::Error>> {
    Ok(risc0_zkvm::serde::from_slice(
        stdout.get(..8).unwrap_or_default(), // first 8 bytes
    )?)
}

pub fn get_commitment(
    &self,
    proving_output: &ProvingOutput,
) -> Result<ProgramOutput, Box<dyn std::error::Error>> {
    let ProvingOutput::RISC0(proof) = proving_output else {
        return Err(Box::new(ProverError::IncorrectProverType));
    };
    let commitment = proof.receipt.journal.decode()?;
    Ok(commitment)
}
