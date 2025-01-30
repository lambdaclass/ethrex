use std::cell::RefCell;
use std::collections::hash_map::Entry;
use std::collections::HashMap;

use crate::constants::{CANCUN_CONFIG, RPC_RATE_LIMIT};
use crate::rpc::{get_account, get_block, get_storage, retry};

use bytes::Bytes;
use ethrex_core::types::{AccountInfo, AccountState};
use ethrex_core::U256;
use ethrex_core::{
    types::{Account as CoreAccount, Block, TxKind},
    Address, H256,
};
use ethrex_vm::execution_db::{ExecutionDB, ToExecDB};
use ethrex_vm::spec_id;
use futures_util::future::join_all;
use revm::db::CacheDB;
use revm::DatabaseRef;
use revm_primitives::{
    AccountInfo as RevmAccountInfo, Address as RevmAddress, Bytecode as RevmBytecode,
    Bytes as RevmBytes, B256 as RevmB256, U256 as RevmU256,
};
use tokio_utils::RateLimiter;

use super::{Account, NodeRLP};

pub struct RpcDB {
    pub rpc_url: String,
    pub block_number: usize,
    // we concurrently download tx callers before pre-execution to minimize sequential RPC calls
    pub cache: RefCell<HashMap<Address, Account>>,
    pub block_hashes: RefCell<HashMap<u64, H256>>,
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
    ) -> Result<HashMap<Address, Account>, String> {
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

        if let Account::Existing {
            account_state,
            storage,
            code,
            ..
        } = account
        {
            Ok(Some(RevmAccountInfo {
                nonce: account_state.nonce,
                balance: RevmU256::from_limbs(account_state.balance.0),
                code_hash: RevmB256::from(account_state.code_hash.0),
                code: code.map(|code| RevmBytecode::new_raw(RevmBytes(code))),
            }))
        } else {
            Ok(None)
        }
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
                let Account::Existing { storage, .. } = entry.get() else {
                    return Err("account doesn't exists".to_string());
                };
                match storage.get(&index) {
                    Some(value) => *value,
                    None => {
                        println!("retrieving storage value for address {address} and key {index}");
                        let handle = tokio::runtime::Handle::current();
                        let account = tokio::task::block_in_place(|| {
                            handle.block_on(get_account(
                                &self.rpc_url,
                                self.block_number,
                                &address,
                                &storage.keys().chain(&[index]).cloned().collect::<Vec<_>>(),
                            ))
                        })?;
                        let value = match &account {
                            Account::Existing { storage, .. } => *storage.get(&index).expect(
                                "rpc account response didn't include requested storage value",
                            ),

                            Account::NonExisting { .. } => {
                                return Err("account doesn't exists".to_string());
                            }
                        };
                        entry.insert(account);
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

                if let Account::Existing { storage, .. } = entry.insert(account) {
                    *storage
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
        let chain_config = CANCUN_CONFIG;

        // pre-execute and get all downloaded accounts
        let CacheDB { db, .. } = ExecutionDB::pre_execute(
            block,
            chain_config.chain_id,
            spec_id(&chain_config, block.header.timestamp),
            self,
        )
        .unwrap(); // TODO: remove unwrap
        let cache = db.cache.borrow();

        #[derive(Clone)]
        struct ExistingAccount<'a> {
            pub account_state: &'a AccountState,
            pub storage: &'a HashMap<H256, U256>,
            pub code: &'a Option<Bytes>,
            pub storage_proofs: &'a Vec<Vec<NodeRLP>>,
        };

        let existing_accs = cache.iter().filter_map(|(address, account)| {
            if let Account::Existing {
                account_state,
                storage,
                code,
                storage_proofs,
                ..
            } = account
            {
                Some((
                    address,
                    ExistingAccount {
                        account_state,
                        storage,
                        code,
                        storage_proofs,
                    },
                ))
            } else {
                None
            }
        });

        let accounts: HashMap<_, _> = existing_accs
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
        let code = existing_accs
            .clone()
            .map(|(_, account)| {
                (
                    account.account_state.code_hash,
                    account.code.clone().unwrap_or_default(),
                )
            })
            .collect();
        let storage = existing_accs
            .clone()
            .map(|(address, account)| (*address, account.storage.clone()))
            .collect();
        let block_hashes = self
            .block_hashes
            .borrow()
            .iter()
            .map(|(num, hash)| (*num, *hash))
            .collect();

        let storage_proofs = existing_accs
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

        let account_proofs = cache.iter().map(|(_, account)| match account {
            Account::Existing { account_proof, .. } => account_proof,
            Account::NonExisting { proof } => proof,
        });
        let state_root = account_proofs
            .clone()
            .next()
            .clone()
            .and_then(|proof| proof.first().cloned());
        let other_state_nodes = account_proofs
            .flat_map(|proof| proof.iter().skip(1).cloned())
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
