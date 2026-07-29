#![no_main]

use std::sync::Arc;

#[cfg(feature = "l2")]
use ethrex_guest_program::l2::{ProgramInput, execution_program};
#[cfg(all(
    not(feature = "l2"),
    not(feature = "eip-8025"),
    not(feature = "stateless-validator")
))]
use ethrex_guest_program::l1::{ProgramInput, execution_program};
#[cfg(all(
    not(feature = "l2"),
    feature = "eip-8025",
    not(feature = "stateless-validator")
))]
use ethrex_guest_program::l1::execution_program;

use ethrex_guest_program::crypto::sp1::Sp1Crypto;
#[cfg(all(not(feature = "eip-8025"), not(feature = "stateless-validator")))]
use rkyv::rancor::Error;

sp1_zkvm::entrypoint!(main);

pub fn main() {
    println!("cycle-tracker-report-start: read_input");
    let input = sp1_zkvm::io::read_vec();

    #[cfg(feature = "stateless-validator")]
    {
        println!("cycle-tracker-report-end: read_input");
        println!("cycle-tracker-report-start: execution");
        let output =
            ethrex_stateless_validator::run_stateless_validation(&input, Arc::new(Sp1Crypto));
        println!("cycle-tracker-report-end: execution");
        println!("cycle-tracker-report-start: commit_public_inputs");
        sp1_zkvm::io::commit_slice(&output);
        println!("cycle-tracker-report-end: commit_public_inputs");
    }

    #[cfg(not(feature = "stateless-validator"))]
    {
        #[cfg(not(feature = "eip-8025"))]
        let input = { rkyv::from_bytes::<ProgramInput, Error>(&input).unwrap() };
        println!("cycle-tracker-report-end: read_input");

        let crypto = Arc::new(Sp1Crypto);

        println!("cycle-tracker-report-start: execution");
        #[cfg(feature = "eip-8025")]
        let output = execution_program(&input, crypto).unwrap();
        #[cfg(not(feature = "eip-8025"))]
        let output = execution_program(input, crypto).unwrap();
        println!("cycle-tracker-report-end: execution");

        println!("cycle-tracker-report-start: commit_public_inputs");
        sp1_zkvm::io::commit_slice(&output.encode());
        println!("cycle-tracker-report-end: commit_public_inputs");
    }
}
