#![allow(clippy::module_inception)]
use tracing::info;

use sp1_sdk::{ProverClient, SP1ProofWithPublicValues, SP1ProvingKey, SP1Stdin, SP1VerifyingKey};

use crate::utils::config::prover_client::ProverClientConfig;

pub struct Prover {
    client: ProverClient,
    pk: SP1ProvingKey,
    vk: SP1VerifyingKey,
}

impl Default for Prover {
    fn default() -> Self {
        let config = ProverClientConfig::from_env().unwrap();
        Self::new_from_config(config)
    }
}

impl Prover {
    pub fn new_from_config(config: ProverClientConfig) -> Self {
        let elf = std::fs::read(config.elf_path).unwrap();

        info!("Setting up prover...");
        let client = ProverClient::new();
        let (pk, vk) = client.setup(elf.as_slice());
        info!("Prover setup complete!");

        Self { client, pk, vk }
    }

    pub fn prove(&self, id: u64) -> Result<SP1ProofWithPublicValues, String> {
        // Setup the inputs.
        let mut stdin = SP1Stdin::new();
        stdin.write(&id);

        info!("Starting Fibonacci proof for n = {id}");

        // Generate the proof
        let proof = self
            .client
            .prove(&self.pk, stdin)
            .groth16()
            .run()
            .map_err(|_| "Failed to generate proof".to_string())?;

        info!("Successfully generated proof!");

        // Verify the proof.
        self.client
            .verify(&proof, &self.vk)
            .map_err(|_| "Failed to verify proof".to_string())?;
        info!("Successfully verified proof!");

        Ok(proof)
    }
}
