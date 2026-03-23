#![no_main]

use ethrex_guest_program::common::app_execution::execute_app_circuit;
use ethrex_guest_program::common::app_types::AppProgramInput;
use ethrex_guest_program::programs::bridge::circuit::BridgeCircuit;
use rkyv::rancor::Error;

sp1_zkvm::entrypoint!(main);

/// Bridge guest program — uses the lightweight `execute_app_circuit` engine
/// instead of full EVM execution. Only processes deposits, withdrawals,
/// and ETH transfers. No app-specific operations.
///
/// This produces significantly faster proofs compared to evm-l2.
pub fn main() {
    println!("cycle-tracker-report-start: read_input");
    let input = sp1_zkvm::io::read_vec();
    let input = rkyv::from_bytes::<AppProgramInput, Error>(&input).unwrap();
    println!("cycle-tracker-report-end: read_input");

    println!("cycle-tracker-report-start: execution");
    let circuit = BridgeCircuit;
    let output = execute_app_circuit(&circuit, input).unwrap();
    println!("cycle-tracker-report-end: execution");

    println!("cycle-tracker-report-start: commit_public_inputs");
    sp1_zkvm::io::commit_slice(&output.encode());
    println!("cycle-tracker-report-end: commit_public_inputs");
}
