use std::time::{Duration, Instant};

use ethrex_guest_program::{
    input::ProgramInput,
    methods::{ETHREX_GUEST_RISC0_ELF, ETHREX_GUEST_RISC0_ID},
};
use ethrex_l2_common::{
    calldata::Value,
    prover::{BatchProof, ProofBytes, ProofCalldata, ProofFormat, ProverType},
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

    /// Execute using already-serialized input.
    fn execute_with_env(&self, env: ExecutorEnv<'_>) -> Result<(), BackendError> {
        let executor = default_executor();
        executor
            .execute(env, ETHREX_GUEST_RISC0_ELF)
            .map_err(BackendError::execution)?;
        Ok(())
    }

    /// Prove using already-serialized input.
    fn prove_with_env(
        &self,
        env: ExecutorEnv<'_>,
        format: ProofFormat,
    ) -> Result<Receipt, BackendError> {
        let prover = default_prover();
        let prover_opts = Self::convert_format(format);
        let prove_info = prover
            .prove_with_opts(env, ETHREX_GUEST_RISC0_ELF, &prover_opts)
            .map_err(BackendError::proving)?;
        Ok(prove_info.receipt)
    }
}

impl ProverBackend for Risc0Backend {
    type ProofOutput = Receipt;
    type SerializedInput = ExecutorEnv<'static>;

    fn serialize_input(&self, input: &ProgramInput) -> Result<Self::SerializedInput, BackendError> {
        let bytes = rkyv::to_bytes::<RkyvError>(input).map_err(BackendError::serialization)?;
        ExecutorEnv::builder()
            .write_slice(bytes.as_slice())
            .build()
            .map_err(BackendError::execution)
    }

    fn execute(&self, input: ProgramInput) -> Result<(), BackendError> {
        let env = self.serialize_input(&input)?;
        self.execute_with_env(env)
    }

    fn prove(
        &self,
        input: ProgramInput,
        format: ProofFormat,
    ) -> Result<Self::ProofOutput, BackendError> {
        let env = self.serialize_input(&input)?;
        self.prove_with_env(env, format)
    }

    fn verify(&self, proof: &Self::ProofOutput) -> Result<(), BackendError> {
        proof
            .verify(ETHREX_GUEST_RISC0_ID)
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

    fn execute_timed(&self, input: ProgramInput) -> Result<Duration, BackendError> {
        let env = self.serialize_input(&input)?;
        let start = Instant::now();
        self.execute_with_env(env)?;
        Ok(start.elapsed())
    }

    fn prove_timed(
        &self,
        input: ProgramInput,
        format: ProofFormat,
    ) -> Result<(Self::ProofOutput, Duration), BackendError> {
        let env = self.serialize_input(&input)?;
        let start = Instant::now();
        let proof = self.prove_with_env(env, format)?;
        Ok((proof, start.elapsed()))
    }
}
