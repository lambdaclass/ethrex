use std::sync::Arc;

// use alloy_primitives::B256;
use parking_lot::RwLock;
// use revm_primitives::{
//     db::{Database, DatabaseRef},
//     AccountInfo, Address, Bytecode, EvmState, U256,
// };

use super::{DBFrag, State};

/// DB That is used when sorting a new frag
/// Thread safe
#[derive(Clone, Debug)]
pub struct DBSorting<Db> {
    pub db: Arc<RwLock<State<DBFrag<Db>>>>,
    state_id: u64,
}

impl<Db> DBSorting<Db> {
    pub fn new(frag_db: DBFrag<Db>) -> Self {
        Self {
            db: Arc::new(RwLock::new(State::new(frag_db))),
            state_id: rand::random(),
        }
    }

    pub fn state_id(&self) -> u64 {
        self.state_id
    }
}
impl<Db: DatabaseRef> DBSorting<Db> {
    pub fn commit_ref(&mut self, state: &EvmState) {
        self.db.write().commit_ref(state);
        self.state_id = rand::random()
    }
}

impl<Db: DatabaseRef> DatabaseRef for DBSorting<Db> {
    type Error = Db::Error;

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

impl<Db: DatabaseRef> Database for DBSorting<Db> {
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
