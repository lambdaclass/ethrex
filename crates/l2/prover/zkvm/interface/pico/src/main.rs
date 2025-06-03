#![no_main]

use pico_sdk::io::{commit, read_as};

use zkvm_interface::io::ProgramInput;

#[cfg(feature = "l2")]
use ethrex_l2_common::{
    get_block_deposits, get_block_withdrawal_hashes, compute_deposit_logs_hash, compute_withdrawals_merkle_root,
};
#[cfg(feature = "l2")]
use ethrex_common::types::blobs_bundle::{blob_from_bytes, kzg_commitment_to_versioned_hash};

pico_sdk::entrypoint!(main);

pub fn main() {
    let input: ProgramInput = read_as();
    let output = zkvm_interface::execution::execution_program(input).unwrap();

    commit(&output);
}
