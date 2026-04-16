use std::time::{Duration, Instant};

use ethrex_common::types::prover::{ProofFormat, ProverOutput, ProverType};
use ethrex_guest_program::input::ProgramInput;
use openvm_sdk::config::AggregationSystemParams;
use openvm_sdk::{Sdk, StdIn};
use openvm_stark_sdk::config::{app_params_with_100_bits_security, MAX_APP_LOG_STACKED_HEIGHT};
use openvm_verify_stark_host::NonRootStarkProof;
use rkyv::rancor::Error;

use crate::backend::{BackendError, ProverBackend};

static PROGRAM_ELF: &[u8] =
    include_bytes!("../../../guest-program/bin/openvm/out/riscv32im-openvm-elf");

/// OpenVM v2 proof output (Compressed STARK proof only).
pub struct OpenVmProveOutput(pub NonRootStarkProof);

/// OpenVM v2 prover backend.
#[derive(Default)]
pub struct OpenVmBackend;

impl OpenVmBackend {
    pub fn new() -> Self {
        Self
    }

    /// Create a new SDK instance with production-recommended parameters.
    ///
    /// The SDK contains `Rc` internally (in the Transpiler) and is therefore
    /// `!Send`. We create it per-call so that `OpenVmBackend` itself remains
    /// `Send` (required by the Actor framework). The SDK lazily caches proving
    /// keys via `OnceLock` fields, but that cache is local to each instance.
    fn sdk() -> Sdk {
        Sdk::standard(
            app_params_with_100_bits_security(MAX_APP_LOG_STACKED_HEIGHT),
            AggregationSystemParams::default(),
        )
    }

    /// Execute using already-serialized input.
    fn execute_with_stdin(&self, stdin: StdIn) -> Result<(), BackendError> {
        Self::sdk()
            .execute(PROGRAM_ELF, stdin)
            .map_err(BackendError::execution)?;
        Ok(())
    }

    /// Prove using already-serialized input.
    fn prove_with_stdin(
        &self,
        stdin: StdIn,
        format: ProofFormat,
    ) -> Result<OpenVmProveOutput, BackendError> {
        match format {
            ProofFormat::Compressed => {
                let (proof, _verification_baseline) = Self::sdk()
                    .prove(PROGRAM_ELF, stdin, &[])
                    .map_err(BackendError::proving)?;
                Ok(OpenVmProveOutput(proof))
            }
            ProofFormat::Groth16 => Err(BackendError::not_implemented(
                "Groth16 is not supported for OpenVM backend",
            )),
        }
    }
}

impl ProverBackend for OpenVmBackend {
    type ProofOutput = OpenVmProveOutput;
    type SerializedInput = StdIn;

    fn prover_type(&self) -> ProverType {
        unimplemented!("OpenVM is not yet enabled as a backend for the L2")
    }

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

    fn to_proof_bytes(
        &self,
        _proof: Self::ProofOutput,
        _format: ProofFormat,
    ) -> Result<ProverOutput, BackendError> {
        Err(BackendError::not_implemented(
            "to_proof_bytes is not implemented for OpenVM backend",
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