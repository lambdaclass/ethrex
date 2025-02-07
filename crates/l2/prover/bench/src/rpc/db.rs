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
use ethrex_storage::{hash_address, hash_key};
use ethrex_trie::{Node, PathRLP, Trie};
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
    pub child_cache: RefCell<HashMap<Address, Account>>,
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
            child_cache: RefCell::new(HashMap::new()),
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
        *self.cache.borrow_mut() = self.fetch_accounts(&accounts, false).await?;

        Ok(())
    }

    async fn fetch_accounts(
        &self,
        index: &[(Address, Vec<H256>)],
        from_child: bool,
    ) -> Result<HashMap<Address, Account>, String> {
        let rate_limiter = RateLimiter::new(std::time::Duration::from_secs(1));
        let block_number = if from_child {
            self.block_number + 1
        } else {
            self.block_number
        };

        let mut fetched = HashMap::new();
        let mut counter = 0;

        for chunk in index.chunks(RPC_RATE_LIMIT) {
            let futures = chunk.iter().map(|(address, storage_keys)| async move {
                Ok((
                    *address,
                    retry(|| get_account(&self.rpc_url, block_number, address, storage_keys))
                        .await?,
                ))
            });

            let fetched_chunk = rate_limiter
                .throttle(|| async { join_all(futures).await })
                .await
                .into_iter()
                .collect::<Result<HashMap<_, _>, String>>()?;

            fetched.extend(fetched_chunk);

            if index.len() == 1 {
                let address = chunk.first().unwrap().0;
                println!("fetched account {address}");
            } else {
                counter += chunk.len();
                println!("fetched {} accounts of {}", counter, index.len());
            }
        }

        if from_child {
            *self.child_cache.borrow_mut() = fetched.clone();
        } else {
            *self.cache.borrow_mut() = fetched.clone();
        }

        Ok(fetched)
    }

    async fn fetch_account(
        &self,
        address: Address,
        storage_keys: &[H256],
        from_child: bool,
    ) -> Result<Account, String> {
        self.fetch_accounts(&[(address, storage_keys.to_vec())], from_child)
            .await
            .map(|mut hashmap| hashmap.remove(&address).expect("account not present"))
    }

    fn fetch_accounts_blocking(
        &self,
        index: &[(Address, Vec<H256>)],
        from_child: bool,
    ) -> Result<HashMap<Address, Account>, String> {
        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| handle.block_on(self.fetch_accounts(index, from_child)))
    }

    fn fetch_account_blocking(
        &self,
        address: Address,
        storage_keys: &[H256],
        from_child: bool,
    ) -> Result<Account, String> {
        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| {
            handle.block_on(self.fetch_account(address, storage_keys, from_child))
        })
    }
}

impl DatabaseRef for RpcDB {
    type Error = String;

    fn basic_ref(&self, address: RevmAddress) -> Result<Option<RevmAccountInfo>, Self::Error> {
        let address = Address::from(address.0.as_ref());

        let account = {
            let cache_ref = self.cache.borrow();
            if let Some(account) = cache_ref.get(&address) {
                account.clone()
            } else {
                drop(cache_ref); // fetch_account_blocking mutably borrows the cache
                self.fetch_account_blocking(address, &[], false)?
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

        // TODO: this can be simplified
        let value = {
            let cache_ref = self.cache.borrow();
            if let Some(account) = cache_ref.get(&address) {
                let Account::Existing { storage, .. } = account else {
                    return Err("account doesn't exists".to_string());
                };
                match storage.get(&index) {
                    Some(value) => *value,
                    None => {
                        let storage_keys =
                            storage.keys().chain(&[index]).cloned().collect::<Vec<_>>();
                        drop(cache_ref); // fetch_account_blocking mutably borrows the cache
                        let account = self.fetch_account_blocking(address, &storage_keys, false)?;
                        let Account::Existing { storage, .. } = account else {
                            return Err("account doesn't exists".to_string());
                        };
                        storage[&index]
                    }
                }
            } else {
                drop(cache_ref); // fetch_account_blocking mutably borrows the cache
                let account = self.fetch_account_blocking(address, &[index], false)?;
                let Account::Existing { storage, .. } = account else {
                    return Err("account doesn't exists".to_string());
                };
                storage[&index]
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
                    handle.block_on(retry(|| get_block(&self.rpc_url, number as usize)))
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
        let chain_config = *CANCUN_CONFIG;

        // pre-execute and get cache db
        let cache_db = ExecutionDB::pre_execute(
            block,
            chain_config.chain_id,
            spec_id(&chain_config, block.header.timestamp),
            self,
        )
        .unwrap(); // TODO: remove unwrap

        // index read and touched account addresses and storage keys
        let index: Vec<_> = cache_db
            .accounts
            .iter()
            .map(|(address, account)| {
                let address = Address::from(address.0.as_ref());
                let storage_keys: Vec<_> = account
                    .storage
                    .keys()
                    .map(|key| H256::from_slice(&key.to_be_bytes_vec()))
                    .collect();
                (address, storage_keys)
            })
            .collect();

        // fetch all of them, both before and after block execution
        let initial_accounts = self.fetch_accounts_blocking(&index, false).unwrap();
        let final_accounts = self.fetch_accounts_blocking(&index, true).unwrap();
        // TODO: remove unwraps

        let initial_account_proofs = initial_accounts
            .iter()
            .map(|(_, account)| account.get_account_proof());
        let final_account_proofs = final_accounts
            .iter()
            .map(|(address, account)| (address, account.get_account_proof()));

        let initial_storage_proofs = initial_accounts
            .iter()
            .map(|(address, account)| (address, account.get_storage_proofs()));
        let final_storage_proofs = final_accounts
            .iter()
            .map(|(address, account)| (address, account.get_storage_proofs()));

        // get potential child nodes of deleted nodes after execution
        let potential_account_child_nodes = final_account_proofs
            .filter_map(|(address, proof)| {
                get_potential_child_nodes(proof, &hash_address(&address))
            })
            .flat_map(|nodes| nodes.into_iter().map(|node| node.encode_raw()));

        let potential_storage_child_nodes: HashMap<_, _> = final_storage_proofs
            .map(|(address, proofs)| {
                let nodes: Vec<_> = proofs
                    .iter()
                    .filter_map(|(key, proof)| get_potential_child_nodes(proof, &hash_key(&key)))
                    .flat_map(|nodes| nodes.into_iter().map(|node| node.encode_raw()))
                    .collect();
                (address, nodes)
            })
            .collect();

        #[derive(Clone)]
        struct ExistingAccount<'a> {
            pub account_state: &'a AccountState,
            pub storage: &'a HashMap<H256, U256>,
            pub code: &'a Option<Bytes>,
            pub storage_proofs: &'a HashMap<H256, Vec<NodeRLP>>,
        };

        let existing_accs = initial_accounts.iter().filter_map(|(address, account)| {
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

        let state_root = initial_account_proofs
            .clone()
            .next()
            .clone()
            .and_then(|proof| proof.first().cloned());
        let other_state_nodes = initial_account_proofs
            .flat_map(|proof| proof.iter().skip(1).cloned())
            .chain(potential_account_child_nodes)
            .collect();
        let state_proofs = (state_root, other_state_nodes);

        let storage_proofs = initial_storage_proofs
            .map(|(address, proofs)| {
                let storage_root = proofs
                    .iter()
                    .next()
                    .and_then(|(_, nodes)| nodes.first())
                    .cloned();
                let other_storage_nodes: Vec<NodeRLP> = proofs
                    .iter()
                    .flat_map(|(_, proof)| proof.iter().skip(1).cloned())
                    .chain(
                        potential_storage_child_nodes
                            .get(address)
                            .cloned()
                            .unwrap_or_default(),
                    )
                    .collect();
                (*address, (storage_root, other_storage_nodes))
            })
            .collect();

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

fn get_potential_child_nodes(proof: &[NodeRLP], key: &PathRLP) -> Option<Vec<Node>> {
    let trie = Trie::from_nodes(
        proof.first(),
        &proof.iter().skip(1).cloned().collect::<Vec<_>>(),
    )
    .unwrap();

    // return some only if this is a proof of exclusion
    if trie.get(key).unwrap().is_none() {
        let final_node = Node::decode_raw(proof.last().unwrap()).unwrap();
        match final_node {
            Node::Extension(mut node) => {
                let mut variants = Vec::with_capacity(node.prefix.len());
                while {
                    variants.push(Node::from(node.clone()));
                    node.prefix.next().is_some()
                } {}
                Some(variants)
            }
            Node::Leaf(mut node) => {
                let mut variants = Vec::with_capacity(node.partial.len());
                while {
                    variants.push(Node::from(node.clone()));
                    node.partial.next().is_some()
                } {}
                Some(variants)
            }
            _ => None,
        }
    } else {
        None
    }
}
