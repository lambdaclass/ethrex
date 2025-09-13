use guest_program::{execution::execution_program, input::ProgramInput};
use rkyv::rancor::Error;
use sha2::{Digest, Sha256};

ziskos::entrypoint!(main);

pub fn main() {
    println!("start reading input");
    let input = ziskos::read_input();
    let input = rkyv::from_bytes::<ProgramInput, Error>(&input).unwrap();
    println!("finish reading input");

    println!("start execution");
    let output = execution_program(input).unwrap();
    println!("finish execution");

    println!("start hashing output");
    let output = Sha256::digest(output.encode());
    println!("finish hashing output");

    println!("start revealing output");
    output.chunks_exact(4).enumerate().for_each(|(idx, bytes)| {
        ziskos::set_output(idx, u32::from_le_bytes(bytes.try_into().unwrap()))
    });
    println!("finish revealing output");
}
