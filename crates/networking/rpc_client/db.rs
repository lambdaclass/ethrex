use crate::constants::{CANCUN_CONFIG, RPC_RATE_LIMIT};
use crate::NodeRLP;
use crate::{get_account, get_block, get_storage, retry};
use std::cell::RefCell;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fs::File;

use crate::Account;
use ethrex_core::types::{AccountInfo, ChainConfig, GenesisAccount};
use ethrex_core::{
    types::{Block, TxKind},
    Address, H256,
};
use ethrex_storage::error::StoreError;
use ethrex_storage::{EngineType, Store};
use ethrex_vm::execution_db::{ExecutionDB, ToExecDB};
use ethrex_vm::spec_id;
use futures_util::future::join_all;
use revm::db::CacheDB;
use revm::DatabaseRef;
use revm_primitives::{
    AccountInfo as RevmAccountInfo, Address as RevmAddress, Bytecode as RevmBytecode,
    Bytes as RevmBytes, B256 as RevmB256, U256 as RevmU256,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tokio_utils::RateLimiter;

#[derive(Serialize, Deserialize)]
pub struct RpcDB {
    pub rpc_url: String,
    pub block_number: usize,
    // we concurrently download tx callers before pre-execution to minimize sequential RPC calls
    #[serde(
        serialize_with = "serialize_refcell",
        deserialize_with = "deserialize_refcell"
    )]
    pub cache: RefCell<HashMap<Address, Option<Account>>>,
    #[serde(
        serialize_with = "serialize_refcell",
        deserialize_with = "deserialize_refcell"
    )]
    pub block_hashes: RefCell<HashMap<u64, H256>>,
}

fn serialize_refcell<T, S>(value: &RefCell<T>, serializer: S) -> Result<S::Ok, S::Error>
where
    T: Serialize,
    S: Serializer,
{
    value.borrow().serialize(serializer)
}

fn deserialize_refcell<'de, T, D>(deserializer: D) -> Result<RefCell<T>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    T::deserialize(deserializer).map(RefCell::new)
}

impl RpcDB {
    pub async fn with_cache(
        rpc_url: &str,
        block_number: usize,
        block: &Block,
    ) -> Result<Self, String> {
        let mut db = RpcDB {
            rpc_url: rpc_url.to_string(),
            block_number,
            cache: RefCell::new(HashMap::new()),
            block_hashes: RefCell::new(HashMap::new()),
        };

        db.cache_accounts(block).await?;

        Ok(db)
    }

    async fn cache_accounts(&mut self, block: &Block) -> Result<(), String> {
        let txs = &block.body.transactions;

        let callers = txs.iter().map(|tx| tx.sender());
        let to = txs.iter().filter_map(|tx| match tx.to() {
            TxKind::Call(to) => Some(to),
            TxKind::Create => None,
        });
        let accessed_storage: Vec<_> = txs.iter().flat_map(|tx| tx.access_list()).collect();

        // dedup accounts and concatenate accessed storage keys
        let mut accounts = HashMap::new();
        for (address, keys) in callers
            .chain(to)
            .map(|address| (address, Vec::new()))
            .chain(accessed_storage)
        {
            accounts
                .entry(address)
                .or_insert_with(Vec::new)
                .extend(keys);
        }
        let accounts: Vec<_> = accounts.into_iter().collect();
        *self.cache.borrow_mut() = self.fetch_accounts(&accounts).await?;

        Ok(())
    }

    async fn fetch_accounts(
        &self,
        accounts: &[(Address, Vec<H256>)],
    ) -> Result<HashMap<Address, Option<Account>>, String> {
        let rate_limiter = RateLimiter::new(std::time::Duration::from_secs(1));
        let mut fetched = HashMap::new();

        let mut counter = 0;
        for chunk in accounts.chunks(RPC_RATE_LIMIT) {
            let futures = chunk.iter().map(|(address, storage_keys)| async move {
                Ok((
                    *address,
                    retry(|| get_account(&self.rpc_url, self.block_number, address, storage_keys))
                        .await?,
                ))
            });

            let fetched_chunk = rate_limiter
                .throttle(|| async { join_all(futures).await })
                .await
                .into_iter()
                .collect::<Result<HashMap<_, _>, String>>()?;

            fetched.extend(fetched_chunk);

            counter += chunk.len();
            println!("fetched {} accounts of {}", counter, accounts.len());
        }

        Ok(fetched)
    }

    pub fn to_in_memory_store(
        &self,
        block: Block,
        chain_config: &ChainConfig,
    ) -> Result<Store, StoreError> {
        let store = Store::new("test", EngineType::InMemory)?;

        let block_hash = block.hash();

        // Store block data
        let block_number: u64 = self.block_number.try_into().unwrap();
        store.add_block(block)?;
        store.set_canonical_block(block_number, block_hash)?;
        store.update_latest_block_number(block_number)?;
        store.update_earliest_block_number(block_number)?;

        // Store genesis state trie
        let genesis_accs: HashMap<Address, GenesisAccount> = self
            .cache
            .borrow()
            .iter()
            .filter_map(|(addr, opt_acc)| {
                opt_acc.as_ref().map(|acc| {
                    let acc_c = acc.clone();
                    (
                        addr.clone(),
                        GenesisAccount {
                            code: acc_c.code.unwrap_or_default(),
                            storage: acc_c.storage,
                            balance: acc_c.account_state.balance,
                            nonce: acc_c.account_state.nonce,
                        },
                    )
                })
            })
            .collect();
        store.setup_genesis_state_trie(genesis_accs)?;
        store.set_chain_config(chain_config)?;

        // Add account states and codes
        /*for (address, account) in db.cache.borrow().iter() {
            if let Some(account) = account {
                // Add code if present
                if let Some(code) = &account.code {
                    store.add_account_code(account.account_state.code_hash, code.clone())?;
                }

                // Create account update with storage
                let mut update = AccountUpdate::new(*address);
                update.info = Some(AccountInfo {
                    nonce: account.account_state.nonce,
                    balance: account.account_state.balance,
                    code_hash: account.account_state.code_hash,
                });
                update.added_storage = account.storage.clone();

                // Apply account update
                let res = store.apply_account_updates(block_hash, &[update])?;
                dbg!(&res);
            }
        }*/

        dbg!(&store);
        Ok(store)
    }

    pub fn serialize_to_file(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let file = File::create(path)?;
        let writer = std::io::BufWriter::new(file);
        bincode::serialize_into(writer, self)?;
        Ok(())
    }

    pub fn deserialize_from_file(path: &str) -> Option<Self> {
        if let Ok(file) = File::open("db.bin") {
            bincode::deserialize_from(file).ok()
        } else {
            None
        }
    }
}

impl DatabaseRef for RpcDB {
    type Error = String;

    fn basic_ref(&self, address: RevmAddress) -> Result<Option<RevmAccountInfo>, Self::Error> {
        let address = Address::from(address.0.as_ref());

        let account = match self.cache.borrow_mut().entry(address) {
            Entry::Occupied(entry) => entry.get().clone(),
            Entry::Vacant(entry) => {
                println!("retrieving account info for address {address}");
                let handle = tokio::runtime::Handle::current();
                let account = tokio::task::block_in_place(|| {
                    handle.block_on(get_account(&self.rpc_url, self.block_number, &address, &[]))
                })?;
                entry.insert(account.clone());
                account
            }
        };

        let account = account.map(|account| RevmAccountInfo {
            nonce: account.account_state.nonce,
            balance: RevmU256::from_limbs(account.account_state.balance.0),
            code_hash: RevmB256::from(account.account_state.code_hash.0),
            code: account
                .code
                .map(|code| RevmBytecode::new_raw(RevmBytes(code))),
        });

        Ok(account)
    }
    #[allow(unused_variables)]
    fn code_by_hash_ref(&self, code_hash: RevmB256) -> Result<RevmBytecode, Self::Error> {
        Ok(RevmBytecode::default()) // code is stored in account info
    }
    fn storage_ref(&self, address: RevmAddress, index: RevmU256) -> Result<RevmU256, Self::Error> {
        let address = Address::from(address.0.as_ref());
        let index = H256::from_slice(&index.to_be_bytes_vec());

        let value = match self.cache.borrow_mut().entry(address) {
            Entry::Occupied(mut entry) => {
                let Some(account) = entry.get() else {
                    return Err("account doesn't exists".to_string());
                };
                match account.storage.get(&index) {
                    Some(value) => *value,
                    None => {
                        println!("retrieving storage value for address {address} and key {index}");
                        let handle = tokio::runtime::Handle::current();
                        let account = tokio::task::block_in_place(|| {
                            handle.block_on(get_account(
                                &self.rpc_url,
                                self.block_number,
                                &address,
                                &account
                                    .storage
                                    .keys()
                                    .chain(&[index])
                                    .cloned()
                                    .collect::<Vec<_>>(),
                            ))
                        })?
                        .expect("previously downloaded account doesn't exists");
                        let value = *account
                            .storage
                            .get(&index)
                            .expect("rpc account response didn't include requested storage value");
                        entry.insert(Some(account));
                        value
                    }
                }
            }
            Entry::Vacant(entry) => {
                let handle = tokio::runtime::Handle::current();
                let account = tokio::task::block_in_place(|| {
                    handle.block_on(get_account(
                        &self.rpc_url,
                        self.block_number,
                        &address,
                        &[index],
                    ))
                })?;

                if let Some(account) = entry.insert(account) {
                    *account
                        .storage
                        .get(&index)
                        .expect("rpc account response didn't include requested storage value")
                } else {
                    return Err("account doesn't exists".to_string());
                }
            }
        };

        Ok(RevmU256::from_limbs(value.0))
    }
    fn block_hash_ref(&self, number: u64) -> Result<RevmB256, Self::Error> {
        let hash = match self.block_hashes.borrow_mut().entry(number) {
            Entry::Occupied(entry) => *entry.get(),
            Entry::Vacant(entry) => {
                println!("retrieving block hash for block number {number}");
                let handle = tokio::runtime::Handle::current();
                let hash = tokio::task::block_in_place(|| {
                    handle.block_on(get_block(&self.rpc_url, number as usize))
                })
                .map(|block| block.hash())?;
                entry.insert(hash);
                hash
            }
        };

        Ok(RevmB256::from(hash.0))
    }
}

impl ToExecDB for RpcDB {
    fn to_exec_db(
        &self,
        block: &Block,
    ) -> Result<ethrex_vm::execution_db::ExecutionDB, ethrex_vm::errors::ExecutionDBError> {
        let parent_hash = block.header.parent_hash;
        let chain_config: ethrex_core::types::ChainConfig = CANCUN_CONFIG;

        // pre-execute and get all downloaded accounts
        let CacheDB { db, .. } = ExecutionDB::pre_execute(
            block,
            chain_config.chain_id,
            spec_id(&chain_config, block.header.timestamp),
            self,
        )
        .unwrap(); // TODO: remove unwrap
        let cache = db.cache.borrow();

        let cache_iter = cache.iter().filter_map(|(address, account)| {
            if let Some(account) = account {
                Some((address, account))
            } else {
                None
            }
        });

        let accounts: HashMap<_, _> = cache_iter
            .clone()
            .map(|(address, account)| {
                (
                    *address,
                    AccountInfo {
                        code_hash: account.account_state.code_hash,
                        balance: account.account_state.balance,
                        nonce: account.account_state.nonce,
                    },
                )
            })
            .collect();
        let code = cache_iter
            .clone()
            .map(|(_, account)| {
                (
                    account.account_state.code_hash,
                    account.code.clone().unwrap_or_default(),
                )
            })
            .collect();
        let storage = cache_iter
            .clone()
            .map(|(address, account)| (*address, account.storage.clone()))
            .collect();
        let block_hashes = self
            .block_hashes
            .borrow()
            .iter()
            .map(|(num, hash)| (*num, *hash))
            .collect();

        let storage_proofs = cache_iter
            .clone()
            .map(|(address, account)| {
                let storage_root = account
                    .storage_proofs
                    .first()
                    .and_then(|nodes| nodes.first())
                    .cloned();
                let other_storage_nodes: Vec<NodeRLP> = account
                    .storage_proofs
                    .iter()
                    .flat_map(|proofs| proofs.iter().skip(1).cloned())
                    .collect();
                (*address, (storage_root, other_storage_nodes))
            })
            .collect();

        let state_root = cache_iter
            .clone()
            .next()
            .clone()
            .and_then(|(_, account)| account.account_proof.first().cloned());
        let other_state_nodes = cache_iter
            .flat_map(|(_, account)| account.account_proof.iter().skip(1).cloned())
            .collect();
        let state_proofs = (state_root, other_state_nodes);

        Ok(ExecutionDB {
            accounts,
            code,
            storage,
            block_hashes,
            chain_config,
            state_proofs,
            storage_proofs,
        })
    }
}
