#![no_main]

use zkvm_interface::io::ProgramInput;

#[cfg(feature = "l2")]
use ethrex_l2_common::{
    get_block_deposits, get_block_withdrawal_hashes, compute_deposit_logs_hash,
    compute_withdrawals_merkle_root, StateDiff
};
#[cfg(feature = "l2")]
use ethrex_common::types::blobs_bundle::{blob_from_bytes, kzg_commitment_to_versioned_hash};

sp1_zkvm::entrypoint!(main);

pub fn main() {
    let input = sp1_zkvm::io::read::<ProgramInput>();
    let output = zkvm_interface::execution::execution_program(input).unwrap();

    sp1_zkvm::io::commit(&output.encode());
}
