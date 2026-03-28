#![no_main]

use std::sync::Arc;

#[cfg(feature = "l2")]
use ethrex_guest_program::l2::{ProgramInput, execution_program};
#[cfg(all(not(feature = "l2"), not(feature = "stateless-validation")))]
use ethrex_guest_program::l1::{ProgramInput, execution_program};
#[cfg(all(not(feature = "l2"), feature = "stateless-validation"))]
use ethrex_guest_program::l1::{decode_eip8025, execution_program};

use ethrex_guest_program::crypto::sp1::Sp1Crypto;

sp1_zkvm::entrypoint!(main);

pub fn main() {
    println!("cycle-tracker-report-start: read_input");
    let input = sp1_zkvm::io::read_vec();

    #[cfg(feature = "stateless-validation")]
    let (new_payload_request, execution_witness) = decode_eip8025(&input).unwrap();
    #[cfg(not(feature = "stateless-validation"))]
    let input = {
        use rkyv::rancor::Error;
        rkyv::from_bytes::<ProgramInput, Error>(&input).unwrap()
    };
    println!("cycle-tracker-report-end: read_input");

    let crypto = Arc::new(Sp1Crypto);

    println!("cycle-tracker-report-start: execution");
    #[cfg(feature = "stateless-validation")]
    let output = execution_program(new_payload_request, execution_witness, crypto).unwrap();
    #[cfg(not(feature = "stateless-validation"))]
    let output = execution_program(input, crypto).unwrap();
    println!("cycle-tracker-report-end: execution");

    println!("cycle-tracker-report-start: commit_public_inputs");
    sp1_zkvm::io::commit_slice(&output.encode());
    println!("cycle-tracker-report-end: commit_public_inputs");
}
