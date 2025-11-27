#![no_main]

use guest_program::{execution::execution_program, input::ProgramInput};
use rkyv::rancor::Error;

pico_sdk::entrypoint!(main);

pub fn main() {
    let input = pico_sdk::io::read_vec();
    let input = rkyv::from_bytes::<ProgramInput, Error>(&input).unwrap();

    let output = execution_program(input).unwrap();

    pico_sdk::io::commit_bytes(&output.encode());
}
