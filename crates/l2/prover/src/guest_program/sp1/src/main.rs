#![no_main]

use ethrex_block_prover::{execution::execution_program, input::JSONProgramInput};

sp1_zkvm::entrypoint!(main);

pub fn main() {
    let input = sp1_zkvm::io::read::<JSONProgramInput>().0;
    let output = execution_program(input).unwrap();

    sp1_zkvm::io::commit(&output.encode());
}
