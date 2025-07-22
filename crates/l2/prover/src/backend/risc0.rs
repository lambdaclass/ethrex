use ethrex_l2_common::{
    calldata::Value,
    prover::{BatchProof, ProofCalldata, ProverType},
};
use risc0_zkp::verify::VerificationError;
use risc0_zkvm::{
    ExecutorEnv, InnerReceipt, ProverOpts, Receipt, default_executor, default_prover,
};
use tracing::info;

use crate::{
    backend::ProverBackend,
    guest_program::input::{JSONProgramInput, ProgramInput},
};

const ZKVM_RISC0_PROGRAM_ELF: &[u8] = &[0u8; 0]; // Placeholder for the actual ELF file content
const ZKVM_RISC0_PROGRAM_ID: [u32; 8] = [0_u32; 8]; // Placeholder for the actual program ID

#[derive(Debug, thiserror::Error)]
pub enum RISC0BackendError {
    #[error("Failed to build RISC0 executor: {0}")]
    ExecutorBuildError(String),
    #[error("Failed to execute RISC0 program: {0}")]
    ProgramExecutionError(String),
    #[error("Failed to generate RISC0 program output: {0}")]
    ProgramOutputError(String),
    #[error("Failed to verify RISC0 program output: {0}")]
    VerificationError(#[from] VerificationError),
    #[error("Internal error, this is a bug: {0}")]
    InternalError(#[from] InternalError),
}

#[derive(Debug, thiserror::Error)]
pub enum InternalError {
    #[error("Tried to encode a non-Groth16 seal")]
    EncodeNonGroth16Seal,
    #[error("No seal selector in receipt's verifier parameters")]
    NoSealSelector,
}

pub struct RISC0Backend;

impl ProverBackend for RISC0Backend {
    type ProgramOutput = Receipt;

    type Error = RISC0BackendError;

    fn r#type(&self) -> ProverType {
        ProverType::RISC0
    }

    fn execute(&self, input: ProgramInput) -> Result<(), Self::Error> {
        let env = ExecutorEnv::builder()
            .write(&JSONProgramInput(input))
            .map_err(|err| RISC0BackendError::ExecutorBuildError(err.to_string()))?
            .build()
            .map_err(|err| RISC0BackendError::ExecutorBuildError(err.to_string()))?;

        let executor = default_executor();

        let _session_info = executor
            .execute(env, ZKVM_RISC0_PROGRAM_ELF)
            .map_err(|err| RISC0BackendError::ProgramExecutionError(err.to_string()))?;

        info!("Successfully generated session info");

        Ok(())
    }

    fn prove(
        &self,
        input: ProgramInput,
        _aligned_mode: bool,
    ) -> Result<Self::ProgramOutput, Self::Error> {
        let mut stdout = Vec::new();

        let env = ExecutorEnv::builder()
            .stdout(&mut stdout)
            .write(&JSONProgramInput(input))
            .map_err(|err| Self::Error::ExecutorBuildError(err.to_string()))?
            .build()
            .map_err(|err| Self::Error::ExecutorBuildError(err.to_string()))?;

        let prover = default_prover();

        // contains the receipt along with statistics about execution of the guest
        let prove_info = prover
            .prove_with_opts(env, ZKVM_RISC0_PROGRAM_ELF, &ProverOpts::groth16())
            .map_err(|err| Self::Error::ProgramOutputError(err.to_string()))?;

        info!("Successfully generated execution receipt.");

        Ok(prove_info.receipt)
    }

    fn verify(&self, output: Self::ProgramOutput) -> Result<(), Self::Error> {
        output
            .verify(ZKVM_RISC0_PROGRAM_ID)
            .map_err(Self::Error::VerificationError)
    }

    fn to_batch_proof(
        &self,
        output: Self::ProgramOutput,
        _aligned_mode: bool,
    ) -> Result<BatchProof, Self::Error> {
        self.to_calldata(output).map(BatchProof::ProofCalldata)
    }

    fn to_calldata(&self, output: Self::ProgramOutput) -> Result<ProofCalldata, Self::Error> {
        let seal = encode_seal(&output)?;
        let journal = output.journal.bytes;

        // bytes calldata seal,
        // bytes32 imageId,
        // bytes journal
        let calldata = vec![Value::Bytes(seal.into()), Value::Bytes(journal.into())];

        let calldata = ProofCalldata {
            prover_type: self.r#type(),
            calldata,
        };

        Ok(calldata)
    }
}

// ref: https://github.com/risc0/risc0-ethereum/blob/046bb34ea4605f9d8420c7db89baf8e1064fa6f5/contracts/src/lib.rs#L88
// this was reimplemented because risc0-ethereum-contracts brings a different version of c-kzg into the workspace (2.1.0),
// which is incompatible with our current version (1.0.3).
fn encode_seal(receipt: &Receipt) -> Result<Vec<u8>, RISC0BackendError> {
    let InnerReceipt::Groth16(receipt) = receipt.inner.clone() else {
        return Err(InternalError::EncodeNonGroth16Seal)?;
    };

    let selector = &receipt
        .verifier_parameters
        .as_bytes()
        .get(..4)
        .ok_or(InternalError::NoSealSelector)?;

    // Create a new vector with the capacity to hold both selector and seal
    let mut selector_seal = Vec::with_capacity(selector.len() + receipt.seal.len());

    selector_seal.extend_from_slice(selector);

    selector_seal.extend_from_slice(receipt.seal.as_ref());

    Ok(selector_seal)
}
