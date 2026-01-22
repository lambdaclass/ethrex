#[cfg(feature = "l2")]
use ethrex_guest_program::l2::{ProgramInput, execution_program};
#[cfg(not(feature = "l2"))]
use ethrex_guest_program::l1::{ProgramInput, execution_program};

use openvm_keccak256::keccak256;
use rkyv::rancor::Error;

openvm::init!();

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
