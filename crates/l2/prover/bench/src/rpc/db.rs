use std::collections::HashMap;

use crate::constants::{CANCUN_CONFIG, RPC_RATE_LIMIT};
use crate::rpc::{get_account, get_block, retry};

use bytes::Bytes;
use ethrex_common::{
    types::{AccountInfo, AccountState, Block, Fork, TxKind},
    Address, H256, U256,
};
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::db::Database as LevmDatabase;
use ethrex_storage::{hash_address, hash_key};
use ethrex_trie::{Node, PathRLP, Trie};
use ethrex_vm::backends::levm::{CacheDB, LEVM};
use ethrex_vm::{ExecutionDB, ExecutionDBError};
use futures_util::future::join_all;
use tokio_utils::RateLimiter;

use ethrex_levm::db::error::DatabaseError;
use std::sync::Arc;
use std::sync::Mutex;

use super::{Account, NodeRLP};

#[derive(Clone)]
pub struct RpcDB {
    pub rpc_url: String,
    pub block_number: usize,
    // we concurrently download tx callers before pre-execution to minimize sequential RPC calls
    pub cache: Arc<Mutex<HashMap<Address, Account>>>,
    pub child_cache: Arc<Mutex<HashMap<Address, Account>>>,
    pub block_hashes: Arc<Mutex<HashMap<u64, H256>>>,
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
            cache: Arc::new(Mutex::new(HashMap::new())),
            child_cache: Arc::new(Mutex::new(HashMap::new())),
            block_hashes: Arc::new(Mutex::new(HashMap::new())),
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
        *self.cache.lock().unwrap() = self.fetch_accounts(&accounts, false).await?;

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
            let mut child_cache = self.child_cache.lock().unwrap();
            for (address, account) in &fetched {
                let acc_account_mut = child_cache.get_mut(address);
                if let Some(acc_account) = acc_account_mut {
                    match (account, acc_account) {
                        (
                            Account::Existing {
                                storage,
                                storage_proofs,
                                ..
                            },
                            Account::Existing {
                                storage: storage_acc,
                                storage_proofs: storage_proofs_acc,
                                ..
                            },
                        ) => {
                            storage_acc.extend(storage);
                            storage_proofs_acc.extend(storage_proofs.clone());
                        }
                        (
                            Account::NonExisting { storage_proofs, .. },
                            Account::NonExisting {
                                storage_proofs: storage_proofs_acc,
                                ..
                            },
                        ) => {
                            storage_proofs_acc.extend(storage_proofs.clone());
                        }
                        _ => {
                            unreachable!()
                        }
                    };
                } else {
                    child_cache.insert(*address, account.clone());
                }
            }
        } else {
            let mut cache = self.cache.lock().unwrap();
            for (address, account) in &fetched {
                let acc_account_mut = cache.get_mut(address);
                if let Some(acc_account) = acc_account_mut {
                    match (account, acc_account) {
                        (
                            Account::Existing {
                                storage,
                                storage_proofs,
                                ..
                            },
                            Account::Existing {
                                storage: storage_acc,
                                storage_proofs: storage_proofs_acc,
                                ..
                            },
                        ) => {
                            storage_acc.extend(storage);
                            storage_proofs_acc.extend(storage_proofs.clone());
                        }
                        (
                            Account::NonExisting { storage_proofs, .. },
                            Account::NonExisting {
                                storage_proofs: storage_proofs_acc,
                                ..
                            },
                        ) => {
                            storage_proofs_acc.extend(storage_proofs.clone());
                        }
                        _ => {
                            unreachable!()
                        }
                    };
                } else {
                    cache.insert(*address, account.clone());
                }
            }
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

    pub fn to_exec_db(&self, block: &Block) -> Result<ethrex_vm::ExecutionDB, ExecutionDBError> {
        // TODO: Simplify this function and potentially merge with the implementation for
        // StoreWrapper.

        let chain_config = *CANCUN_CONFIG;
        let fork = chain_config.fork(block.header.timestamp);

        let mut db = GeneralizedDatabase::new(Arc::new(self.clone()), CacheDB::new());

        // pre-execute and get all state changes
        let result = LEVM::execute_block(block, &mut db).map_err(Box::new)?;
        let execution_updates =
            LEVM::get_state_transitions(&mut db, Fork::default()).map_err(Box::new)?;

        let index: Vec<(Address, Vec<H256>)> = self
            .cache
            .lock()
            .unwrap()
            .iter()
            .map(|(address, account)| match account {
                Account::Existing {
                    account_state,
                    storage,
                    account_proof,
                    storage_proofs,
                    code,
                } => (*address, storage.keys().cloned().collect()),
                Account::NonExisting {
                    account_proof,
                    storage_proofs,
                } => {
                    let address_account_update = execution_updates
                        .iter()
                        .find(|update| update.address == *address);

                    if let Some(update) = address_account_update {
                        (*address, update.added_storage.keys().cloned().collect())
                    } else {
                        (*address, vec![])
                    }
                }
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
            .filter_map(|(address, proof)| get_potential_child_nodes(proof, &hash_address(address)))
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
        }

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
            .lock()
            .unwrap()
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

impl LevmDatabase for RpcDB {
    fn account_exists(&self, address: Address) -> bool {
        // look into the cache
        {
            if self
                .cache
                .lock()
                .unwrap()
                .get(&address)
                .is_some_and(|account| matches!(account, Account::Existing { .. }))
            {
                return true;
            }
        }
        self.fetch_account_blocking(address, &[], false)
            .is_ok_and(|account| matches!(account, Account::Existing { .. }))
    }

    fn get_account_code(&self, _code_hash: H256) -> Result<Option<Bytes>, DatabaseError> {
        Ok(None) // code is stored in account info
    }

    fn get_account(
        &self,
        address: Address,
    ) -> std::result::Result<ethrex_common::types::Account, ethrex_levm::db::error::DatabaseError>
    {
        let cache = self.cache.lock().unwrap();
        let account = if let Some(account) = cache.get(&address).cloned() {
            account
        } else {
            drop(cache);
            self.fetch_accounts_blocking(&[(address, vec![])], false)
                .map_err(|e| DatabaseError::Custom(format!("Failed to fetch account info: {e}")))?
                .get(&address)
                .unwrap()
                .clone()
        };
        if let Account::Existing {
            account_state,
            code,
            ..
        } = account
        {
            Ok(ethrex_common::types::Account::new(
                account_state.balance,
                code.clone().unwrap_or_default(),
                account_state.nonce,
                HashMap::new(),
            ))
        } else {
            Ok(ethrex_common::types::Account::default())
        }
    }

    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError> {
        let account = self
            .fetch_accounts_blocking(&[(address, vec![key])], false)
            .map_err(|e| DatabaseError::Custom(format!("Failed to fetch account info: {e}")))?
            .get(&address)
            .unwrap()
            .clone();
        if let Account::Existing { storage, .. } = account {
            if let Some(value) = storage.get(&key) {
                Ok(*value)
            } else {
                Ok(U256::zero())
            }
        } else {
            Ok(U256::zero())
        }
    }

    fn get_block_hash(&self, block_number: u64) -> Result<Option<H256>, DatabaseError> {
        let handle = tokio::runtime::Handle::current();
        let hash = tokio::task::block_in_place(|| {
            handle.block_on(retry(|| get_block(&self.rpc_url, block_number as usize)))
        })
        .map_err(|e| DatabaseError::Custom(e))
        .map(|block| block.hash())?;
        self.block_hashes.lock().unwrap().insert(block_number, hash);
        Ok(Some(hash))
    }

    fn get_chain_config(&self) -> ethrex_common::types::ChainConfig {
        *CANCUN_CONFIG
    }
}

/// Get all potential child nodes of a node whose value was deleted.
///
/// After deleting a value from a (partial) trie it's possible that the node containing the value gets
/// replaced by its child, whose prefix is possibly modified by appending some nibbles to it.
/// If we don't have this child node (because we're modifying a partial trie), then we can't
/// perform the deletion. If we have the final proof of exclusion of the deleted value, we can
/// calculate all posible child nodes.
fn get_potential_child_nodes(proof: &[NodeRLP], key: &PathRLP) -> Option<Vec<Node>> {
    // TODO: Perhaps it's possible to calculate the child nodes instead of storing all possible ones.
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
                    node.partial.next();
                    !node.partial.is_empty() // skip the last nibble, which is the leaf flag.
                                             // if we encode a leaf with its flag missing, it’s going to be encoded as an
                                             // extension.
                } {}
                Some(variants)
            }
            _ => None,
        }
    } else {
        None
    }
}
