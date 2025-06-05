#![no_main]

use zkvm_interface::io::ProgramInput;

sp1_zkvm::entrypoint!(main);

pub fn main() {
    let input = sp1_zkvm::io::read::<ProgramInput>();
    let output = zkvm_interface::execution::execution_program(input).unwrap();

    sp1_zkvm::io::commit(&output.encode());
}
