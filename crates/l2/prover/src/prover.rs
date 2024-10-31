use serde::Deserialize;
use tracing::info;

// risc0
use zkvm_interface::methods::{ZKVM_PROGRAM_ELF, ZKVM_PROGRAM_ID};

use risc0_zkvm::{default_prover, ExecutorEnv, ExecutorEnvBuilder, ProverOpts};

use ethereum_rust_core::types::Receipt;
use ethereum_rust_l2::{
    proposer::prover_server::ProverInputData, utils::config::prover_client::ProverClientConfig,
};
use ethereum_rust_rlp::encode::RLPEncode;
use ethereum_rust_vm::execution_db::ExecutionDB;

// The order of variables in this structure should match the order in which they were
// committed in the zkVM, with each variable represented by a field.
#[derive(Debug, Deserialize)]
pub struct ProverOutputData {
    /// It is rlp encoded, it has to be decoded.
    /// Block::decode(&prover_output_data.block).unwrap());
    pub _block: Vec<u8>,
    pub _execution_db: ExecutionDB,
    pub _parent_block_header: Vec<u8>,
    pub block_receipts: Vec<Receipt>,
}

pub struct Prover<'a> {
    env_builder: ExecutorEnvBuilder<'a>,
    elf: &'a [u8],
    id: [u32; 8],
}

impl<'a> Default for Prover<'a> {
    fn default() -> Self {
        let _config = ProverClientConfig::from_env().unwrap();
        Self::new()
    }
}

impl<'a> Prover<'a> {
    pub fn new() -> Self {
        Self {
            env_builder: ExecutorEnv::builder(),
            elf: ZKVM_PROGRAM_ELF,
            id: ZKVM_PROGRAM_ID,
        }
    }

    pub fn set_input(&mut self, input: ProverInputData) -> &mut Self {
        let head_block_rlp = input.block.encode_to_vec();
        let parent_header_rlp = input.parent_header.encode_to_vec();

        // We should pass the inputs as a whole struct
        self.env_builder.write(&head_block_rlp).unwrap();
        self.env_builder.write(&input.db).unwrap();
        self.env_builder.write(&parent_header_rlp).unwrap();

        self
    }

    /// Example:
    /// let prover = Prover::new();
    /// let proof = prover.set_input(inputs).prove().unwrap();
    pub fn prove(&mut self) -> Result<risc0_zkvm::Receipt, Box<dyn std::error::Error>> {
        let env = self.env_builder.build()?;

        // Generate the Receipt
        let prover = default_prover();

        // Proof information by proving the specified ELF binary.
        // This struct contains the receipt along with statistics about execution of the guest
        let prove_info = prover.prove_with_opts(env, self.elf, &ProverOpts::groth16())?;

        // extract the receipt.
        let receipt = prove_info.receipt;

        info!("Successfully generated execution receipt.");
        Ok(receipt)
    }

    pub fn verify(&self, receipt: &risc0_zkvm::Receipt) -> Result<(), Box<dyn std::error::Error>> {
        // Verify the proof.
        receipt.verify(self.id)?;
        Ok(())
    }

    pub fn get_commitment(
        receipt: &risc0_zkvm::Receipt,
    ) -> Result<ProverOutputData, Box<dyn std::error::Error>> {
        let commitment: ProverOutputData = receipt.journal.decode()?;
        Ok(commitment)
    }
}
