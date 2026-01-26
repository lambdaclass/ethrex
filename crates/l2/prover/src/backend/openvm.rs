use std::time::{Duration, Instant};

use ethrex_guest_program::input::ProgramInput;
use ethrex_l2_common::prover::{BatchProof, ProofFormat};
use openvm_continuations::verifier::internal::types::VmStarkProof;
use openvm_sdk::{Sdk, StdIn, types::EvmProof};
use openvm_stark_sdk::config::baby_bear_poseidon2::BabyBearPoseidon2Config;
use rkyv::rancor::Error;

use crate::backend::{BackendError, ProverBackend};

static PROGRAM_ELF: &[u8] =
    include_bytes!("../../../../guest-program/bin/openvm/out/riscv32im-openvm-elf");

/// OpenVM-specific proof output.
pub enum OpenVmProveOutput {
    Compressed(VmStarkProof<BabyBearPoseidon2Config>),
    Groth16(EvmProof),
}

/// OpenVM prover backend.
#[derive(Default)]
pub struct OpenVmBackend;

impl OpenVmBackend {
    pub fn new() -> Self {
        Self
    }

    /// Execute using already-serialized input.
    fn execute_with_stdin(&self, stdin: StdIn) -> Result<(), BackendError> {
        let sdk = Sdk::standard();
        sdk.execute(PROGRAM_ELF, stdin)
            .map_err(BackendError::execution)?;
        Ok(())
    }

    /// Prove using already-serialized input.
    fn prove_with_stdin(
        &self,
        stdin: StdIn,
        format: ProofFormat,
    ) -> Result<OpenVmProveOutput, BackendError> {
        let sdk = Sdk::standard();
        let proof = match format {
            ProofFormat::Compressed => {
                let (proof, _) = sdk
                    .prove(PROGRAM_ELF, stdin)
                    .map_err(BackendError::proving)?;
                OpenVmProveOutput::Compressed(proof)
            }
            ProofFormat::Groth16 => {
                let proof = sdk
                    .prove_evm(PROGRAM_ELF, stdin)
                    .map_err(BackendError::proving)?;
                OpenVmProveOutput::Groth16(proof)
            }
        };
        Ok(proof)
    }
}

impl ProverBackend for OpenVmBackend {
    type ProofOutput = OpenVmProveOutput;
    type SerializedInput = StdIn;

    fn serialize_input(&self, input: &ProgramInput) -> Result<Self::SerializedInput, BackendError> {
        let mut stdin = StdIn::default();
        let bytes = rkyv::to_bytes::<Error>(input).map_err(BackendError::serialization)?;
        stdin.write_bytes(bytes.as_slice());
        Ok(stdin)
    }

    fn execute(&self, input: ProgramInput) -> Result<(), BackendError> {
        let stdin = self.serialize_input(&input)?;
        self.execute_with_stdin(stdin)
    }

    fn prove(
        &self,
        input: ProgramInput,
        format: ProofFormat,
    ) -> Result<Self::ProofOutput, BackendError> {
        let stdin = self.serialize_input(&input)?;
        self.prove_with_stdin(stdin, format)
    }

    fn verify(&self, _proof: &Self::ProofOutput) -> Result<(), BackendError> {
        Err(BackendError::not_implemented(
            "verify is not implemented for OpenVM backend",
        ))
    }

    fn to_batch_proof(
        &self,
        _proof: Self::ProofOutput,
        _format: ProofFormat,
    ) -> Result<BatchProof, BackendError> {
        Err(BackendError::not_implemented(
            "to_batch_proof is not implemented for OpenVM backend",
        ))
    }

    fn execute_timed(&self, input: ProgramInput) -> Result<Duration, BackendError> {
        let stdin = self.serialize_input(&input)?;
        let start = Instant::now();
        self.execute_with_stdin(stdin)?;
        Ok(start.elapsed())
    }

    fn prove_timed(
        &self,
        input: ProgramInput,
        format: ProofFormat,
    ) -> Result<(Self::ProofOutput, Duration), BackendError> {
        let stdin = self.serialize_input(&input)?;
        let start = Instant::now();
        let proof = self.prove_with_stdin(stdin, format)?;
        Ok((proof, start.elapsed()))
    }
}
