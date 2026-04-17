use std::{
    io::ErrorKind,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use ethrex_common::types::prover::{ProofFormat, ProverOutput, ProverType};
use ethrex_guest_program::{ZKVM_AIRBENDER_PROGRAM_ELF, input::ProgramInput};

use crate::backend::{BackendError, ProverBackend};

const INPUT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/airbender_input.bin");
const ELF_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/zkvm-airbender-program.elf");
const PROOF_OUTPUT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/airbender_proof.bin");

/// Airbender-specific proof output containing the proof bytes.
pub struct AirbenderProveOutput(pub Vec<u8>);

/// Airbender prover backend.
///
/// Uses `cargo-airbender` CLI commands for execution and proving.
/// Future versions will use the `airbender-host` Rust API directly.
#[derive(Default)]
pub struct AirbenderBackend;

impl AirbenderBackend {
    pub fn new() -> Self {
        Self
    }

    fn write_elf_file() -> Result<(), BackendError> {
        let needs_write = match std::fs::metadata(ELF_PATH) {
            Ok(meta) => {
                if meta.len() != u64::try_from(ZKVM_AIRBENDER_PROGRAM_ELF.len()).unwrap_or(0) {
                    true
                } else {
                    let existing = std::fs::read(ELF_PATH).map_err(BackendError::execution)?;
                    existing != ZKVM_AIRBENDER_PROGRAM_ELF
                }
            }
            Err(e) if e.kind() == ErrorKind::NotFound => true,
            Err(e) => return Err(BackendError::execution(e)),
        };

        if needs_write {
            std::fs::write(ELF_PATH, ZKVM_AIRBENDER_PROGRAM_ELF)
                .map_err(BackendError::execution)?;
        }
        Ok(())
    }

    /// Execute assuming input is already serialized to INPUT_PATH.
    fn execute_core(&self) -> Result<(), BackendError> {
        // cargo airbender run <APP_BIN> --input <INPUT>
        let args = vec!["run", ELF_PATH, "--input", INPUT_PATH];
        let output = Command::new("cargo-airbender")
            .args(args)
            .stdin(Stdio::inherit())
            .stderr(Stdio::piped())
            .output()
            .map_err(BackendError::execution)?;

        if !output.status.success() {
            return Err(BackendError::execution(format!(
                "Airbender execution failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        Ok(())
    }

    /// Prove assuming input is already serialized to INPUT_PATH.
    fn prove_core(&self, backend: &str) -> Result<AirbenderProveOutput, BackendError> {
        // cargo airbender prove <APP_BIN> --input <INPUT> --output <OUTPUT> --backend <BACKEND>
        let args = vec![
            "prove",
            ELF_PATH,
            "--input",
            INPUT_PATH,
            "--output",
            PROOF_OUTPUT_PATH,
            "--backend",
            backend,
        ];

        let output = Command::new("cargo-airbender")
            .args(args)
            .stdin(Stdio::inherit())
            .stderr(Stdio::piped())
            .output()
            .map_err(BackendError::proving)?;

        if !output.status.success() {
            return Err(BackendError::proving(format!(
                "Airbender proof generation failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        let proof_bytes = std::fs::read(PROOF_OUTPUT_PATH).map_err(BackendError::proving)?;

        Ok(AirbenderProveOutput(proof_bytes))
    }
}

impl ProverBackend for AirbenderBackend {
    type ProofOutput = AirbenderProveOutput;
    type SerializedInput = ();

    fn prover_type(&self) -> ProverType {
        unimplemented!("Airbender is not yet enabled as a backend for the L2")
    }

    fn serialize_input(&self, input: &ProgramInput) -> Result<Self::SerializedInput, BackendError> {
        let input_bytes =
            rkyv::to_bytes::<rkyv::rancor::Error>(input).map_err(BackendError::serialization)?;
        std::fs::write(INPUT_PATH, input_bytes.as_slice()).map_err(BackendError::serialization)?;
        Ok(())
    }

    fn execute(&self, input: ProgramInput) -> Result<(), BackendError> {
        Self::write_elf_file()?;
        self.serialize_input(&input)?;
        self.execute_core()
    }

    fn prove(
        &self,
        input: ProgramInput,
        _format: ProofFormat,
    ) -> Result<Self::ProofOutput, BackendError> {
        Self::write_elf_file()?;
        self.serialize_input(&input)?;
        self.prove_core("gpu")
    }

    fn execute_timed(&self, input: ProgramInput) -> Result<Duration, BackendError> {
        Self::write_elf_file()?;
        self.serialize_input(&input)?;
        let start = Instant::now();
        self.execute_core()?;
        Ok(start.elapsed())
    }

    fn prove_timed(
        &self,
        input: ProgramInput,
        _format: ProofFormat,
    ) -> Result<(Self::ProofOutput, Duration), BackendError> {
        Self::write_elf_file()?;
        self.serialize_input(&input)?;
        let start = Instant::now();
        let proof = self.prove_core("gpu")?;
        Ok((proof, start.elapsed()))
    }

    fn verify(&self, _proof: &Self::ProofOutput) -> Result<(), BackendError> {
        Err(BackendError::not_implemented(
            "verify is not implemented for Airbender backend",
        ))
    }

    fn to_proof_bytes(
        &self,
        _proof: Self::ProofOutput,
        _format: ProofFormat,
    ) -> Result<ProverOutput, BackendError> {
        Err(BackendError::not_implemented(
            "to_proof_bytes is not implemented for Airbender backend",
        ))
    }
}
