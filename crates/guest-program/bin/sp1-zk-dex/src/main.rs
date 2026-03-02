#![no_main]

use ethrex_guest_program::programs::zk_dex::execution::execution_program;
use ethrex_guest_program::programs::zk_dex::types::DexProgramInput;
use rkyv::rancor::Error;

sp1_zkvm::entrypoint!(main);

pub fn main() {
    let input = sp1_zkvm::io::read_vec();
    let input = rkyv::from_bytes::<DexProgramInput, Error>(&input).unwrap();

    let output = execution_program(input).unwrap();

    sp1_zkvm::io::commit_slice(&output.encode());
}
