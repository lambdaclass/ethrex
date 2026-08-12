use std::{
    io::ErrorKind,
    process::{Command, Stdio},
    sync::OnceLock,
    time::{Duration, Instant},
};

use ethrex_common::types::prover::{ProofFormat, ProverOutput, ProverType};
use ethrex_guest_program::{ZKVM_LAMBDAVM_PROGRAM_ELF, input::ProgramInput};

use crate::backend::{BackendError, ProverBackend};

const INPUT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/lambdavm_input.bin");
const PROOF_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/lambdavm_proof.bin");
const ELF_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/zkvm-lambdavm-program");

/// Set once the guest ELF has been written to disk; skips the per-call
/// metadata + read + compare dance for the lifetime of the process.
static ELF_WRITTEN: OnceLock<()> = OnceLock::new();

/// Resolves the LambdaVM CLI binary: `LAMBDA_VM_CLI` env var, then
/// `lambda-vm-cli` on `$PATH` (the name `.github/actions/install-lambdavm`
/// installs it under — upstream builds it as the collision-prone `cli`).
fn cli_command() -> Command {
    let bin = std::env::var("LAMBDA_VM_CLI").unwrap_or_else(|_| "lambda-vm-cli".to_string());
    Command::new(bin)
}

/// LambdaVM-specific proof output containing the raw proof bytes emitted by
/// `cli prove`.
pub struct LambdaVmProveOutput(pub Vec<u8>);

/// LambdaVM prover backend.
///
/// LambdaVM has no consumer-ready Rust host SDK today. The published interface
/// is the `cli` binary built from `yetanotherco/lambda_vm`. This backend
/// shells out to that binary for execute / prove / verify and serializes
/// inputs to a length-prefixed file matching LambdaVM's `PRIVATE_INPUT_START`
/// memory layout (4-byte LE length + rkyv-encoded `ProgramInput`).
///
/// The binary must be on `$PATH` as `lambda-vm-cli` (override with the
/// `LAMBDA_VM_CLI` env var). CI installs it via
/// `.github/actions/install-lambdavm`.
#[derive(Default)]
pub struct LambdaVmBackend;

impl LambdaVmBackend {
    pub fn new() -> Self {
        Self
    }

    fn write_elf_file() -> Result<(), BackendError> {
        if ELF_WRITTEN.get().is_some() {
            return Ok(());
        }

        // The plain `lambdavm` feature embeds an empty ELF placeholder; only
        // `lambdavm-build-elf` compiles and embeds the real guest binary.
        if ZKVM_LAMBDAVM_PROGRAM_ELF.is_empty() {
            return Err(BackendError::execution(
                "no LambdaVM guest ELF embedded: rebuild with the `lambdavm-build-elf` feature",
            ));
        }

        let needs_write = match std::fs::metadata(ELF_PATH) {
            Ok(meta) => {
                if meta.len() != u64::try_from(ZKVM_LAMBDAVM_PROGRAM_ELF.len()).unwrap_or(0) {
                    true
                } else {
                    let existing_content =
                        std::fs::read(ELF_PATH).map_err(BackendError::execution)?;
                    existing_content != ZKVM_LAMBDAVM_PROGRAM_ELF
                }
            }
            Err(e) if e.kind() == ErrorKind::NotFound => true,
            Err(e) => return Err(BackendError::execution(e)),
        };

        if needs_write {
            let tmp_path = format!("{ELF_PATH}.{}.tmp", std::process::id());
            std::fs::write(&tmp_path, ZKVM_LAMBDAVM_PROGRAM_ELF).map_err(|e| {
                let _ = std::fs::remove_file(&tmp_path);
                BackendError::execution(e)
            })?;
            std::fs::rename(&tmp_path, ELF_PATH).map_err(|e| {
                let _ = std::fs::remove_file(&tmp_path);
                BackendError::execution(e)
            })?;
        }

        let _ = ELF_WRITTEN.set(());
        Ok(())
    }

    /// Execute assuming input is already serialized to INPUT_PATH.
    fn execute_core(&self) -> Result<(), BackendError> {
        let output = cli_command()
            .args(["execute", ELF_PATH, "--private-input", INPUT_PATH])
            .stdin(Stdio::inherit())
            .stderr(Stdio::piped())
            .output()
            .map_err(BackendError::execution)?;

        if !output.status.success() {
            return Err(BackendError::execution(format!(
                "LambdaVM execution failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        Ok(())
    }

    /// Prove assuming input is already serialized to INPUT_PATH.
    ///
    /// Returns the proof bytes plus the parsed stdout (used by `prove_timed`
    /// to extract `Proving time: <float>s`).
    fn prove_core(
        &self,
        format: ProofFormat,
    ) -> Result<(LambdaVmProveOutput, String), BackendError> {
        // LambdaVM emits its native STARK proof (a fixed-size execution proof,
        // which is what `Compressed` asks for). There is no L1-cheap wrapper
        // (Groth16/Plonk) yet.
        if matches!(format, ProofFormat::Groth16) {
            return Err(BackendError::not_implemented(
                "Groth16-wrapped proofs are not supported by LambdaVM yet; use ProofFormat::Compressed for the native STARK proof",
            ));
        }

        let output = cli_command()
            .args([
                "prove",
                ELF_PATH,
                "-o",
                PROOF_PATH,
                "--private-input",
                INPUT_PATH,
                "--time",
            ])
            .stdin(Stdio::inherit())
            .stderr(Stdio::piped())
            .output()
            .map_err(BackendError::proving)?;

        if !output.status.success() {
            return Err(BackendError::proving(format!(
                "LambdaVM proof generation failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        let proof_bytes = std::fs::read(PROOF_PATH).map_err(BackendError::proving)?;
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();

        Ok((LambdaVmProveOutput(proof_bytes), stdout))
    }

    /// Verify by writing the proof bytes to disk and invoking `cli verify`.
    fn verify_core(&self, proof: &LambdaVmProveOutput) -> Result<(), BackendError> {
        std::fs::write(PROOF_PATH, &proof.0).map_err(BackendError::verification)?;

        let output = cli_command()
            .args(["verify", PROOF_PATH, ELF_PATH])
            .stdin(Stdio::inherit())
            .stderr(Stdio::piped())
            .output()
            .map_err(BackendError::verification)?;

        if output.status.success() {
            Ok(())
        } else {
            Err(BackendError::Verification(format!(
                "LambdaVM proof verification failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }
}

impl ProverBackend for LambdaVmBackend {
    type ProofOutput = LambdaVmProveOutput;
    type SerializedInput = ();

    fn prover_type(&self) -> ProverType {
        unimplemented!("LambdaVM is not yet enabled as a backend for the L2")
    }

    fn serialize_input(&self, input: &ProgramInput) -> Result<Self::SerializedInput, BackendError> {
        let input_bytes =
            rkyv::to_bytes::<rkyv::rancor::Error>(input).map_err(BackendError::serialization)?;

        // LambdaVM expects `PRIVATE_INPUT_START` to point at a 4-byte LE length
        // prefix followed by the data. The guest reads it via
        // `lambda_vm_syscalls::syscalls::get_private_input`.
        let data_len = u32::try_from(input_bytes.len())
            .map_err(|_| BackendError::serialization("ProgramInput exceeds 4GB"))?;

        let mut buf = Vec::with_capacity(4usize.saturating_add(input_bytes.len()));
        buf.extend_from_slice(&data_len.to_le_bytes());
        buf.extend_from_slice(&input_bytes);

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
        let (proof, _stdout) = self.prove_core(format)?;
        Ok(proof)
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

        let wall_start = Instant::now();
        let (proof, stdout) = self.prove_core(format)?;
        let wall_elapsed = wall_start.elapsed();

        // Prefer the CLI's self-reported `Proving time: <float>s` when present —
        // it isolates the proving step from input/ELF I/O overhead. Fall back
        // to wall-clock measurement if the line is missing or unparseable.
        let duration = parse_proving_time(&stdout).unwrap_or(wall_elapsed);
        Ok((proof, duration))
    }

    fn verify(&self, proof: &Self::ProofOutput) -> Result<(), BackendError> {
        Self::write_elf_file()?;
        self.verify_core(proof)
    }

    fn to_proof_bytes(
        &self,
        _proof: Self::ProofOutput,
        _format: ProofFormat,
    ) -> Result<ProverOutput, BackendError> {
        Err(BackendError::not_implemented(
            "to_proof_bytes is not implemented for LambdaVM backend",
        ))
    }
}

/// Extract the `Proving time: <float>s` line emitted by `cli prove --time`.
fn parse_proving_time(stdout: &str) -> Option<Duration> {
    for line in stdout.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Proving time: ") {
            let secs_str = rest.strip_suffix('s').unwrap_or(rest);
            if let Ok(secs) = secs_str.parse::<f64>()
                && secs.is_finite()
                && secs >= 0.0
            {
                return Some(Duration::from_secs_f64(secs));
            }
        }
    }
    None
}
