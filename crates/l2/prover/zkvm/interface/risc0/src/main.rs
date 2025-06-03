use risc0_zkvm::guest::env;

use zkvm_interface::io::ProgramInput;

fn main() {
    let input: ProgramInput = env::read();
    let output = zkvm_interface::execution::execution_program(input).unwrap();

    env::commit(&output);
}
