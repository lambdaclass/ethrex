use std::io::Read;
use std::sync::Arc;

#[cfg(feature = "l2")]
use ethrex_guest_program::l2::run_guest;
#[cfg(not(feature = "l2"))]
use ethrex_guest_program::l1::run_stateless_guest;

use ethrex_guest_program::crypto::risc0::Risc0Crypto;
use risc0_zkvm::guest::env;

fn main() {
    println!("start reading input");
    let start = env::cycle_count();
    let mut input = Vec::new();
    env::stdin().read_to_end(&mut input).unwrap();
    let end = env::cycle_count();
    println!("end reading input, cycles: {}", end - start);

    let crypto = Arc::new(Risc0Crypto);

    println!("start execution");
    #[cfg(not(feature = "l2"))]
    let output = run_stateless_guest(&input, crypto);
    #[cfg(feature = "l2")]
    let output = run_guest(&input, crypto).unwrap();
    let end_exec = env::cycle_count();
    println!("end execution, cycles: {}", end_exec - end);

    println!("start committing public inputs");
    env::commit_slice(&output);
    let end_commit = env::cycle_count();
    println!(
        "end committing public inputs, cycles: {}",
        end_commit - end_exec
    );

    println!("total cycles: {}", end_commit - start);
}
