use risc0_zkvm::guest::env;

use zkvm_interface::io::ProgramInput;

fn main() {
    let input: ProgramInput = env::read();
    
    let cumulative_gas_used: u64 = input.blocks.iter().map(|b| b.header.gas_used).sum();
    let output = zkvm_interface::execution::execution_program(input).unwrap();

    env::write(&cumulative_gas_used);

    env::commit(&output);
}
