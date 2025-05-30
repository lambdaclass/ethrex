#![no_main]

use pico_sdk::io::{commit, read_as};

use zkvm_interface::io::ProgramInput;

pico_sdk::entrypoint!(main);

pub fn main() {
    let input: ProgramInput = read_as();
    
    let cumulative_gas_used: u64 = input.blocks.iter().map(|b| b.header.gas_used).sum();
    let output = zkvm_interface::execution::execution_program(input).unwrap();

    commit(&output);
}
