use std::{
    io::ErrorKind,
    process::{Command, Stdio},
    sync::OnceLock,
    time::{Duration, Instant},
};
use zisk_common::io::ZiskStdin;
use zisk_sdk::{Asm, Proof, ProverClient, ZiskProveResult, ZiskProver};

use ethrex_l2_common::prover::{BatchProof, ProofFormat};
use guest_program::{ZKVM_ZISK_PROGRAM_ELF, input::ProgramInput, output::ProgramOutput};

const INPUT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/zisk_input.bin");
const OUTPUT_DIR_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/zisk_output");
const ELF_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/zkvm-zisk-program");

pub struct ProveOutput(pub Vec<u8>);

pub static PROVE_CLIENT: OnceLock<ZiskProver<Asm>> = OnceLock::new();
pub static EXECUTE_CLIENT: OnceLock<ZiskProver<Asm>> = OnceLock::new();

pub fn execute_client() -> &'static ZiskProver<Asm> {
    if PROVE_CLIENT.get().is_some() {
        panic!(
            "ZisK prover was previously initialized for proving, which is not allowed because of MPI requiring to be initialized just once."
        );
    }
    EXECUTE_CLIENT.get_or_init(|| {
        ProverClient::builder()
            .asm()
            .verify_constraints()
            .elf_path(ELF_PATH.into())
            .unlock_mapped_memory(true)
            .build()
            .unwrap_or_else(|e| panic!("Failed to setup ZisK prover client: {e}"))
    })
}

pub fn prove_client() -> &'static ZiskProver<Asm> {
    if EXECUTE_CLIENT.get().is_some() {
        panic!(
            "ZisK prover was previously initialized for execution, which is not allowed because of MPI requiring to be initialized just once."
        );
    }
    PROVE_CLIENT.get_or_init(|| {
        ProverClient::builder()
            .asm()
            .prove()
            .aggregation(true)
            .elf_path(ELF_PATH.into())
            .unlock_mapped_memory(true)
            .build()
            .unwrap_or_else(|e| panic!("Failed to setup ZisK prover client: {e}"))
    })
}

pub fn execute(input: ProgramInput) -> Result<(), Box<dyn std::error::Error>> {
    write_elf_file()?;
    let stdin_bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&input)?.to_vec();
    let stdin = ZiskStdin::from_vec(stdin_bytes);

    let client = execute_client();
    client.execute(stdin)?;

    Ok(())
}

pub fn prove(
    input: ProgramInput,
    format: ProofFormat,
) -> Result<Proof, Box<dyn std::error::Error>> {
    write_elf_file()?;
    let stdin_bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&input)?.to_vec();
    let stdin = ZiskStdin::from_vec(stdin_bytes);

    let client = prove_client();
    let proof = client.prove(stdin)?.proof;
    Ok(proof)
}

pub fn verify(_output: &ProgramOutput) -> Result<(), Box<dyn std::error::Error>> {
    unimplemented!("verify is not implemented for ZisK backend")
}

pub fn to_batch_proof(
    proof: Proof,
    format: ProofFormat,
) -> Result<BatchProof, Box<dyn std::error::Error>> {
    unimplemented!("to_batch_proof is not implemented for ZisK backend")
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
