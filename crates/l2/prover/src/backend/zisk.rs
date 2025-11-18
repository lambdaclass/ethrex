use std::{
    io::ErrorKind,
    process::{Command, Stdio},
};

use ethrex_l2_common::prover::{BatchProof, ProofFormat};
use guest_program::{ZKVM_ZISK_PROGRAM_ELF, input::ProgramInput, output::ProgramOutput};

const INPUT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/zisk_input.bin");

const OUTPUT_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/zisk_output/vadcop_final_proof.compressed.bin"
);

const ELF_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/zkvm-zisk-program");

pub struct ProveOutput(pub Vec<u8>);

pub fn execute(input: ProgramInput) -> Result<(), Box<dyn std::error::Error>> {
    write_elf_file()?;

    let input_bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&input)?;
    std::fs::write(INPUT_PATH, input_bytes.as_slice())?;

    let args = vec![
        "prove",
        "--elf",
        ELF_PATH,
        "--input",
        INPUT_PATH,
        "--output-dir",
        OUTPUT_PATH,
        "--aggregation",
    ];
    let output = Command::new("ziskemu")
        .args(args)
        .stdin(Stdio::inherit())
        .stderr(Stdio::inherit())
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "ZisK execution failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    Ok(())
}

pub fn prove(
    input: ProgramInput,
    format: ProofFormat,
) -> Result<ProveOutput, Box<dyn std::error::Error>> {
    write_elf_file()?;

    let input_bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&input)?;
    std::fs::write(INPUT_PATH, input_bytes.as_slice())?;

    let static_args = vec![
        "prove",
        "--elf",
        ELF_PATH,
        "--input",
        INPUT_PATH,
        "--output-dir",
        OUTPUT_PATH,
        "--aggregation",
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
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "ZisK proof generation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    let proof_bytes = std::fs::read(OUTPUT_PATH)?;
    let output = ProveOutput(proof_bytes);
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

fn write_elf_file() -> Result<(), Box<dyn std::error::Error>> {
    match std::fs::read(ELF_PATH) {
        Ok(existing_content) => {
            if existing_content != ZKVM_ZISK_PROGRAM_ELF {
                std::fs::write(ELF_PATH, ZKVM_ZISK_PROGRAM_ELF)?;
            }
        }
        Err(e) => {
            if e.kind() == ErrorKind::NotFound {
                std::fs::write(ELF_PATH, ZKVM_ZISK_PROGRAM_ELF)?;
            } else {
                return Err(Box::new(e));
            }
        }
    }
    return Ok(());
}
