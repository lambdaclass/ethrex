#![no_main]

use rkyv::rancor::Error;
use zkvm_interface::{execution::execution_program, io::ProgramInput};

sp1_zkvm::entrypoint!(main);

pub fn main() {
    let input = sp1_zkvm::io::read_vec();
    let input = rkyv::from_bytes::<ProgramInput, Error>(&input).unwrap();

    let output = execution_program(input).unwrap();

    // Commit the contract public inputs
    sp1_zkvm::io::commit_slice(&output.encode_contract_pis());
}
