use guest_program::input::ProgramInput;
use openvm_sdk::{Sdk, StdIn, types::EvmProof};
use rkyv::rancor::Error;

pub struct ProgramOutput(pub [u8; 32]);

static PROGRAM_ELF: &[u8] = include_bytes!(
    "../guest_program/src/openvm/target/riscv32im-risc0-zkvm-elf/release/zkvm-openvm-program"
);

pub fn execute(input: ProgramInput) -> Result<(), Box<dyn std::error::Error>> {
    let sdk = Sdk::standard();

    let mut stdin = StdIn::default();
    let bytes = rkyv::to_bytes::<Error>(&input)?;
    stdin.write_bytes(bytes.as_slice());

    sdk.execute(PROGRAM_ELF.clone(), stdin.clone())?;

    Ok(())
}

pub fn prove(
    input: ProgramInput,
    _aligned_mode: bool,
) -> Result<EvmProof, Box<dyn std::error::Error>> {
    let sdk = Sdk::standard();

    let mut stdin = StdIn::default();
    let bytes = rkyv::to_bytes::<Error>(&input)?;
    stdin.write_bytes(bytes.as_slice());

    Ok(sdk.prove_evm(PROGRAM_ELF.clone(), stdin.clone())?)
}

pub fn to_batch_proof(
    _aligned_mode: bool,
) -> Result<ethrex_l2_common::prover::BatchProof, Box<dyn std::error::Error>> {
    unimplemented!("OpenVM to_batch_proof is not implemented yet");
}
