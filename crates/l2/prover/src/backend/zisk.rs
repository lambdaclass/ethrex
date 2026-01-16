use ethrex_l2_common::calldata::Value;
use ethrex_l2_common::prover::{BatchProof, ProofBytes, ProofCalldata, ProofFormat, ProverType};
use guest_program::{ZKVM_ZISK_PROGRAM_ELF, input::ProgramInput, output::ProgramOutput};
use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const INPUT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/zisk_input.bin");
const OUTPUT_DIR_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/zisk_output");
const ELF_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/zkvm-zisk-program");
const ZISK_VK_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/guest_program/src/zisk/out/riscv64ima-zisk-vk"
);

fn resolve_elf_path() -> PathBuf {
    std::env::var_os("ZISK_ELF_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(ELF_PATH))
}

pub struct ProveOutput {
    pub proof: Vec<u8>,
    pub publics: Vec<u8>,
    pub vk: Vec<u8>,
}

pub fn execute(input: ProgramInput) -> Result<(), Box<dyn std::error::Error>> {
    let elf_path = resolve_elf_path();
    if elf_path == Path::new(ELF_PATH) {
        write_elf_file()?;
    } else if !elf_path.exists() {
        return Err(format!("ELF file not found at {}", elf_path.display()).into());
    }

    let input_bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&input)?;
    std::fs::write(INPUT_PATH, input_bytes.as_slice())?;

    let elf_path_str = elf_path.to_string_lossy();
    let args = vec!["--elf", elf_path_str.as_ref(), "--inputs", INPUT_PATH];
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

pub fn execute_timed(
    input: ProgramInput,
) -> Result<std::time::Duration, Box<dyn std::error::Error>> {
    let elf_path = resolve_elf_path();
    if elf_path == Path::new(ELF_PATH) {
        write_elf_file()?;
    } else if !elf_path.exists() {
        return Err(format!("ELF file not found at {}", elf_path.display()).into());
    }

    let input_bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&input)?;
    std::fs::write(INPUT_PATH, input_bytes.as_slice())?;

    let start = std::time::Instant::now();
    let elf_path_str = elf_path.to_string_lossy();
    let args = vec!["--elf", elf_path_str.as_ref(), "--inputs", INPUT_PATH];
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
    let duration = start.elapsed();

    Ok(duration)
}

pub fn prove(
    input: ProgramInput,
    format: ProofFormat,
) -> Result<ProveOutput, Box<dyn std::error::Error>> {
    let elf_path = resolve_elf_path();
    if elf_path == Path::new(ELF_PATH) {
        write_elf_file()?;
    } else if !elf_path.exists() {
        return Err(format!("ELF file not found at {}", elf_path.display()).into());
    }

    let input_bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&input)?;
    std::fs::write(INPUT_PATH, input_bytes.as_slice())?;

    let elf_path_str = elf_path.to_string_lossy();
    let static_args = vec![
        "prove",
        "--elf",
        elf_path_str.as_ref(),
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
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "ZisK proof generation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    let path_to_proof = format!("{OUTPUT_DIR_PATH}/vadcop_final_proof.bin");
    if let ProofFormat::Groth16 = format {
        // get the final snark wrapping
        let args = vec![
            "prove-snark",
            "-k",
            "PKEY of snark",
            "-p",
            &path_to_proof,
            "-o",
            OUTPUT_DIR_PATH,
        ];
        let snark = Command::new("cargo-zisk")
            .args(args)
            .stdin(Stdio::inherit())
            .stderr(Stdio::inherit())
            .output()?;

        if !snark.status.success() {
            return Err(format!(
                "ZisK snark generation failed: {}",
                String::from_utf8_lossy(&snark.stderr)
            )
            .into());
        }
        let proof_bytes = std::fs::read(format!("{OUTPUT_DIR_PATH}/final_snark_proof.bin"))?;
        let publics_bytes = std::fs::read(format!("{OUTPUT_DIR_PATH}/final_snark_publics.bin"))?;
        let vk = std::fs::read(ZISK_VK_PATH)?;
        Ok(ProveOutput {
            proof: proof_bytes,
            publics: publics_bytes,
            vk,
        })
    } else {
        let proof_bytes = std::fs::read(path_to_proof)?;
        let output = ProveOutput {
            proof: proof_bytes,
            publics: vec![],
            vk: vec![],
        };
        Ok(output)
    }
}

pub fn prove_timed(
    input: ProgramInput,
    format: ProofFormat,
) -> Result<(ProveOutput, std::time::Duration), Box<dyn std::error::Error>> {
    let proof = prove(input, format)?;

    #[derive(serde::Deserialize)]
    struct ZisKResult {
        #[serde(rename = "cycles")]
        _cycles: u64,
        #[serde(rename = "id")]
        _id: String,
        time: f64,
    }

    let zisk_result_bytes = std::fs::read(format!("{OUTPUT_DIR_PATH}/result.json"))?;

    let zisk_result: ZisKResult = serde_json::from_slice(&zisk_result_bytes)?;

    let duration = std::time::Duration::from_secs_f64(zisk_result.time);

    Ok((proof, duration))
}

pub fn verify(_output: &ProgramOutput) -> Result<(), Box<dyn std::error::Error>> {
    unimplemented!("verify is not implemented for ZisK backend")
}

pub fn to_batch_proof(
    proof: ProveOutput,
    format: ProofFormat,
) -> Result<BatchProof, Box<dyn std::error::Error>> {
    let batch_proof = match format {
        ProofFormat::Compressed => BatchProof::ProofBytes(ProofBytes {
            prover_type: ProverType::ZisK,
            proof: bincode::serialize(&proof.proof)?,
            public_values: proof.publics,
        }),
        ProofFormat::Groth16 => BatchProof::ProofCalldata(ProofCalldata {
            prover_type: ProverType::ZisK,
            calldata: vec![Value::Bytes(proof.proof.into())],
        }),
    };
    Ok(batch_proof)
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
    Ok(())
}
