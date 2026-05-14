use std::sync::Arc;

#[cfg(feature = "l2")]
use ethrex_guest_program::l2::{ProgramInput, execution_program};
#[cfg(all(not(feature = "l2"), not(feature = "eip-8025")))]
use ethrex_guest_program::l1::{ProgramInput, execution_program};
#[cfg(all(not(feature = "l2"), feature = "eip-8025"))]
use ethrex_guest_program::l1::execution_program;

use ethrex_guest_program::crypto::openvm::OpenVmCrypto;
use openvm_keccak256::keccak256;

openvm::init!();

pub fn main() {
    openvm::io::println("start reading input");
    let input = openvm::io::read_vec();

    #[cfg(not(feature = "eip-8025"))]
    let input = {
        use rkyv::rancor::Error;
        rkyv::from_bytes::<ProgramInput, Error>(&input).unwrap()
    };
    openvm::io::println("finish reading input");

    let crypto = Arc::new(OpenVmCrypto);

    openvm::io::println("start execution");
    #[cfg(feature = "eip-8025")]
    let output = execution_program(&input, crypto).unwrap();
    #[cfg(not(feature = "eip-8025"))]
    let output = execution_program(input, crypto).unwrap();
    openvm::io::println("finish execution");

    openvm::io::println("start hashing output");
    let output = keccak256(&output.encode());
    openvm::io::println("finish hashing output");

    openvm::io::println("start revealing output");
    openvm::io::reveal_bytes32(output);
    openvm::io::println("finish revealing output");
}
