use ethrex_l2_common::{
    calldata::Value,
    prover::{BatchProof, ProofBytes, ProofCalldata, ProofFormat, ProverType},
};
use guest_program::{
    input::ProgramInput,
    methods::{ZKVM_RISC0_PROGRAM_ELF, ZKVM_RISC0_PROGRAM_ID},
};
use risc0_zkvm::{
    ExecutorEnv, InnerReceipt, ProverOpts, Receipt, default_executor, default_prover,
};
use rkyv::rancor::Error as RkyvError;

use crate::backend::{BackendError, ProverBackend};

/// RISC0 prover backend.
#[derive(Default)]
pub struct Risc0Backend;

impl Risc0Backend {
    pub fn new() -> Self {
        Self
    }

    fn serialize_input(input: &ProgramInput) -> Result<ExecutorEnv<'static>, BackendError> {
        let bytes = rkyv::to_bytes::<RkyvError>(input).map_err(BackendError::serialization)?;
        ExecutorEnv::builder()
            .write_slice(bytes.as_slice())
            .build()
            .map_err(BackendError::execution)
    }

    fn convert_format(format: ProofFormat) -> ProverOpts {
        match format {
            ProofFormat::Compressed => ProverOpts::succinct(),
            ProofFormat::Groth16 => ProverOpts::groth16(),
        }
    }

    fn to_calldata(receipt: &Receipt) -> Result<ProofCalldata, BackendError> {
        let seal = Self::encode_seal(receipt)?;

        let calldata = vec![Value::Bytes(seal.into())];

        Ok(ProofCalldata {
            prover_type: ProverType::RISC0,
            calldata,
        })
    }

    // ref: https://github.com/risc0/risc0-ethereum/blob/046bb34ea4605f9d8420c7db89baf8e1064fa6f5/contracts/src/lib.rs#L88
    // this was reimplemented because risc0-ethereum-contracts brings a different version of c-kzg into the workspace (2.1.0),
    // which is incompatible with our current version (1.0.3).
    fn encode_seal(receipt: &Receipt) -> Result<Vec<u8>, BackendError> {
        let InnerReceipt::Groth16(groth16_receipt) = receipt.inner.clone() else {
            return Err(BackendError::batch_proof("can only encode groth16 seals"));
        };
        let selector = groth16_receipt
            .verifier_parameters
            .as_bytes()
            .get(..4)
            .ok_or_else(|| BackendError::batch_proof("failed to get seal selector"))?;
        // Create a new vector with the capacity to hold both selector and seal
        let mut selector_seal = Vec::with_capacity(selector.len() + groth16_receipt.seal.len());
        selector_seal.extend_from_slice(selector);
        selector_seal.extend_from_slice(groth16_receipt.seal.as_ref());
        Ok(selector_seal)
    }
}

impl ProverBackend for Risc0Backend {
    type ProofOutput = Receipt;

    fn execute(&self, input: ProgramInput) -> Result<(), BackendError> {
        let env = Self::serialize_input(&input)?;
        let executor = default_executor();

        executor
            .execute(env, ZKVM_RISC0_PROGRAM_ELF)
            .map_err(BackendError::execution)?;

        Ok(())
    }

    fn prove(
        &self,
        input: ProgramInput,
        format: ProofFormat,
    ) -> Result<Self::ProofOutput, BackendError> {
        let mut stdout = Vec::new();

        let bytes = rkyv::to_bytes::<RkyvError>(&input).map_err(BackendError::serialization)?;
        let env = ExecutorEnv::builder()
            .stdout(&mut stdout)
            .write_slice(bytes.as_slice())
            .build()
            .map_err(BackendError::execution)?;

        let prover = default_prover();
        let prover_opts = Self::convert_format(format);

        let prove_info = prover
            .prove_with_opts(env, ZKVM_RISC0_PROGRAM_ELF, &prover_opts)
            .map_err(BackendError::proving)?;

        Ok(prove_info.receipt)
    }

    fn verify(&self, proof: &Self::ProofOutput) -> Result<(), BackendError> {
        proof
            .verify(ZKVM_RISC0_PROGRAM_ID)
            .map_err(BackendError::verification)?;

        Ok(())
    }

    fn to_batch_proof(
        &self,
        proof: Self::ProofOutput,
        format: ProofFormat,
    ) -> Result<BatchProof, BackendError> {
        let batch_proof = match format {
            ProofFormat::Compressed => BatchProof::ProofBytes(ProofBytes {
                prover_type: ProverType::RISC0,
                proof: bincode::serialize(&proof.inner).map_err(BackendError::batch_proof)?,
                public_values: proof.journal.bytes,
            }),
            ProofFormat::Groth16 => BatchProof::ProofCalldata(Self::to_calldata(&proof)?),
        };

        Ok(batch_proof)
    }
}
