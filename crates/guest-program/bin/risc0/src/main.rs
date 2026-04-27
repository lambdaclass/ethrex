use std::io::Read;
use std::sync::Arc;

#[cfg(feature = "l2")]
use ethrex_guest_program::l2::{ProgramInput, execution_program};
#[cfg(all(not(feature = "l2"), not(feature = "experimental-devnet")))]
use ethrex_guest_program::l1::{ProgramInput, execution_program};
#[cfg(all(not(feature = "l2"), feature = "experimental-devnet"))]
use ethrex_guest_program::l1::execution_program;

use ethrex_guest_program::crypto::risc0::Risc0Crypto;
use risc0_zkvm::guest::env;

fn main() {
    println!("start reading input");
    let start = env::cycle_count();
    let mut input = Vec::new();
    env::stdin().read_to_end(&mut input).unwrap();

    #[cfg(not(feature = "experimental-devnet"))]
    let input = {
        use rkyv::rancor::Error;
        rkyv::from_bytes::<ProgramInput, Error>(&input).unwrap()
    };
    let end = env::cycle_count();
    println!("end reading input, cycles: {}", end - start);

    let crypto = Arc::new(Risc0Crypto);

    println!("start execution");
    #[cfg(feature = "experimental-devnet")]
    let output = execution_program(&input, crypto).unwrap();
    #[cfg(not(feature = "experimental-devnet"))]
    let output = execution_program(input, crypto).unwrap();
    let end_exec = env::cycle_count();
    println!("end execution, cycles: {}", end_exec - end);

    println!("start committing public inputs");
    env::commit_slice(&output.encode());
    let end_commit = env::cycle_count();
    println!(
        "end committing public inputs, cycles: {}",
        end_commit - end_exec
    );

    println!("total cycles: {}", end_commit - start);
}
