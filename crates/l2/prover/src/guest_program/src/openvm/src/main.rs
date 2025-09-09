use openvm_keccak256::keccak256;
use rkyv::rancor::Error;
use guest_program::{execution::execution_program, input::ProgramInput};

//openvm::init!();

pub fn main() {
    openvm::io::println("start reading input");
    let input = openvm::io::read_vec();
    let input = rkyv::from_bytes::<ProgramInput, Error>(&input).unwrap();
    openvm::io::println("finish reading input");

    openvm::io::println("start execution");
    let output = execution_program(input).unwrap();
    openvm::io::println("finish execution");

    openvm::io::println("start hashing output");
    let output = keccak256(&output.encode());
    openvm::io::println("finish hashing output");

    openvm::io::println("start revealing output");
    openvm::io::reveal_bytes32(output);
    openvm::io::println("finish revealing output");
}
