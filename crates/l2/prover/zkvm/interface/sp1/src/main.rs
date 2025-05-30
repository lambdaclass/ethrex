#![no_main]

use zkvm_interface::io::ProgramInput;

sp1_zkvm::entrypoint!(main);

pub fn main() {
    let input = sp1_zkvm::io::read::<ProgramInput>();
    let cumulative_gas_used: u64 = input.blocks.iter().map(|b| b.header.gas_used).sum();
    let output = zkvm_interface::execution::execution_program(input).unwrap();

    // Output gas for measurement purposes
    sp1_zkvm::io::commit(&cumulative_gas_used);

    sp1_zkvm::io::commit(&output.encode());
}
