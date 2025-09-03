#![no_main]

use rkyv::rancor::Error;
use zkvm_interface::{execution::execution_program, io::ProgramInput};

sp1_zkvm::entrypoint!(main);

pub fn main() {
    println!("cycle-tracker-report-start: read_input");
    let input = sp1_zkvm::io::read_vec();
    let input = rkyv::from_bytes::<ProgramInput, Error>(&input).unwrap();
    println!("cycle-tracker-report-end: read_input");

    println!("cycle-tracker-report-start: execution");
    let output = execution_program(input).unwrap();
    println!("cycle-tracker-report-end: execution");

    println!("cycle-tracker-report-start: commit_public_inputs");
    sp1_zkvm::io::commit(&output.encode());
    println!("cycle-tracker-report-end: commit_public_inputs");
}
