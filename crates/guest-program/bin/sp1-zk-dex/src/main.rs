#![no_main]

use ethrex_common::Address;
use ethrex_guest_program::common::app_execution::execute_app_circuit;
use ethrex_guest_program::common::app_types::AppProgramInput;
use ethrex_guest_program::programs::zk_dex::circuit::DexCircuit;
use rkyv::rancor::Error;

sp1_zkvm::entrypoint!(main);

/// DEX contract address on the L2 (build-time placeholder).
const DEX_CONTRACT_ADDRESS: Address = Address([0xDE; 20]);

pub fn main() {
    println!("cycle-tracker-report-start: read_input");
    let input = sp1_zkvm::io::read_vec();
    let input = rkyv::from_bytes::<AppProgramInput, Error>(&input).unwrap();
    println!("cycle-tracker-report-end: read_input");

    println!("cycle-tracker-report-start: execution");
    let circuit = DexCircuit {
        contract_address: DEX_CONTRACT_ADDRESS,
    };
    let output = execute_app_circuit(&circuit, input).unwrap();
    println!("cycle-tracker-report-end: execution");

    println!("cycle-tracker-report-start: commit_public_inputs");
    sp1_zkvm::io::commit_slice(&output.encode());
    println!("cycle-tracker-report-end: commit_public_inputs");
}
