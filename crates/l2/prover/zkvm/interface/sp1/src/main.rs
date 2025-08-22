#![no_main]

use rkyv::rancor::Error;
use zkvm_interface::{
    execution::{execution_program, format_duration},
    io::ProgramInput,
};

sp1_zkvm::entrypoint!(main);

pub fn main() {
    let start = SystemTime::now();

    let input = sp1_zkvm::io::read_vec();

    let input_read_duration = start.elapsed().unwrap_or_else(|e| {
        panic!("SystemTime::elapsed failed: {e}");
    });

    println!("Read input in {}", format_duration(input_read_duration));

    let input = rkyv::from_bytes::<ProgramInput, Error>(&input).unwrap();

    let input_decode_duration = start.elapsed().unwrap_or_else(|e| {
        panic!("SystemTime::elapsed failed: {e}");
    });

    println!(
        "Decoded input in {}",
        format_duration(input_decode_duration - input_read_duration)
    );

    let output = execution_program(input).unwrap();

    let execution_duration = start.elapsed().unwrap_or_else(|e| {
        panic!("SystemTime::elapsed failed: {e}");
    });

    println!(
        "Executed program in {}",
        format_duration(execution_duration - input_decode_duration)
    );

    sp1_zkvm::io::commit(&output.encode());

    let commit_duration = start.elapsed().unwrap_or_else(|e| {
        panic!("SystemTime::elapsed failed: {e}");
    });

    println!(
        "Committed output in {}",
        format_duration(commit_duration - execution_duration)
    );
}
