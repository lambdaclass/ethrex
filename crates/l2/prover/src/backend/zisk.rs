use std::process::{Command, Stdio};

use ethrex_l2_common::prover::{BatchProof, ProofFormat};
use guest_program::{ZKVM_ZISK_PROGRAM_ELF, input::ProgramInput, output::ProgramOutput};

const INPUT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/zisk_input.bin");

const OUTPUT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/zisk_output/");

const ELF_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/zisk_elf");

pub struct ProveOutput(pub Vec<u8>);

pub fn execute(input: ProgramInput) -> Result<(), Box<dyn std::error::Error>> {
    // We write the ELF to a temp file because ziskemu currently only accepts
    // ELF files from disk
    if !std::path::Path::new(ELF_PATH).exists() {
        std::fs::write(ELF_PATH, ZKVM_ZISK_PROGRAM_ELF)?;
    }

    let input_bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&input)?;

    // We write the input to a temp file because ziskemu currently only accepts
    // input files from disk
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
        .stdin(Stdio::inherit())
        .stderr(Stdio::inherit())
        .output()?;
    let duration = start.elapsed();

    std::fs::remove_file(INPUT_PATH)?;

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

    Ok(())
}

pub fn prove(
    input: ProgramInput,
    format: ProofFormat,
) -> Result<ProveOutput, Box<dyn std::error::Error>> {
    // We write the ELF to a temp file because cargo-zisk prove currently only
    // accepts ELF files from disk
    if !std::path::Path::new(ELF_PATH).exists() {
        std::fs::write(ELF_PATH, ZKVM_ZISK_PROGRAM_ELF)?;
    }

    let input_bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&input)?;

    // We write the input to a temp file because cargo-zisk prove currently only
    // accepts input files from disk
    std::fs::write(INPUT_PATH, input_bytes.as_slice())?;

    let mut cmd = Command::new("cargo-zisk");

    let start = std::time::Instant::now();
    let output = cmd
        .arg("prove")
        .arg("--elf")
        .arg(ELF_PATH)
        .arg("--input")
        .arg(INPUT_PATH)
        .arg("--output-dir")
        .arg(OUTPUT_PATH)
        .arg("--unlock-mapped-memory")
        //.arg("--proving-key")
        .arg("--final-snark")
        .stdin(Stdio::inherit())
        .stderr(Stdio::inherit())
        .output()?;
    let duration = start.elapsed();

    if !output.status.success() {
        return Err(format!(
            "ZisK proof generation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    println!(
        "ZisK proof generated in {:.2?} seconds",
        duration.as_secs_f64()
    );

    let proof_bytes = std::fs::read(OUTPUT_PATH)?;
    let output = ProveOutput(proof_bytes);
    std::fs::remove_file(OUTPUT_PATH)?;
    Ok(output)
}

pub fn verify(_output: &ProgramOutput) -> Result<(), Box<dyn std::error::Error>> {
    Err("verify is not implemented for ZisK backend".into())
}

pub fn to_batch_proof(
    proof: ProveOutput,
    format: ProofFormat,
) -> Result<BatchProof, Box<dyn std::error::Error>> {
    Err("to_batch_proof is not implemented for ZisK backend".into())
}
