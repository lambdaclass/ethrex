#![no_main]

use zkvm_interface::{execution::execution_program, io::JSONProgramInput};

sp1_zkvm::entrypoint!(main);

pub fn main() {
    dbg!("INSIDE SP1");
    let input = sp1_zkvm::io::read::<JSONProgramInput>().0;
    dbg!("FINISH INPUT PARSING");
    let output = execution_program(input).unwrap();
    dbg!("FINISH EXECUTION PROGRAM");

    sp1_zkvm::io::commit(&output.encode());
}
