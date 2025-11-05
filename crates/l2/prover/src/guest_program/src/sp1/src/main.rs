#![no_main]

use guest_program::{execution::execution_program, input::ProgramInput};
use rkyv::rancor::Error;

sp1_zkvm::entrypoint!(main);

pub fn main() {
    println!("cycle-tracker-report-start: read_input");
    let mut input_bytes = sp1_zkvm::io::read_vec();
    let mut input_archived = rkyv::access_mut::<ArchivedProgramInput, Error>(&mut input_bytes);
    rkyv::munge::munge!(let ArchivedProgramInput { mut input, .. } = input_archived);

    println!("cycle-tracker-report-end: read_input");

    println!("cycle-tracker-report-start: execution");
    let output = execution_program(&mut input).unwrap();
    println!("cycle-tracker-report-end: execution");

    println!("cycle-tracker-report-start: commit_public_inputs");
    sp1_zkvm::io::commit_slice(&output.encode());
    println!("cycle-tracker-report-end: commit_public_inputs");
}
