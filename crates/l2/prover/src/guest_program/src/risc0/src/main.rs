use guest_program::{execution::execution_program, input::JSONProgramInput};
use risc0_zkvm::guest::env;

fn main() {
    println!("start reading input");
    let start = env::cycle_count();
    let input: JSONProgramInput = env::read();
    let end = env::cycle_count();
    println!("end reading input, cycles: {}", end - start);

    println!("start execution");
    let output = execution_program(input.0).unwrap();
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
