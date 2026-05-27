#![no_main]

use std::sync::Arc;

#[cfg(feature = "l2")]
use ethrex_guest_program::l2::execution_program;
#[cfg(not(feature = "l2"))]
use ethrex_guest_program::l1::execution_program;

use ethrex_guest_program::crypto::sp1::Sp1Crypto;

sp1_zkvm::entrypoint!(main);

pub fn main() {
    println!("cycle-tracker-report-start: read_input");
    let input = sp1_zkvm::io::read_vec();
    println!("cycle-tracker-report-end: read_input");

    let crypto = Arc::new(Sp1Crypto);

    println!("cycle-tracker-report-start: execution");
    let output = execution_program(&input, crypto).unwrap();
    println!("cycle-tracker-report-end: execution");

    println!("cycle-tracker-report-start: commit_public_inputs");
    sp1_zkvm::io::commit_slice(&output.encode());
    println!("cycle-tracker-report-end: commit_public_inputs");
}
