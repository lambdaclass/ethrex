#![no_main]

use guest_program::{execution::execution_program, input::ProgramInput};
use rkyv::rancor::Error;

pico_sdk::entrypoint!(main);

pub fn main() {
    println!("reading input");
    let input = pico_sdk::io::read_vec();
    println!("deserializing input");
    let input = rkyv::from_bytes::<ProgramInput, Error>(&input).unwrap();

    println!("executing");
    let output = execution_program(input).unwrap();

    println!("committing output");
    pico_sdk::io::commit_bytes(&output.encode());
}
