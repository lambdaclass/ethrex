#![no_main]

use rkyv::rancor::Error;
use zkvm_interface::{execution::execution_program, io::ProgramInput};

sp1_zkvm::entrypoint!(main);

pub fn main() {
    let input = sp1_zkvm::io::read_vec();
    eprintln!("{}", input.len());
    let output =
        execution_program(rkyv::from_bytes::<ProgramInput, Error>(&input).unwrap()).unwrap();

    sp1_zkvm::io::commit(&output.encode());
}
