#![no_main]

#[cfg(feature = "l2")]
use ethrex_guest_program::l2::{ProgramInput, execution_program};
#[cfg(not(any(feature = "l2", feature = "eip-8025")))]
use ethrex_guest_program::l1::{ProgramInput, execution_program};

use rkyv::rancor::Error;

sp1_zkvm::entrypoint!(main);

pub fn main() {
    #[cfg(feature = "eip-8025")]
    {
        eip8025_main();
    }
    #[cfg(not(feature = "eip-8025"))]
    {
        println!("cycle-tracker-report-start: read_input");
        let input = sp1_zkvm::io::read_vec();
        let input = rkyv::from_bytes::<ProgramInput, Error>(&input).unwrap();
        println!("cycle-tracker-report-end: read_input");

        println!("cycle-tracker-report-start: execution");
        let output = execution_program(input).unwrap();
        println!("cycle-tracker-report-end: execution");

        println!("cycle-tracker-report-start: commit_public_inputs");
        sp1_zkvm::io::commit_slice(&output.encode());
        println!("cycle-tracker-report-end: commit_public_inputs");
    }
}

#[cfg(feature = "eip-8025")]
fn eip8025_main() {
    use ethrex_guest_program::l1::{Eip8025ProgramInput, eip8025_execution_program};

    println!("cycle-tracker-report-start: read_input");
    let input = sp1_zkvm::io::read_vec();
    let input = rkyv::from_bytes::<Eip8025ProgramInput, Error>(&input).unwrap();
    println!("cycle-tracker-report-end: read_input");

    println!("cycle-tracker-report-start: execution");
    let output = eip8025_execution_program(input);
    println!("cycle-tracker-report-end: execution");

    println!("cycle-tracker-report-start: commit_public_inputs");
    sp1_zkvm::io::commit_slice(&output.encode());
    println!("cycle-tracker-report-end: commit_public_inputs");
}
