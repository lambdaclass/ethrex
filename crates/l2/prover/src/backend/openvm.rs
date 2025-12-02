use crate::ProofFormat;
use guest_program::input::ProgramInput;
use openvm_continuations::verifier::internal::types::VmStarkProof;
use openvm_sdk::{Sdk, StdIn, types::EvmProof};
use openvm_stark_sdk::config::baby_bear_poseidon2::BabyBearPoseidon2Config;
use rkyv::rancor::Error;

static PROGRAM_ELF: &[u8] = include_bytes!("../guest_program/src/openvm/out/riscv32im-openvm-elf");

pub enum ProveOutput {
    Compressed(VmStarkProof<BabyBearPoseidon2Config>),
    Groth16(EvmProof),
}

pub fn execute(input: ProgramInput) -> Result<(), Box<dyn std::error::Error>> {
    let sdk = Sdk::standard();

    let mut stdin = StdIn::default();
    let bytes = rkyv::to_bytes::<Error>(&input)?;
    stdin.write_bytes(bytes.as_slice());

    sdk.execute(PROGRAM_ELF, stdin.clone())?;

    Ok(())
}

pub fn execute_timed(
    input: ProgramInput,
) -> Result<std::time::Duration, Box<dyn std::error::Error>> {
    let sdk = Sdk::standard();

    let mut stdin = StdIn::default();
    let bytes = rkyv::to_bytes::<Error>(&input)?;
    stdin.write_bytes(bytes.as_slice());

    let start = std::time::Instant::now();
    sdk.execute(PROGRAM_ELF, stdin.clone())?;
    let duration = start.elapsed();

    Ok(duration)
}

pub fn prove(
    input: ProgramInput,
    format: ProofFormat,
) -> Result<ProveOutput, Box<dyn std::error::Error>> {
    let sdk = Sdk::standard();

    let mut stdin = StdIn::default();
    let bytes = rkyv::to_bytes::<Error>(&input)?;
    stdin.write_bytes(bytes.as_slice());

    let proof = match format {
        ProofFormat::Compressed => {
            let (proof, _) = sdk.prove(PROGRAM_ELF, stdin.clone())?;
            ProveOutput::Compressed(proof)
        }
        ProofFormat::Groth16 => {
            let proof = sdk.prove_evm(PROGRAM_ELF, stdin.clone())?;
            ProveOutput::Groth16(proof)
        }
    };

    Ok(proof)
}

pub fn prove_timed(
    input: ProgramInput,
    format: ProofFormat,
) -> Result<(ProveOutput, std::time::Duration), Box<dyn std::error::Error>> {
    let sdk = Sdk::standard();

    let mut stdin = StdIn::default();
    let bytes = rkyv::to_bytes::<Error>(&input)?;
    stdin.write_bytes(bytes.as_slice());

    let start = std::time::Instant::now();
    let proof = match format {
        ProofFormat::Compressed => {
            let (proof, _) = sdk.prove(PROGRAM_ELF, stdin.clone())?;
            ProveOutput::Compressed(proof)
        }
        ProofFormat::Groth16 => {
            let proof = sdk.prove_evm(PROGRAM_ELF, stdin.clone())?;
            ProveOutput::Groth16(proof)
        }
    };
    let duration = start.elapsed();

    Ok((proof, duration))
}

pub fn to_batch_proof(
    _proof: ProveOutput,
    _format: ProofFormat,
) -> Result<ethrex_l2_common::prover::BatchProof, Box<dyn std::error::Error>> {
    unimplemented!("OpenVM to_batch_proof is not implemented yet");
}
