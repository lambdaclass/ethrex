use std::{
    collections::hash_map::Entry,
    fmt::{Debug, Display},
};

// use alloy_primitives::{map::HashMap, B256};
use auto_impl::auto_impl;
// use reth_optimism_primitives::{OpBlock, OpReceipt};
// use reth_primitives::BlockWithSenders;
// use reth_provider::BlockExecutionOutput;
// use reth_storage_errors::provider::ProviderError;
// use reth_trie_common::updates::TrieUpdates;
// use revm::db::{BundleState, CacheDB};
// use revm_primitives::{
//     db::{Database, DatabaseRef},
//     Account, Address,
// };

pub mod error;
pub use error::Error;
pub mod frag;
pub use frag::DBFrag;
pub mod sorting;
pub use sorting::DBSorting;
pub mod state;
pub use state::State;

use crate::utils::time::BlockSyncTimers;

/// Database trait for all DB operations.
#[auto_impl(&, Arc)]
pub trait DatabaseWrite:
    Database<Error: Into<ProviderError> + Display> + Send + Sync + 'static + Clone + Debug
{
    fn commit_block_unchecked(
        &self,
        block: &BlockWithSenders<OpBlock>,
        block_execution_output: BlockExecutionOutput<OpReceipt>,
        trie_updates: TrieUpdates,
        timers: &mut BlockSyncTimers,
    ) -> Result<(), Error>;

    fn roll_back_head(&self) -> Result<(), Error>;
}

/// Database read functions
#[auto_impl(&, Arc)]
pub trait DatabaseRead:
    DatabaseRef<Error: Debug + Display + Into<ProviderError>> + Send + Sync + 'static + Clone + Debug
{
    /// Calculate the state root with the provided `BundleState` overlaid on the latest DB state.
    fn calculate_state_root(
        &self,
        bundle_state: &BundleState,
    ) -> Result<(B256, TrieUpdates), Error>;

    /// Returns the head block number, ie. the highest block number on the chain
    fn head_block_number(&self) -> Result<u64, Error>;

    /// Returns the head block hash, ie. the hash of the highest block on the chain
    fn head_block_hash(&self) -> Result<B256, Error>;
}

impl<DbRead: DatabaseRead> DatabaseRead for CacheDB<DbRead> {
    fn calculate_state_root(
        &self,
        bundle_state: &BundleState,
    ) -> Result<(B256, TrieUpdates), Error> {
        self.db.calculate_state_root(bundle_state)
    }

    fn head_block_number(&self) -> Result<u64, Error> {
        self.db.head_block_number()
    }

    fn head_block_hash(&self) -> Result<B256, Error> {
        self.db.head_block_hash()
    }
}

/// Converts cached state in a `CachedDB` into `BundleState`
pub fn state_changes_to_bundle_state<D: DatabaseRef>(
    db: &D,
    changes: HashMap<Address, Account>,
) -> Result<BundleState, D::Error> {
    let mut bundle_state = BundleState::builder(0..=2);

    for (address, account) in changes {
        if let Some(original_account_info) = db.basic_ref(address)? {
            bundle_state = bundle_state.state_original_account_info(address, original_account_info);
        }
        bundle_state = bundle_state.state_present_account_info(address, account.info);
        bundle_state = bundle_state.state_storage(
            address,
            account
                .storage
                .into_iter()
                .map(|(i, s)| (i, (s.original_value, s.present_value)))
                .collect(),
        );
    }
    Ok(bundle_state.build())
}

// This function is used to flatten a vector of state changes into a single HashMap.
// The idea is to merge the changes that happened to the same account across multiple transactions
// into a single "Account" struct that represents the final state of the accounts after all
// transactions.
pub fn flatten_state_changes(
    state_changes: Vec<HashMap<Address, Account>>,
) -> HashMap<Address, Account> {
    let mut flat_state_change_map: HashMap<Address, Account> = HashMap::default();

    for tx_state_changes in state_changes {
        update_state_changes(&mut flat_state_change_map, tx_state_changes);
    }

    flat_state_change_map
}

// This function is used to add state changes to an existing map of state changes.
// The idea is to merge the changes that happened to the same account across multiple transactions
// into a single "Account" struct that represents the final state of the accounts after all
// transactions.
pub fn update_state_changes(
    original_state_changes: &mut HashMap<Address, Account>,
    tx_state_changes: HashMap<Address, Account>,
) {
    for (address, mut new_account) in tx_state_changes {
        if !new_account.is_touched() {
            continue;
        }

        match original_state_changes.entry(address) {
            Entry::Occupied(mut entry) => {
                let db_account = entry.get_mut();
                let is_newly_created = new_account.is_created();

                // Set the storage
                if new_account.is_selfdestructed() {
                    db_account.storage.clear();
                } else if is_newly_created || !db_account.is_selfdestructed() {
                    db_account.storage.extend(new_account.storage);
                }

                if !db_account.is_selfdestructed() || is_newly_created {
                    // Set the info
                    db_account.info = new_account.info;
                    db_account.status = new_account.status;
                }
            }
            Entry::Vacant(entry) => {
                if new_account.is_selfdestructed() {
                    new_account.storage.clear();
                }
                entry.insert(new_account);
            }
        }
    }
}
