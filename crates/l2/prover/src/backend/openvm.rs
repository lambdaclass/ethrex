use openvm_sdk::{Sdk, StdIn};

pub struct ProgramOutput(pub [u8; 32]);

const PROGRAM_ELF: &[u8] = include_bytes!("../../zkvm/interface/openvm/out/riscv32im-openvm-elf");

pub fn execute(input: zkvm_interface::io::ProgramInput) -> Result<(), Box<dyn std::error::Error>> {
    let sdk = Sdk::standard();

    let mut stdin = StdIn::default();
    stdin.write(&input);

    sdk.execute(PROGRAM_ELF.clone(), stdin.clone())?;
}

pub fn prove(
    _input: zkvm_interface::io::ProgramInput,
    _aligned_mode: bool,
) -> Result<ProgramOutput, Box<dyn std::error::Error>> {
    unimplemented!("OpenVM prove is not implemented yet");
}

pub fn to_batch_proof(
    _aligned_mode: bool,
) -> Result<ethrex_l2_common::prover::BatchProof, Box<dyn std::error::Error>> {
    unimplemented!("OpenVM to_batch_proof is not implemented yet");
}
