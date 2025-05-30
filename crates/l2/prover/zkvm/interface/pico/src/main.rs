#![no_main]

use pico_sdk::io::{commit, read_as};

use ethrex_blockchain::{validate_block, validate_gas_used};
use ethrex_common::Address;
use ethrex_common::types::AccountUpdate;
use ethrex_vm::Evm;
use std::collections::HashMap;
use zkvm_interface::{
    io::{ProgramInput, ProgramOutput},
    trie::{update_tries, verify_db},
};

#[cfg(feature = "l2")]
use ethrex_l2_common::{
    get_block_deposits, get_block_withdrawal_hashes, compute_deposit_logs_hash, compute_withdrawals_merkle_root,
};
#[cfg(feature = "l2")]
use ethrex_common::types::blobs_bundle::{blob_from_bytes, kzg_commitment_to_versioned_hash};

pico_sdk::entrypoint!(main);

pub fn main() {
    let ProgramInput {
        blocks,
        parent_block_header,
        mut db,
        elasticity_multiplier,
        #[cfg(feature = "l2")]
        state_diff,
        #[cfg(feature = "l2")]
        blob_commitment,
        #[cfg(feature = "l2")]
        blob_proof,
    } = read_as();
    // Tries used for validating initial and final state root
    let (mut state_trie, mut storage_tries) = db
        .get_tries()
        .expect("failed to build state and storage tries or state is not valid");

    // Validate the initial state
    let initial_state_hash = state_trie.hash_no_commit();
    if initial_state_hash != parent_block_header.state_root {
        panic!("invalid initial state trie");
    }
    if !verify_db(&db, &state_trie, &storage_tries).expect("failed to validate database") {
        panic!("invalid database")
    };

    let last_block = blocks.last().expect("empty batch");
    let last_block_state_root = last_block.header.state_root;
    let mut parent_header = parent_block_header;
    let mut acc_account_updates: HashMap<Address, AccountUpdate> = HashMap::new();

    let mut cumulative_gas_used = 0;

    #[cfg(feature = "l2")]
    let mut withdrawal_hashes = vec![];
    #[cfg(feature = "l2")]
    let mut deposits_hashes = vec![];

    for block in blocks {
        // Validate the block
        validate_block(
            &block,
            &parent_header,
            &db.chain_config,
            elasticity_multiplier,
        )
        .expect("invalid block");

        // Execute block
        let mut vm = Evm::from_prover_db(db.clone());
        let result = vm.execute_block(&block).expect("failed to execute block");
        let receipts = result.receipts;
        let account_updates = vm
            .get_state_transitions()
            .expect("failed to get state transitions");

        // Get L2 withdrawals and deposits for this block
        #[cfg(feature = "l2")]
        {
            let txs = block.body.transactions;
            let block_deposits = get_block_deposits(&txs);

            let txs_and_receipts: Vec<_> = txs.into_iter().zip(receipts.clone().into_iter()).collect();
            let block_withdrawal_hashes = get_block_withdrawal_hashes(&txs_and_receipts).expect("failed to retrieve withdrawal hashes");

            let mut block_deposit_hashes = Vec::with_capacity(block_deposits.len());
            for deposit in &block_deposits {
                block_deposit_hashes.push(
                    deposit
                        .get_deposit_hash()
                        .expect("Failed to get deposit hash for tx"),
                );
            }
            withdrawal_hashes.extend(block_withdrawal_hashes);
            deposits_hashes.extend(block_deposit_hashes);
        }

        cumulative_gas_used += receipts
            .last()
            .map(|last_receipt| last_receipt.cumulative_gas_used)
            .unwrap_or_default();

        // Update db for the next block
        db.apply_account_updates(&account_updates);

        // Update acc_account_updates
        for account in account_updates {
            let address = account.address;
            if let Some(existing) = acc_account_updates.get_mut(&address) {
                existing.merge(account);
            } else {
                acc_account_updates.insert(address, account);
            }
        }

        validate_gas_used(&receipts, &block.header).expect("invalid gas used");
        parent_header = block.header;
    }

    // Calculate L2 withdrawals root
    #[cfg(feature = "l2")]
    let Ok(withdrawals_merkle_root) = compute_withdrawals_merkle_root(withdrawal_hashes) else {
        panic!("Failed to calculate withdrawals merkle root");
    };

    // Calculate L2 deposits logs root
    #[cfg(feature = "l2")]
    let Ok(deposit_logs_hash) = compute_deposit_logs_hash(deposits_hashes) else {
        panic!("Failed to calculate deposits logs hash");
    };

    // Update state trie
    update_tries(
        &mut state_trie,
        &mut storage_tries,
        &acc_account_updates.values().cloned().collect::<Vec<_>>(),
    )
    .expect("failed to update state and storage tries");

    // Calculate final state root hash and check
    let final_state_hash = state_trie.hash_no_commit();
    if final_state_hash != last_block_state_root {
        panic!("invalid final state trie");
    }

    // TODO: this could be replaced with something like a ProverConfig.
    let validium = (blob_commitment, blob_proof) == ([0; 48], [0; 48]);

    // Check state diffs are valid
    #[cfg(feature = "l2")]
    if !validium {
        let state_diff_updates = state_diff
            .to_account_updates(&state_trie)
            .expect("failed to calculate account updates from state diffs");

        if state_diff_updates != acc_account_updates {
            panic!("invalid state diffs")
        }
    }

    // Verify KZG blob proof
    #[cfg(feature = "l2")]
    let blob_versioned_hash = if validium { 
        ethrex_common::H256::zero()
    } else {
        use kzg_rs::{dtypes::Blob, kzg_proof::KzgProof, trusted_setup::get_kzg_settings};

        let encoded_state_diff = state_diff.encode().expect("failed to encode state diff");
        let blob_data = blob_from_bytes(encoded_state_diff)
            .expect("failed to convert encoded state diff into blob data");
        let blob = Blob::from_slice(&blob_data).expect("failed to convert blob data into Blob");

        let blob_proof_valid = KzgProof::verify_blob_kzg_proof(
            blob,
            &kzg_rs::Bytes48::from_slice(&blob_commitment)
                .expect("failed type conversion for blob commitment"),
            &kzg_rs::Bytes48::from_slice(&blob_proof)
                .expect("failed type conversion for blob proof"),
            &get_kzg_settings(),
        )
        .expect("failed to verify blob proof (neither valid or invalid proof)");

        if !blob_proof_valid {
            panic!("invalid blob proof");
        }

        kzg_commitment_to_versioned_hash(&blob_commitment)
    };

    commit(&ProgramOutput {
        initial_state_hash,
        final_state_hash,
        #[cfg(feature = "l2")]
        withdrawals_merkle_root,
        #[cfg(feature = "l2")]
        deposit_logs_hash,
        #[cfg(feature = "l2")]
        blob_versioned_hash,
    });
}