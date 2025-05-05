#![no_main]

use pico_sdk::io::{commit, read_as};

use ethrex_blockchain::{validate_block, validate_gas_used};
use ethrex_common::Address;
use ethrex_storage::AccountUpdate;
use ethrex_vm::Evm;
use std::collections::HashMap;
use zkvm_interface::{
    io::{ProgramInput, ProgramOutput},
    trie::{update_tries, verify_db},
    withdrawals::get_withdrawals_root,
};

pico_sdk::entrypoint!(main);

pub fn main() {
    let ProgramInput {
        blocks,
        parent_block_header,
        mut db,
        #[cfg(feature = "l2")]
        withdrawals_merkle_roots,
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

    for (i, block) in blocks.into_iter().enumerate() {
        let fork = db.chain_config.fork(block.header.timestamp);
        // Validate the block
        validate_block(&block, &parent_header, &db.chain_config).expect("invalid block");

        // Execute block
        let mut vm = Evm::from_execution_db(db.clone());
        let result = vm.execute_block(&block).expect("failed to execute block");
        let receipts = result.receipts;
        let account_updates = vm.get_state_transitions(fork)?;

        // Validate L2 withdrawals
        #[cfg(feature = "l2")]
        {
            if let Some(withdrawals_root) = withdrawals_merkle_roots.get(i) {
                if *withdrawals_root != get_withdrawals_root(&block.body.transactions, &receipts)? {
                    panic!("invalid withdrawals merkle root");
                }
            } else {
                panic!("failed to get withdrawal root for block");
            }
        }

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

    // Update state trie
    let acc_account_updates: Vec<AccountUpdate> = acc_account_updates.values().cloned().collect();
    update_tries(&mut state_trie, &mut storage_tries, &acc_account_updates)
        .expect("failed to update state and storage tries");

    // Calculate final state root hash and check
    let final_state_hash = state_trie.hash_no_commit();
    if final_state_hash != last_block_state_root {
        panic!("invalid final state trie");
    }

    commit(&ProgramOutput {
        initial_state_hash,
        final_state_hash,
    });
}
