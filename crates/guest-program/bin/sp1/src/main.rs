#![no_main]

use std::sync::Arc;

#[cfg(feature = "l2")]
use ethrex_guest_program::l2::run_guest;
#[cfg(not(feature = "l2"))]
use ethrex_guest_program::l1::run_stateless_guest;

use ethrex_guest_program::crypto::sp1::Sp1Crypto;

sp1_zkvm::entrypoint!(main);

pub fn main() {
    println!("cycle-tracker-report-start: read_input");
    let input = sp1_zkvm::io::read_vec();
    println!("cycle-tracker-report-end: read_input");

    let crypto = Arc::new(Sp1Crypto);

    println!("cycle-tracker-report-start: execution");
    #[cfg(not(feature = "l2"))]
    let output = run_stateless_guest(&input, crypto);
    #[cfg(feature = "l2")]
    let output = run_guest(&input, crypto).unwrap();
    println!("cycle-tracker-report-end: execution");

    println!("cycle-tracker-report-start: commit_public_inputs");
    sp1_zkvm::io::commit_slice(&output);
    println!("cycle-tracker-report-end: commit_public_inputs");
}
