use std::process::{Command, Stdio};

use ethrex_l2_common::prover::{BatchProof, ProofFormat};
use guest_program::{ZKVM_ZISK_PROGRAM_ELF, input::ProgramInput, output::ProgramOutput};

const INPUT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/zisk_input.bin");

const OUTPUT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/zisk_output.bin");

const ELF_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/zisk_elf");

pub fn execute(input: ProgramInput) -> Result<(), Box<dyn std::error::Error>> {
    // We write the ELF to a temp file because ziskemu currently only accepts ELF files from disk
    std::fs::write(ELF_PATH, ZKVM_ZISK_PROGRAM_ELF)?;

    let input_bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&input)?;

    std::fs::write(INPUT_PATH, input_bytes.as_slice())?;

    let mut cmd = Command::new("ziskemu");

    let start = std::time::Instant::now();
    let output = cmd
        .arg("--elf")
        .arg(ELF_PATH)
        .arg("--inputs")
        .arg(INPUT_PATH)
        .arg("--output")
        .arg(OUTPUT_PATH)
        .arg("--stats")
        .stdin(Stdio::inherit())
        .stderr(Stdio::inherit())
        .output()?;
    let duration = start.elapsed();

    if !output.status.success() {
        return Err(format!(
            "ZisK execution failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    println!(
        "ZisK guest program executed in {:.2?} seconds",
        duration.as_secs_f64()
    );

    std::fs::remove_file(INPUT_PATH)?;

    Ok(())
}

pub fn prove(
    _input: ProgramInput,
    _format: ProofFormat,
) -> Result<ProgramOutput, Box<dyn std::error::Error>> {
    Err("prove is not implemented for ZisK backend".into())
}

pub fn verify(_output: &ProgramOutput) -> Result<(), Box<dyn std::error::Error>> {
    Err("verify is not implemented for ZisK backend".into())
}

pub fn to_batch_proof(
    _proof: ProgramOutput,
    _format: ProofFormat,
) -> Result<BatchProof, Box<dyn std::error::Error>> {
    Err("to_batch_proof is not implemented for ZisK backend".into())
}
