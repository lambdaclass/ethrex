use std::sync::Arc;
// use alloy_primitives::{map::HashMap, B256};
use parking_lot::RwLock;
use rand::RngCore;
// use reth_trie_common::updates::TrieUpdates;
// use revm::db::{states::bundle_state::BundleRetention, BundleState};
// use revm_primitives::{
//     db::{Database, DatabaseCommit, DatabaseRef},
//     Account, AccountInfo, Address, Bytecode, U256,
// };
use super::{DatabaseRead, Error, State};
use crate::utils::transaction::SimulatedTx;

/// This is a wrapper around db to tag frags onto before
/// sealing the block and commmiting it to db.
/// Each time a frag is done being sorted it gets applied and
/// a new DBSorting gets created around db_frag to which individual
/// txs will be attached
/// Furthermore, this db is shared with the RPC serving layer, hence it
/// should not be outright overwritten. Only the internal db should be
/// changed when a sorted frag is applied or (reset) when a block is sealed and committed to
/// the persisted underlying db.
#[derive(Clone, Debug)]
pub struct DBFrag<Db> {
    pub db: Arc<RwLock<State<Db>>>,
    /// Unique identifier for the state in the db
    state_id: u64,
}

impl<Db> DBFrag<Db> {
    /// Used on block commit to clear State, ready for next round of Sequenced Frags
    pub fn reset(&mut self) {
        self.db.write().reset();
        self.state_id = rand::rng().next_u64();
    }

    pub fn state_id(&self) -> u64 {
        self.state_id
    }

    pub fn is_valid(&self, sim_res_state: u64) -> bool {
        sim_res_state == self.state_id
    }
}

impl<Db: DatabaseRef> DBFrag<Db> {
    pub fn commit_txs<'a>(&mut self, txs: impl Iterator<Item = &'a mut SimulatedTx>) {
        let mut guard = self.db.write();

        for t in txs {
            let evm_state = std::mem::take(&mut t.result_and_state.state);
            for a in evm_state.keys() {
                let _ = guard.load_cache_account(*a);
            }
            let transitions = guard.cache.apply_evm_state(&evm_state);
            if let Some(s) = guard.transition_state.as_mut() {
                s.add_transitions(transitions)
            }
        }

        self.state_id = rand::random()
    }

    pub fn get_nonce(&self, address: Address) -> Result<u64, Error> {
        self.basic_ref(address)
            .map(|acc| acc.map(|acc| acc.nonce).unwrap_or_default())
            .map_err(|_| Error::Other("failed to get nonce".to_string()))
    }

    pub fn get_balance(&self, address: Address) -> Result<U256, Error> {
        self.basic_ref(address)
            .map(|acc| acc.map(|acc| acc.balance).unwrap_or_default())
            .map_err(|_| Error::Other("failed to get nonce".to_string()))
    }

    pub fn take_state_changes(&self) -> BundleState {
        self.db.write().merge_transitions(BundleRetention::Reverts);
        self.db.write().take_bundle()
    }
}

impl<Db: DatabaseRef> DatabaseRef for DBFrag<Db> {
    type Error = <Db as DatabaseRef>::Error;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.db.read().basic_ref(address)
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.db.read().code_by_hash_ref(code_hash)
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.db.read().storage_ref(address, index)
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        self.db.read().block_hash_ref(number)
    }
}

impl<Db: DatabaseRef> Database for DBFrag<Db> {
    type Error = <Db as DatabaseRef>::Error;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.db.write().basic(address)
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.db.write().code_by_hash(code_hash)
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.db.write().storage(address, index)
    }

    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        self.db.write().block_hash(number)
    }
}

impl<Db: DatabaseRef> DatabaseCommit for DBFrag<Db> {
    fn commit(&mut self, changes: HashMap<Address, Account>) {
        self.db.write().commit_ref(&changes)
    }
}

impl<Db: DatabaseRead> DatabaseRead for DBFrag<Db> {
    fn calculate_state_root(
        &self,
        bundle_state: &BundleState,
    ) -> Result<(B256, TrieUpdates), Error> {
        // todo!();
        self.db.read().database.calculate_state_root(bundle_state)
    }

    fn head_block_number(&self) -> Result<u64, Error> {
        self.db.read().database.head_block_number()
    }

    fn head_block_hash(&self) -> Result<B256, Error> {
        self.db.read().database.head_block_hash()
    }
}

impl<Db: DatabaseRead + Database> From<Db> for DBFrag<Db> {
    fn from(value: Db) -> Self {
        let state = State::builder()
            .with_database(value)
            .with_bundle_update()
            .without_state_clear()
            .build();
        Self {
            db: Arc::new(RwLock::new(state)),
            state_id: rand::random(),
        }
    }
}
