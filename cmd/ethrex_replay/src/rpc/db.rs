use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use crate::rpc::{get_account, get_block, retry};

use bytes::Bytes;
use ethrex_common::constants::EMPTY_KECCACK_HASH;
use ethrex_common::types::{ChainConfig, code_hash};
use ethrex_common::{
    Address, H256, U256,
    types::{AccountInfo, Block, TxKind},
};
use ethrex_levm::db::Database as LevmDatabase;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::errors::DatabaseError;
use ethrex_levm::vm::VMType;
use ethrex_rlp::encode::RLPEncode;
use ethrex_rpc::debug::execution_witness::RpcExecutionWitness;
use ethrex_storage::{hash_address, hash_key};
use ethrex_trie::{Node, PathRLP, Trie};
use ethrex_vm::backends::levm::LEVM;
use eyre::Context;
use futures_util::future::join_all;
use sha3::{Digest, Keccak256};
use tokio::time::sleep;
use tracing::{debug, info};

use std::sync::Arc;
use std::sync::Mutex;

use super::{Account, NodeRLP};

// 10 Requests Per second max cap. It's a conservative number that every Free tier of RPC Providers supports.
const RPC_RATE_LIMIT: usize = 10;
const RATE_LIMIT: Duration = Duration::from_millis(100);

/// Structure for a database that fetches data from an RPC endpoint on demand.
/// Caches already fetched data to minimize RPC calls.
/// Implements the `LevmDatabase` trait to be used as the db for execution.
#[derive(Clone)]
pub struct RpcDB {
    /// RPC endpoint URL.
    pub rpc_url: String,
    /// Block number of the actual block to execute.
    pub block_number: usize,
    /// Cache of already fetched accounts. This includes state, code, storage and proofs.
    /// Accounts in the parent block, i.e. the initial state of the execution.
    pub cache: Arc<Mutex<HashMap<Address, Account>>>,
    /// Cache of already fetched accounts. This includes state, code, storage and proofs.
    /// Accounts in the actual block being executed, i.e. the post-state.
    pub child_cache: Arc<Mutex<HashMap<Address, Account>>>,
    /// Cache of already fetched block hashes.
    pub block_hashes: Arc<Mutex<HashMap<u64, H256>>>,
    /// Cache of already fetched contract codes.
    pub codes: Arc<Mutex<HashMap<H256, Bytes>>>,
    /// Chain config of the blockchain.
    pub chain_config: ChainConfig,
    /// VM type (L1 or L2).
    pub vm_type: VMType,
}

impl RpcDB {
    pub fn new(
        rpc_url: &str,
        chain_config: ChainConfig,
        block_number: usize,
        vm_type: VMType,
    ) -> Self {
        RpcDB {
            rpc_url: rpc_url.to_string(),
            block_number,
            cache: Arc::new(Mutex::new(HashMap::new())),
            child_cache: Arc::new(Mutex::new(HashMap::new())),
            block_hashes: Arc::new(Mutex::new(HashMap::new())),
            codes: Arc::new(Mutex::new(HashMap::new())),
            chain_config,
            vm_type,
        }
    }

    /// Create a new RpcDB and pre-cache all known accounts touched by the block.
    pub async fn with_cache(
        rpc_url: &str,
        chain_config: ChainConfig,
        block_number: usize,
        block: &Block,
        vm_type: VMType,
    ) -> eyre::Result<Self> {
        let mut db = RpcDB::new(rpc_url, chain_config, block_number, vm_type);

        db.cache_accounts(block).await?;

        Ok(db)
    }

    /// Pre-cache all accounts touched by the block to minimize RPC calls during execution.
    ///
    /// This method extracts and fetches:
    /// 1. Transaction senders (from addresses)
    /// 2. Transaction recipients (to addresses, excluding contract creations)
    /// 3. Storage slots from access lists
    ///
    /// All these accounts are pre-fetched and stored in the cache
    /// This is done to batch request for multiple storage slots in a single RPC call.
    async fn cache_accounts(&mut self, block: &Block) -> eyre::Result<()> {
        let txs = &block.body.transactions;

        let callers = txs.iter().filter_map(|tx| tx.sender().ok());
        let to = txs.iter().filter_map(|tx| match tx.to() {
            TxKind::Call(to) => Some(to),
            TxKind::Create => None,
        });
        let accessed_storage: Vec<_> = txs.iter().flat_map(|tx| tx.access_list().clone()).collect();

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

    /// Fetches account data from the RPC endpoint and updates the appropriate cache.
    ///
    /// This method retrieves account information including state, code, storage values, and proofs
    /// for the specified addresses and storage keys using the `eth_getProof` RPC method.
    ///
    /// # Parameters
    /// * `index` - List of addresses and their storage keys to fetch
    /// * `from_child` - If true, fetches data for the post-state (block_number + 1),
    ///   otherwise fetches data for the pre-state (block_number)
    ///
    /// # Implementation details
    /// * Uses rate limiting to avoid surpassing the RPC endpoint limits
    /// * Merges new data with existing cached data
    /// * Updates code cache with the bytecode
    async fn fetch_accounts(
        &self,
        index: &[(Address, Vec<H256>)],
        from_child: bool,
    ) -> eyre::Result<HashMap<Address, Account>> {
        let block_number = if from_child {
            self.block_number + 1
        } else {
            self.block_number
        };

        let mut fetched = HashMap::new();
        let mut counter = 0;

        // Fetch accounts in chunks to respect rate limits of the RPC endpoint
        for chunk in index.chunks(RPC_RATE_LIMIT) {
            let start = Instant::now();

            // Call to `eth_getProof` for each account in the chunk
            let futures = chunk.iter().map(|(address, storage_keys)| async move {
                Ok((
                    *address,
                    retry(|| {
                        get_account(
                            &self.rpc_url,
                            block_number,
                            address,
                            storage_keys,
                            &self.codes,
                        )
                    })
                    .await?,
                ))
            });

            let fetched_chunk = join_all(futures)
                .await
                .into_iter()
                .collect::<eyre::Result<HashMap<_, _>>>()?;

            fetched.extend(fetched_chunk);

            if index.len() == 1 {
                let address = chunk.first().unwrap().0;
                debug!("fetched account {address}");
            } else {
                counter += chunk.len();
                debug!("fetched {} accounts of {}", counter, index.len());
            }

            // Cooldown depends on chunk size.
            let chunk_size = chunk.len() as u32;
            let target_gap = RATE_LIMIT * chunk_size;
            let elapsed = start.elapsed();

            if target_gap > elapsed {
                sleep(target_gap - elapsed).await;
            }
        }

        // Merge fetched accounts into the appropriate cache based on the `from_child` flag.
        // If from_child is true, we update the post-state cache (child_cache).
        // Otherwise, we update the pre-state cache (cache).
        // For existing cache entries, we merge storage and proof data.
        if from_child {
            let mut child_cache = self.child_cache.lock().unwrap();
            for (address, account) in &fetched {
                let acc_account_mut = child_cache.get_mut(address);
                if let Some(cached_account) = acc_account_mut {
                    // If already in cache, merge storage and proofs
                    match (account, cached_account) {
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
                if let Some(cached_account) = acc_account_mut {
                    // If already in cache, merge storage and proofs
                    match (account, cached_account) {
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
        // Update code cache with any newly fetched code and hash it.
        {
            let mut codes = self.codes.lock().unwrap();
            for account in fetched.values() {
                if let Account::Existing {
                    code: Some(code), ..
                } = account
                {
                    codes.insert(code_hash(code), code.clone());
                }
            }
        }

        Ok(fetched)
    }

    /// Blocking version of fetch_accounts to be used inside LevmDatabase trait methods.
    fn fetch_accounts_blocking(
        &self,
        index: &[(Address, Vec<H256>)],
        from_child: bool,
    ) -> eyre::Result<HashMap<Address, Account>> {
        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| handle.block_on(self.fetch_accounts(index, from_child)))
    }

    /// Creates an execution witness for the given block from the current database state.
    ///
    /// This method:
    /// 1. Pre-executes the block to capture all state changes
    /// 2. Gathers account and storage proofs for both initial and final states
    /// 3. Collects potential child nodes for deleted account and storage entries
    pub fn to_execution_witness(&self, block: &Block) -> eyre::Result<RpcExecutionWitness> {
        let mut db = GeneralizedDatabase::new(Arc::new(self.clone()));

        // pre-execute and get all state changes
        let _ = LEVM::execute_block(block, &mut db, self.vm_type).map_err(Box::new)?;
        let execution_updates = LEVM::get_state_transitions(&mut db).map_err(Box::new)?;

        info!(
            "Finished pre-executing block {}. Now gathering execution witness.",
            block.header.number
        );

        let index: Vec<(Address, Vec<H256>)> = self
            .cache
            .lock()
            .unwrap()
            .iter()
            .map(|(address, account)| match account {
                Account::Existing { storage, .. } => (*address, storage.keys().cloned().collect()),
                Account::NonExisting { .. } => {
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
        let initial_accounts = self
            .fetch_accounts_blocking(&index, false)
            .wrap_err("failed to fetch initial accounts")?;
        let final_accounts = self
            .fetch_accounts_blocking(&index, true)
            .wrap_err("failed to fetch final accounts")?;

        let initial_account_proofs = initial_accounts
            .values()
            .map(|account| account.get_account_proof());
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

        let potential_storage_child_nodes: Vec<Vec<u8>> = final_storage_proofs
            .flat_map(|(_, proofs)| {
                proofs
                    .iter()
                    .filter_map(|(key, proof)| get_potential_child_nodes(proof, &hash_key(key)))
                    .flat_map(|nodes| nodes.into_iter().map(|node| node.encode_raw()))
                    .collect::<Vec<_>>()
            })
            .collect();

        #[derive(Clone)]
        struct ExistingAccount<'a> {
            pub storage: &'a HashMap<H256, U256>,
            pub code: &'a Option<Bytes>,
        }

        let existing_accs = initial_accounts.iter().filter_map(|(address, account)| {
            if let Account::Existing { storage, code, .. } = account {
                Some((address, ExistingAccount { storage, code }))
            } else {
                None
            }
        });
        let codes: Vec<Bytes> = existing_accs
            .clone()
            .map(|(_, account)| Bytes::from(account.code.clone().unwrap_or_default().to_vec()))
            .collect();
        let keys: Vec<Bytes> = existing_accs
            .clone()
            .flat_map(|(address, account)| {
                std::iter::once(Bytes::from(address.as_bytes().to_vec())).chain(
                    account
                        .storage
                        .keys()
                        .map(|value| Bytes::from(value.as_bytes().to_vec())),
                )
            })
            .collect();

        let mut block_headers_bytes = Vec::new();
        let oldest_required_block_number = self
            .block_hashes
            .lock()
            .unwrap()
            .keys()
            .min()
            .cloned()
            .unwrap_or(block.header.number - 1);
        for number in oldest_required_block_number..block.header.number {
            let handle = tokio::runtime::Handle::current();
            let number_usize: usize = number
                .try_into()
                .wrap_err("failed to convert block number into usize")?;
            let header = tokio::task::block_in_place(|| {
                handle.block_on(get_block(&self.rpc_url, number_usize, false))
            })
            .wrap_err("failed to fetch block header")?
            .header;
            block_headers_bytes.push(Bytes::from(header.encode_to_vec()));
        }

        let state_root = initial_account_proofs
            .clone()
            .next()
            .and_then(|proof| proof.first().cloned());
        let other_state_nodes: Vec<Vec<u8>> = initial_account_proofs
            .flat_map(|proof| proof.iter().skip(1).cloned())
            .chain(potential_account_child_nodes)
            .collect();
        let state_proofs = (state_root, other_state_nodes);

        let mut all_nodes = Vec::new();

        if let Some(root) = &state_proofs.0 {
            all_nodes.push(Bytes::from(root.clone()));
        }

        all_nodes.extend(state_proofs.1.clone().into_iter().map(Bytes::from));

        for (_, proofs) in initial_storage_proofs {
            if let Some(root) = proofs.iter().next().and_then(|(_, nodes)| nodes.first()) {
                all_nodes.push(Bytes::from(root.clone()));
            }

            for proof in proofs.values() {
                all_nodes.extend(proof.iter().skip(1).cloned().map(Bytes::from));
            }
        }
        all_nodes.extend(potential_storage_child_nodes.into_iter().map(Bytes::from));

        Ok(RpcExecutionWitness {
            state: all_nodes,
            keys,
            codes,
            headers: block_headers_bytes,
        })
    }
}

impl LevmDatabase for RpcDB {
    fn get_account_code(&self, code_hash: H256) -> Result<Bytes, DatabaseError> {
        if code_hash == *EMPTY_KECCACK_HASH {
            return Ok(Bytes::new());
        }
        let codes = self.codes.lock().unwrap();
        codes.get(&code_hash).cloned().ok_or_else(|| {
            DatabaseError::Custom("Code not found on already fetched accounts".to_string())
        })
    }

    fn get_account_info(&self, address: Address) -> Result<AccountInfo, DatabaseError> {
        let cache = self.cache.lock().unwrap();
        let account = if let Some(account) = cache.get(&address).cloned() {
            account
        } else {
            drop(cache);
            let account = self
                .fetch_accounts_blocking(&[(address, vec![])], false)
                .map_err(|e| DatabaseError::Custom(format!("Failed to fetch account info: {e}")))?
                .get(&address)
                .cloned();
            if let Some(account) = account.clone() {
                self.cache.lock().unwrap().insert(address, account.clone());
                account
            } else {
                // If the RPC did not return the account, consider it non-existing
                return Ok(AccountInfo::default());
            }
        };
        if let Account::Existing {
            account_state,
            code,
            ..
        } = account
        {
            if let Some(code) = code {
                let mut codes = self.codes.lock().unwrap();
                codes
                    .entry(code_hash(&code))
                    .or_insert_with(|| code.clone());
            }
            Ok(AccountInfo {
                code_hash: account_state.code_hash,
                balance: account_state.balance,
                nonce: account_state.nonce,
            })
        } else {
            Ok(AccountInfo::default())
        }
    }

    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError> {
        // look into the cache
        {
            if let Some(Account::Existing { storage, .. }) =
                self.cache.lock().unwrap().get(&address)
            {
                if let Some(value) = storage.get(&key) {
                    return Ok(*value);
                }
            }
        }
        let account = self
            .fetch_accounts_blocking(&[(address, vec![key])], false)
            .map_err(|e| DatabaseError::Custom(format!("Failed to fetch account info: {e}")))?
            .get(&address)
            .cloned();
        if let Some(Account::Existing { storage, .. }) = account {
            if let Some(value) = storage.get(&key) {
                return Ok(*value);
            }
        }
        Ok(U256::zero())
    }

    fn get_block_hash(&self, block_number: u64) -> Result<H256, DatabaseError> {
        if let Some(hash) = self.block_hashes.lock().unwrap().get(&block_number) {
            return Ok(*hash);
        }
        let handle = tokio::runtime::Handle::current();
        let hash = tokio::task::block_in_place(|| {
            handle.block_on(retry(|| {
                get_block(&self.rpc_url, block_number as usize, false)
            }))
        })
        .map_err(|e| DatabaseError::Custom(e.to_string()))?
        .hash;
        self.block_hashes.lock().unwrap().insert(block_number, hash);
        Ok(hash)
    }

    fn get_chain_config(&self) -> Result<ethrex_common::types::ChainConfig, DatabaseError> {
        Ok(self.chain_config)
    }
}

/// Get all potential child nodes of a node whose value was deleted.
///
/// After deleting a value from a (partial) trie it's possible that the node containing the value gets
/// replaced by its child, whose prefix is possibly modified by appending some nibbles to it.
/// If we don't have this child node (because we're modifying a partial trie), then we can't
/// perform the deletion. If we have the final proof of exclusion of the deleted value, we can
/// calculate all possible child nodes.
pub fn get_potential_child_nodes(proof: &[NodeRLP], key: &PathRLP) -> Option<Vec<Node>> {
    // TODO: Perhaps it's possible to calculate the child nodes instead of storing all possible ones?.
    // TODO: https://github.com/lambdaclass/ethrex/issues/2938

    let mut state_nodes = BTreeMap::new();
    for node in proof.iter().skip(1) {
        let hash = Keccak256::digest(node);
        state_nodes.insert(H256::from_slice(&hash), node.clone());
    }

    let hash = if let Some(root) = proof.first() {
        let hash = H256::from_slice(&Keccak256::digest(root));
        state_nodes.insert(hash, root.clone());
        hash
    } else {
        *EMPTY_KECCACK_HASH
    };
    let trie = Trie::from_nodes(hash, &state_nodes).ok()?;

    // return some only if this is a proof of exclusion
    if trie.get(key).ok()?.is_none() {
        let final_node = Node::decode_raw(proof.last()?).ok()?;
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
