use std::io::Read;

use guest_program::{execution::execution_program, input::ProgramInput};
use risc0_zkvm::guest::env;
use rkyv::rancor::Error;

fn main() {
    println!("start reading input");
    let start = env::cycle_count();
    let mut input = Vec::new();
    env::stdin().read_to_end(&mut input).unwrap();
    let input = rkyv::from_bytes::<ProgramInput, Error>(&input).unwrap();
    let end = env::cycle_count();
    println!("end reading input, cycles: {}", end - start);

    println!("start execution");
    let output = execution_program(input).unwrap();
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
