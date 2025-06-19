#![no_main]

use zkvm_interface::{io::ProgramInput, execution::execution_program};

sp1_zkvm::entrypoint!(main);

pub fn main() {
    let input = sp1_zkvm::io::read::<ProgramInput>();
    let output = execution_program(input).unwrap();

    sp1_zkvm::io::commit_slice(&output.encode());
}
