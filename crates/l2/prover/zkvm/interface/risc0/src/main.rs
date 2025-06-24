use risc0_zkvm::guest::env;
use zkvm_interface::{io::ProgramInput, execution::execution_program};

fn main() {
    let input: ProgramInput = env::read();
    let output = execution_program(input).unwrap();

    env::commit(&output);
}
