use ethrex_l2_common::calldata::Value;
use ethrex_l2_common::prover::{BatchProof, ProofBytes, ProofCalldata, ProofFormat, ProverType};
use guest_program::{ZKVM_ZISK_PROGRAM_ELF, input::ProgramInput, output::ProgramOutput};
use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use tracing::info;

const INPUT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/zisk_input.bin");
const OUTPUT_DIR_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/zisk_output");
const ELF_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/zkvm-zisk-program");
const ZISK_VK_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/guest_program/src/zisk/out/riscv64ima-zisk-vk"
);

const ZISK_HOME_ENV: &str = "ZISK_HOME";
const ZISK_PROVING_KEY_ENV: &str = "ZISK_PROVING_KEY_PATH";
const ZISK_PROVING_KEY_SNARK_ENV: &str = "ZISK_PROVING_KEY_SNARK_PATH";
const ZISK_WITNESS_LIB_ENV: &str = "ZISK_WITNESS_LIB_PATH";
const ZISK_REPO_ENV: &str = "ZISK_REPO_PATH";

fn resolve_elf_path() -> PathBuf {
    std::env::var_os("ZISK_ELF_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(ELF_PATH))
}

fn resolve_zisk_home() -> PathBuf {
    std::env::var_os(ZISK_HOME_ENV)
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".zisk"))
        })
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

fn resolve_witness_lib_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Some(path) = std::env::var_os(ZISK_WITNESS_LIB_ENV) {
        return Ok(PathBuf::from(path));
    }
    if let Some(repo) = std::env::var_os(ZISK_REPO_ENV) {
        return Ok(PathBuf::from(repo).join("target/release/libzisk_witness.so"));
    }
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")));
    let fallback = repo_root.join("zisk/target/release/libzisk_witness.so");
    if fallback.exists() {
        return Ok(fallback);
    }
    Err("Missing ZisK witness library path. Set ZISK_WITNESS_LIB_PATH or ZISK_REPO_PATH."
        .into())
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

    let output_dir = Path::new(OUTPUT_DIR_PATH);
    std::fs::create_dir_all(output_dir)?;

    let proving_key_path = resolve_proving_key_path();
    let witness_lib_path = resolve_witness_lib_path()?;

    let elf_path_str = elf_path.to_string_lossy();
    let proving_key_path_str = proving_key_path.to_string_lossy();
    let witness_lib_path_str = witness_lib_path.to_string_lossy();
    let args = vec![
        "prove",
        "-e",
        elf_path_str.as_ref(),
        "-i",
        INPUT_PATH,
        "-a",
        "-u",
        "-f",
        "-k",
        proving_key_path_str.as_ref(),
        "-w",
        witness_lib_path_str.as_ref(),
        "-o",
        OUTPUT_DIR_PATH,
    ];

    let output = Command::new("cargo-zisk")
        .args(args)
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

    let path_to_proof = output_dir.join("vadcop_final_proof.bin");
    if let ProofFormat::Groth16 = format {
        // get the final snark wrapping
        info!(
            output_dir = %output_dir.display(),
            "ZisK prove complete; starting snark wrapping"
        );
        std::fs::create_dir_all(output_dir.join("proofs"))?;
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
        let proof_bytes = std::fs::read(output_dir.join("final_snark_proof.bin"))?;
        let publics_bytes = std::fs::read(output_dir.join("final_snark_publics.bin"))?;
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
