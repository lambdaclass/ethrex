use std::{
    io::ErrorKind,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use ethrex_common::types::prover::{ProofFormat, ProverOutput, ProverType};
use ethrex_guest_program::{ZKVM_ZISK_PROGRAM_ELF, input::ProgramInput};

use crate::backend::{BackendError, ProverBackend};

const INPUT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/zisk_input.bin");
const OUTPUT_DIR_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/zisk_output");
const PROOF_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/zisk_output/proof.bin");
const ELF_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/zkvm-zisk-program");

/// ZisK-specific proof output containing the proof bytes.
pub struct ZiskProveOutput(pub Vec<u8>);

/// ZisK prover backend.
///
/// This backend drives the `cargo-zisk` CLI (ZisK v1.0.0-alpha and later) to
/// execute and prove programs: `cargo-zisk execute` for a dry run and
/// `cargo-zisk prove` / `cargo-zisk verify` for proving and verification.
#[derive(Default)]
pub struct ZiskBackend;

impl ZiskBackend {
    pub fn new() -> Self {
        Self
    }

    fn write_elf_file() -> Result<(), BackendError> {
        let needs_write = match std::fs::metadata(ELF_PATH) {
            Ok(meta) => {
                // If the file size doesn't match, we know we need to rewrite without
                // reading potentially large/corrupted file contents.
                if meta.len() != u64::try_from(ZKVM_ZISK_PROGRAM_ELF.len()).unwrap_or(0) {
                    true
                } else {
                    // Size matches — read and compare contents.
                    let existing_content =
                        std::fs::read(ELF_PATH).map_err(BackendError::execution)?;
                    existing_content != ZKVM_ZISK_PROGRAM_ELF
                }
            }
            Err(e) if e.kind() == ErrorKind::NotFound => true,
            Err(e) => return Err(BackendError::execution(e)),
        };

        if needs_write {
            // Atomic write: write to a temporary file in the same directory, then
            // rename into place. rename() is atomic on POSIX filesystems, so we
            // never leave a half-written ELF file behind if the process crashes.
            let tmp_path = format!("{ELF_PATH}.{}.tmp", std::process::id());
            std::fs::write(&tmp_path, ZKVM_ZISK_PROGRAM_ELF).map_err(|e| {
                let _ = std::fs::remove_file(&tmp_path);
                BackendError::execution(e)
            })?;
            std::fs::rename(&tmp_path, ELF_PATH).map_err(|e| {
                let _ = std::fs::remove_file(&tmp_path);
                BackendError::execution(e)
            })?;
        }

        Ok(())
    }

    /// Execute assuming input is already serialized to INPUT_PATH.
    fn execute_core(&self) -> Result<(), BackendError> {
        let output = Command::new("cargo-zisk")
            .args(["execute", "-e", ELF_PATH, "-i", INPUT_PATH])
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
        std::fs::create_dir_all(OUTPUT_DIR_PATH).map_err(BackendError::proving)?;

        // Proof shape:
        // - Groth16 -> `--plonk`: PLONK-wrapped SNARK for on-chain EVM verification.
        // - Compressed -> `--minimal`: smaller fixed-size STARK proof.
        let format_arg: &[&str] = match format {
            ProofFormat::Groth16 => &["--plonk"],
            ProofFormat::Compressed => &["--minimal"],
        };

        // Use GPU acceleration when built with the `gpu` feature. This requires the
        // GPU build of the ZisK toolchain (installed via `ziskup --gpu`).
        #[cfg(feature = "gpu")]
        let gpu_arg: &[&str] = &["--gpu"];
        #[cfg(not(feature = "gpu"))]
        let gpu_arg: &[&str] = &[];

        let output = Command::new("cargo-zisk")
            .args(["prove", "-e", ELF_PATH, "-i", INPUT_PATH, "-o", PROOF_PATH])
            .args(format_arg)
            .args(gpu_arg)
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

        let proof_bytes = std::fs::read(PROOF_PATH).map_err(BackendError::proving)?;

        Ok(ZiskProveOutput(proof_bytes))
    }
}

impl ProverBackend for ZiskBackend {
    type ProofOutput = ZiskProveOutput;
    type SerializedInput = ();

    fn prover_type(&self) -> ProverType {
        unimplemented!("ZisK is not yet enabled as a backend for the L2")
    }

    fn serialize_input(&self, input: &ProgramInput) -> Result<Self::SerializedInput, BackendError> {
        let input_bytes =
            rkyv::to_bytes::<rkyv::rancor::Error>(input).map_err(BackendError::serialization)?;

        // ZisK expects input in ZiskStdin format: an 8-byte little-endian length
        // prefix, the data, then zero-padding to 8-byte alignment. The guest reads
        // it back via `ziskos::io::read_slice()` (the standard `read_input` C ABI).
        let data_len = input_bytes.len();
        let total_len = 8 + data_len;
        let padding = (8 - (total_len % 8)) % 8;

        let mut buf = Vec::with_capacity(total_len + padding);
        buf.extend_from_slice(&data_len.to_le_bytes());
        buf.extend_from_slice(&input_bytes);
        buf.extend(std::iter::repeat_n(0u8, padding));

        std::fs::write(INPUT_PATH, &buf).map_err(BackendError::serialization)?;
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
        Self::write_elf_file()?;
        self.serialize_input(&input)?;
        let start = Instant::now();
        let proof = self.prove_core(format)?;
        Ok((proof, start.elapsed()))
    }

    fn verify(&self, proof: &Self::ProofOutput) -> Result<(), BackendError> {
        // `cargo-zisk verify` reads the proof from a file and auto-detects its kind.
        std::fs::create_dir_all(OUTPUT_DIR_PATH).map_err(BackendError::verification)?;
        std::fs::write(PROOF_PATH, &proof.0).map_err(BackendError::verification)?;

        let output = Command::new("cargo-zisk")
            .args(["verify", "-p", PROOF_PATH])
            .stdin(Stdio::inherit())
            .stderr(Stdio::inherit())
            .output()
            .map_err(BackendError::verification)?;

        if !output.status.success() {
            return Err(BackendError::verification(format!(
                "ZisK proof verification failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        Ok(())
    }

    fn to_proof_bytes(
        &self,
        _proof: Self::ProofOutput,
        _format: ProofFormat,
    ) -> Result<ProverOutput, BackendError> {
        Err(BackendError::not_implemented(
            "to_proof_bytes is not implemented for ZisK backend",
        ))
    }
}
