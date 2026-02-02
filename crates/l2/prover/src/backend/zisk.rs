use ethrex_guest_program::{ZKVM_ZISK_PROGRAM_ELF, input::ProgramInput};
use ethrex_l2_common::{
    calldata::Value,
    prover::{BatchProof, ProofBytes, ProofCalldata, ProofFormat, ProverType},
};
use sha2::{Digest, Sha256};
use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use tracing::{debug, info};

use crate::backend::{BackendError, ProverBackend};

const INPUT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/zisk_input.bin");
const OUTPUT_DIR_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/zisk_output");
const ELF_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/ethrex-guest-zisk");

const ZISK_ELF_ENV: &str = "ZISK_ELF_PATH";
const ZISK_HOME_ENV: &str = "ZISK_HOME";
const ZISK_PROVING_KEY_ENV: &str = "ZISK_PROVING_KEY_PATH";
const ZISK_PROVING_KEY_SNARK_ENV: &str = "ZISK_PROVING_KEY_SNARK_PATH";
const ZISK_STARK_BINARY_ENV: &str = "ZISK_STARK_BINARY";
const ZISK_SNARK_BINARY_ENV: &str = "ZISK_SNARK_BINARY";

fn resolve_elf_path() -> PathBuf {
    std::env::var_os(ZISK_ELF_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(ELF_PATH))
}

fn resolve_zisk_home() -> PathBuf {
    std::env::var_os(ZISK_HOME_ENV)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".zisk")))
        .unwrap_or_else(|| PathBuf::from(".zisk"))
}

fn resolve_proving_key_path() -> PathBuf {
    std::env::var_os(ZISK_PROVING_KEY_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| resolve_zisk_home().join("provingKey"))
}

fn resolve_proving_key_snark_path() -> PathBuf {
    std::env::var_os(ZISK_PROVING_KEY_SNARK_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| resolve_zisk_home().join("provingKeySnark"))
}

/// Returns the binary to use for STARK proof generation.
/// Defaults to "cargo-zisk" if ZISK_STARK_BINARY is not set.
fn resolve_stark_binary() -> String {
    std::env::var(ZISK_STARK_BINARY_ENV).unwrap_or_else(|_| "cargo-zisk".to_string())
}

/// Returns the binary to use for SNARK proof generation.
/// Defaults to "cargo-zisk" if ZISK_SNARK_BINARY is not set.
fn resolve_snark_binary() -> String {
    std::env::var(ZISK_SNARK_BINARY_ENV).unwrap_or_else(|_| "cargo-zisk".to_string())
}

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

    fn prepare_elf_path(&self) -> Result<PathBuf, BackendError> {
        let elf_path = resolve_elf_path();
        if elf_path.as_path() == Path::new(ELF_PATH) {
            Self::write_elf_file()?;
        } else if !elf_path.exists() {
            return Err(BackendError::execution(format!(
                "ELF file not found at {}",
                elf_path.display()
            )));
        }
        Ok(elf_path)
    }

    /// Execute assuming input is already serialized to INPUT_PATH.
    fn execute_core(&self, elf_path: &Path) -> Result<(), BackendError> {
        let elf_path_str = elf_path.to_string_lossy();
        let args = vec!["--elf", elf_path_str.as_ref(), "--inputs", INPUT_PATH];
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
    fn prove_core(
        &self,
        elf_path: &Path,
        format: ProofFormat,
    ) -> Result<ZiskProveOutput, BackendError> {
        let output_dir = Path::new(OUTPUT_DIR_PATH);
        std::fs::create_dir_all(output_dir).map_err(BackendError::proving)?;
        std::fs::create_dir_all(output_dir.join("proofs")).map_err(BackendError::proving)?;

        let cwd = std::env::current_dir().map_err(BackendError::proving)?;
        std::fs::create_dir_all(cwd.join("tmp")).map_err(BackendError::proving)?;

        let proving_key_path = resolve_proving_key_path();

        let elf_path_str = elf_path.to_string_lossy();
        let proving_key_path_str = proving_key_path.to_string_lossy();
        let args = vec![
            "prove",
            "-e",
            elf_path_str.as_ref(),
            "-i",
            INPUT_PATH,
            "-a",
            "-u",
            "-k",
            proving_key_path_str.as_ref(),
            "-o",
            OUTPUT_DIR_PATH,
        ];

        let stark_binary = resolve_stark_binary();
        debug!(binary = %stark_binary, ?args, "Running STARK proof generation");

        let output = Command::new(&stark_binary)
            .args(args)
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

        let path_to_proof = output_dir.join("vadcop_final_proof.bin");
        if let ProofFormat::Groth16 = format {
            info!(
                output_dir = %output_dir.display(),
                "ZisK prove complete; starting snark wrapping"
            );
            let snark_key_path = resolve_proving_key_snark_path();
            let snark_key_path_str = snark_key_path.to_string_lossy();
            let path_to_proof_str = path_to_proof.to_string_lossy();
            let args = vec![
                "prove-snark",
                "-k",
                snark_key_path_str.as_ref(),
                "-p",
                path_to_proof_str.as_ref(),
                "-o",
                OUTPUT_DIR_PATH,
            ];
            let snark_binary = resolve_snark_binary();
            debug!(binary = %snark_binary, ?args, "Running SNARK proof generation");

            let snark = Command::new(&snark_binary)
                .args(args.as_slice())
                .stdin(Stdio::inherit())
                .output()
                .map_err(BackendError::proving)?;

            debug!(
                stdout = %String::from_utf8_lossy(&snark.stdout),
                stderr = %String::from_utf8_lossy(&snark.stderr),
                "cargo-zisk prove-snark output"
            );

            if !snark.status.success() {
                return Err(BackendError::proving(format!(
                    "ZisK snark generation failed: {}",
                    String::from_utf8_lossy(&snark.stderr)
                )));
            }
            let proof_path = output_dir.join("snark_proof").join("final_snark_proof.bin");
            let proof_bytes = std::fs::read(&proof_path).map_err(BackendError::proving)?;

            // Log the SNARK public values (what the guest output)
            let publics_path = output_dir
                .join("snark_proof")
                .join("final_snark_publics.bin");
            if let Ok(publics_bytes) = std::fs::read(&publics_path) {
                debug!(
                    publics_len = publics_bytes.len(),
                    publics_hex = %hex::encode(&publics_bytes),
                    "SNARK public values (guest output)"
                );
                // Extract the SHA256 hash (bytes 4-36, after the count header)
                if let Some(guest_output_hash) = publics_bytes.get(4..36) {
                    debug!(
                        guest_output_sha256 = %hex::encode(guest_output_hash),
                        "Guest program output (sha256 of ProgramOutput.encode())"
                    );
                }
            }

            Ok(ZiskProveOutput(proof_bytes))
        } else {
            let proof_bytes = std::fs::read(path_to_proof).map_err(BackendError::proving)?;
            Ok(ZiskProveOutput(proof_bytes))
        }
    }
}

impl ProverBackend for ZiskBackend {
    type ProofOutput = ZiskProveOutput;
    type SerializedInput = ();

    fn serialize_input(&self, input: &ProgramInput) -> Result<Self::SerializedInput, BackendError> {
        let input_bytes =
            rkyv::to_bytes::<rkyv::rancor::Error>(input).map_err(BackendError::serialization)?;

        // Log the hash of the serialized input
        let input_hash = Sha256::digest(input_bytes.as_slice());
        debug!(
            input_len = input_bytes.len(),
            input_sha256 = %hex::encode(input_hash),
            "Serialized ProgramInput"
        );

        std::fs::write(INPUT_PATH, input_bytes.as_slice()).map_err(BackendError::serialization)?;
        Ok(())
    }

    fn execute(&self, input: ProgramInput) -> Result<(), BackendError> {
        let elf_path = self.prepare_elf_path()?;
        self.serialize_input(&input)?;
        self.execute_core(&elf_path)
    }

    fn prove(
        &self,
        input: ProgramInput,
        format: ProofFormat,
    ) -> Result<Self::ProofOutput, BackendError> {
        let elf_path = self.prepare_elf_path()?;
        self.serialize_input(&input)?;
        self.prove_core(&elf_path, format)
    }

    fn verify(&self, _proof: &Self::ProofOutput) -> Result<(), BackendError> {
        Err(BackendError::not_implemented(
            "verify is not implemented for ZisK backend",
        ))
    }

    fn to_batch_proof(
        &self,
        proof: Self::ProofOutput,
        format: ProofFormat,
    ) -> Result<BatchProof, BackendError> {
        let batch_proof = match format {
            ProofFormat::Compressed => BatchProof::ProofBytes(ProofBytes {
                prover_type: ProverType::ZisK,
                proof: proof.0,
                public_values: vec![],
            }),
            ProofFormat::Groth16 => BatchProof::ProofCalldata(ProofCalldata {
                prover_type: ProverType::ZisK,
                calldata: vec![Value::Bytes(proof.0.into())],
            }),
        };
        Ok(batch_proof)
    }

    fn execute_timed(&self, input: ProgramInput) -> Result<Duration, BackendError> {
        let elf_path = self.prepare_elf_path()?;
        self.serialize_input(&input)?;
        let start = Instant::now();
        self.execute_core(&elf_path)?;
        Ok(start.elapsed())
    }

    fn prove_timed(
        &self,
        input: ProgramInput,
        format: ProofFormat,
    ) -> Result<(Self::ProofOutput, Duration), BackendError> {
        // ZisK reports its own timing in result.json, so we use that instead of measuring.
        let proof = self.prove(input, format)?;

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
}
