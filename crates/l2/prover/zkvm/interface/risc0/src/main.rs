use risc0_zkvm::guest::env;
use zkvm_interface::{execution::execution_program, io::ProgramInput};

fn main() {
    let input: ProgramInput = env::read();
    let output = execution_program(input).unwrap();

    env::commit_slice(&output.encode());
}
