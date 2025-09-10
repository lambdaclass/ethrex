use guest_program::input::ProgramInput;
use openvm_sdk::{Sdk, StdIn};

pub struct ProgramOutput(pub [u8; 32]);

static PROGRAM_ELF: &[u8] = include_bytes!("../guest_program/src/openvm/target/riscv32im-risc0-zkvm-elf/release/zkvm-openvm-program");

pub fn execute(input: ProgramInput) -> Result<(), Box<dyn std::error::Error>> {
    let sdk = Sdk::standard();

    let mut stdin = StdIn::default();
    stdin.write(&input);

    sdk.execute(PROGRAM_ELF.clone(), stdin.clone())?;

    Ok(())
}

pub fn prove(
    _input: ProgramInput,
    _aligned_mode: bool,
) -> Result<ProgramOutput, Box<dyn std::error::Error>> {
    unimplemented!("OpenVM prove is not implemented yet");
}

pub fn to_batch_proof(
    _aligned_mode: bool,
) -> Result<ethrex_l2_common::prover::BatchProof, Box<dyn std::error::Error>> {
    unimplemented!("OpenVM to_batch_proof is not implemented yet");
}
