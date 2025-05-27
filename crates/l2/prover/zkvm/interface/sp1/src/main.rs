#![no_main]

use ethrex_blockchain::{validate_block, validate_gas_used};
use ethrex_common::types::AccountUpdate;
use ethrex_common::Address;
use ethrex_vm::Evm;
use std::collections::HashMap;
#[cfg(feature = "l2")]
use zkvm_interface::deposits::{get_block_deposits, get_deposit_hash};
#[cfg(feature = "l2")]
use zkvm_interface::withdrawals::{get_block_withdrawals, get_withdrawals_merkle_root};
use zkvm_interface::{
    io::{ProgramInput, ProgramOutput},
    trie::{update_tries, verify_db},
};

sp1_zkvm::entrypoint!(main);

pub fn main() {
    let ProgramInput {
        blocks,
        parent_block_header,
        mut db,
        elasticity_multiplier,
    } = sp1_zkvm::io::read::<ProgramInput>();
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
    let mut withdrawals = vec![];
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
            let block_withdrawals = get_block_withdrawals(&block.body.transactions, &receipts)
                .expect("failed to get block withdrawals");
            let block_deposits = get_block_deposits(&block.body.transactions);
            let mut block_deposits_hashes = Vec::with_capacity(block_deposits.len());
            for deposit in block_deposits {
                block_deposits_hashes.push(
                    deposit
                        .get_deposit_hash()
                        .expect("Failed to get deposit hash for tx"),
                );
            }
            withdrawals.extend(block_withdrawals);
            deposits_hashes.extend(block_deposits_hashes);
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
    let Ok(withdrawals_merkle_root) = get_withdrawals_merkle_root(withdrawals) else {
        panic!("Failed to calculate withdrawals merkle root");
    };

    // Calculate L2 deposits logs root
    #[cfg(feature = "l2")]
    let Ok(deposit_logs_hash) = get_deposit_hash(deposits_hashes) else {
        panic!("Failed to calculate deposits logs hash");
    };

    // Update state trie
    let acc_account_updates: Vec<AccountUpdate> = acc_account_updates.values().cloned().collect();
    update_tries(&mut state_trie, &mut storage_tries, &acc_account_updates)
        .expect("failed to update state and storage tries");

    // Calculate final state root hash and check
    let final_state_hash = state_trie.hash_no_commit();
    if final_state_hash != last_block_state_root {
        panic!("invalid final state trie");
    }

    // Output gas for measurement purposes
    sp1_zkvm::io::commit(&cumulative_gas_used);

    sp1_zkvm::io::commit(
        &ProgramOutput {
            initial_state_hash,
            final_state_hash,
            #[cfg(feature = "l2")]
            withdrawals_merkle_root,
            #[cfg(feature = "l2")]
            deposit_logs_hash,
        }
        .encode(),
    );
}
