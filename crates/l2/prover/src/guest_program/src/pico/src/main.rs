#![no_main]

use guest_program::{execution::execution_program, input::ProgramInput};
use rkyv::rancor::Error;

pico_sdk::entrypoint!(main);

pub fn main() {
    dbg!("reading input");
    let input = pico_sdk::io::read_vec();
    dbg!("deserializing input");
    let input = rkyv::from_bytes::<ProgramInput, Error>(&input).unwrap();

    dbg!("executing");
    let output = execution_program(input).unwrap();

    dbg!("committing output");
    pico_sdk::io::commit_bytes(&output.encode());
}
