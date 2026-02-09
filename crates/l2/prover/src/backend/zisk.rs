use std::{
    io::ErrorKind,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use ethrex_guest_program::{ZKVM_ZISK_PROGRAM_ELF, input::ProgramInput};
use ethrex_l2_common::prover::{BatchProof, ProofFormat, ProverType};

use crate::backend::{BackendError, ProverBackend};

const INPUT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/zisk_input.bin");
const OUTPUT_DIR_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/zisk_output");
const ELF_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/zkvm-zisk-program");

/// ZisK-specific proof output containing the proof bytes.
pub struct ZiskProveOutput(pub Vec<u8>);

/// ZisK prover backend.
///
/// This backend uses external commands (`ziskemu` and `cargo-zisk`) to execute
/// and prove programs.
#[derive(Default)]
pub struct ZiskBackend;

impl ZiskBackend {
    pub fn new() -> Self {
        Self
    }

    fn write_elf_file() -> Result<(), BackendError> {
        match std::fs::read(ELF_PATH) {
            Ok(existing_content) => {
                if existing_content != ZKVM_ZISK_PROGRAM_ELF {
                    std::fs::write(ELF_PATH, ZKVM_ZISK_PROGRAM_ELF)
                        .map_err(BackendError::execution)?;
                }
            }
            Err(e) => {
                if e.kind() == ErrorKind::NotFound {
                    std::fs::write(ELF_PATH, ZKVM_ZISK_PROGRAM_ELF)
                        .map_err(BackendError::execution)?;
                } else {
                    return Err(BackendError::execution(e));
                }
            }
        }
        Ok(())
    }

    /// Execute assuming input is already serialized to INPUT_PATH.
    fn execute_core(&self) -> Result<(), BackendError> {
        let args = vec!["--elf", ELF_PATH, "--inputs", INPUT_PATH];
        let output = Command::new("ziskemu")
            .args(args)
            .stdin(Stdio::inherit())
            .stderr(Stdio::inherit())
            .output()
            .map_err(BackendError::execution)?;

        if !output.status.success() {
            return Err(BackendError::execution(format!(
                "ZisK execution failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        Ok(())
    }

    /// Prove assuming input is already serialized to INPUT_PATH.
    fn prove_core(&self, format: ProofFormat) -> Result<ZiskProveOutput, BackendError> {
        let static_args = vec![
            "prove",
            "--elf",
            ELF_PATH,
            "--input",
            INPUT_PATH,
            "--output-dir",
            OUTPUT_DIR_PATH,
            "--aggregation",
            "--unlock-mapped-memory",
        ];
        let conditional_groth16_arg = if let ProofFormat::Groth16 = format {
            vec!["--final-snark"]
        } else {
            vec![]
        };

        let output = Command::new("cargo-zisk")
            .args(static_args)
            .args(conditional_groth16_arg)
            .stdin(Stdio::inherit())
            .stderr(Stdio::inherit())
            .output()
            .map_err(BackendError::proving)?;

        if !output.status.success() {
            return Err(BackendError::proving(format!(
                "ZisK proof generation failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        let proof_bytes = std::fs::read(format!("{OUTPUT_DIR_PATH}/vadcop_final_proof.bin"))
            .map_err(BackendError::proving)?;

        Ok(ZiskProveOutput(proof_bytes))
    }
}

impl ProverBackend for ZiskBackend {
    type ProofOutput = ZiskProveOutput;
    type SerializedInput = ();

    fn prover_type(&self) -> ProverType {
        ProverType::Exec
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
        format: ProofFormat,
    ) -> Result<Self::ProofOutput, BackendError> {
        Self::write_elf_file()?;
        self.serialize_input(&input)?;
        self.prove_core(format)
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
        format: ProofFormat,
    ) -> Result<(Self::ProofOutput, Duration), BackendError> {
        // ZisK reports its own timing in result.json, so we use that instead of measuring
        Self::write_elf_file()?;
        self.serialize_input(&input)?;
        let proof = self.prove_core(format)?;

        #[derive(serde::Deserialize)]
        struct ZisKResult {
            #[serde(rename = "cycles")]
            _cycles: u64,
            #[serde(rename = "id")]
            _id: String,
            time: f64,
        }

        let zisk_result_bytes = std::fs::read(format!("{OUTPUT_DIR_PATH}/result.json"))
            .map_err(BackendError::proving)?;

        let zisk_result: ZisKResult =
            serde_json::from_slice(&zisk_result_bytes).map_err(BackendError::proving)?;

        let duration = Duration::from_secs_f64(zisk_result.time);

        Ok((proof, duration))
    }

    fn verify(&self, _proof: &Self::ProofOutput) -> Result<(), BackendError> {
        Err(BackendError::not_implemented(
            "verify is not implemented for ZisK backend",
        ))
    }

    fn to_batch_proof(
        &self,
        _proof: Self::ProofOutput,
        _format: ProofFormat,
    ) -> Result<BatchProof, BackendError> {
        Err(BackendError::not_implemented(
            "to_batch_proof is not implemented for ZisK backend",
        ))
    }
}
