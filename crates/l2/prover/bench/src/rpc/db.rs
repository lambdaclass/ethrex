use std::collections::HashMap;

use crate::constants::RPC_RATE_LIMIT;
use crate::rpc::{get_account, get_block, get_storage};

use ethrex_core::{types::{Block, TxKind}, Address, H256};
use revm::DatabaseRef;
use revm_primitives::{
    AccountInfo as RevmAccountInfo, Address as RevmAddress, Bytecode as RevmBytecode,
    Bytes as RevmBytes, B256 as RevmB256, U256 as RevmU256,
};
use tokio_utils::RateLimiter;
use futures_util::future::join_all;

use super::Account;

pub struct RpcDB {
    pub rpc_url: String,
    pub block_number: usize,
    // we concurrently download tx callers before pre-execution to minimize sequential RPC calls
    pub accounts: HashMap<Address, Account>,
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
            accounts: HashMap::new(),
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

        let accounts: Vec<_> = callers
            .chain(to)
            .map(|address| (address, Vec::new()))
            .chain(accessed_storage)
            .collect();
        self.accounts = self.fetch_accounts(&accounts).await?;

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
                    get_account(
                        &self.rpc_url,
                        self.block_number,
                        address,
                        storage_keys,
                    )
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

        let account = match self.accounts.get(&address) {
            Some(account) => account.clone(),
            None => {
                println!("retrieving account info for address {address}");
                let handle = tokio::runtime::Handle::current();
                tokio::task::block_in_place(|| {
                    handle.block_on(
                    get_account(&self.rpc_url, self.block_number, &address, &[])
                    )})?
            }
        };

        Ok(Some(RevmAccountInfo {
            nonce: account.account_state.nonce,
            balance: RevmU256::from_limbs(account.account_state.balance.0),
            code_hash: RevmB256::from(account.account_state.code_hash.0),
            code: account
                .code
                .map(|code| RevmBytecode::new_raw(RevmBytes(code))),
        }))
    }
    #[allow(unused_variables)]
    fn code_by_hash_ref(&self, code_hash: RevmB256) -> Result<RevmBytecode, Self::Error> {
        Ok(RevmBytecode::default()) // code is stored in account info
    }
    fn storage_ref(&self, address: RevmAddress, index: RevmU256) -> Result<RevmU256, Self::Error> {
        let address = Address::from(address.0.as_ref());
        let index = H256::from_slice(&index.to_be_bytes_vec());

        let value = match self
            .accounts
            .get(&address)
            .and_then(|account| account.storage.get(&index))
        {
            Some(value) => *value,
            None => {
                println!("retrieving storage value for address {address} and key {index}");
                let handle = tokio::runtime::Handle::current();
                tokio::task::block_in_place(|| {
                    handle.block_on(
                        get_storage(&self.rpc_url, self.block_number, &address, index)
                    )})?
            }
        };

        Ok(RevmU256::from_limbs(value.0))
    }
    fn block_hash_ref(&self, number: u64) -> Result<RevmB256, Self::Error> {
        println!("retrieving block hash for block number {number}");
                let handle = tokio::runtime::Handle::current();
                tokio::task::block_in_place(|| {
                    handle.block_on(
        get_block(&self.rpc_url, number as usize)
                    )})
            .map(|block| RevmB256::from(block.hash().0))
    }
}

// impl ToExecDB for RpcDB {
//     fn to_exec_db(
//         &self,
//         block: &Block,
//     ) -> Result<ethrex_vm::execution_db::ExecutionDB, ethrex_vm::errors::ExecutionDBError> {
//         let parent_hash = block.header.parent_hash;
//         let chain_config = CANCUN_CONFIG;
//
//         // pre-execute and get all state changes
//         let cache = ExecutionDB::pre_execute(
//             block,
//             chain_config.chain_id,
//             spec_id(&chain_config, block.header.timestamp),
//             self,
//         )
//         .unwrap(); // TODO: fix evm, executiondb errors to remove this unwrap
//         let store_wrapper = cache.db;
//
//         // fetch all read/written values from store
//         let already_existing_accounts = cache
//             .accounts
//             .iter()
//             // filter out new accounts, we're only interested in already existing accounts.
//             // new accounts are storage cleared, self-destructed accounts too but they're marked with "not
//             // existing" status instead.
//             .filter_map(|(address, account)| {
//                 if !account.account_state.is_storage_cleared() {
//                     Some((Address::from(address.0.as_ref()), account))
//                 } else {
//                     None
//                 }
//             });
//
//         let accounts = already_existing_accounts
//             .clone()
//             .map(|(address, _)| {
//                 // return error if account is missing
//                 let account = match store_wrapper
//                     .store
//                     .get_account_info_by_hash(parent_hash, address)
//                 {
//                     Ok(None) => Err(ExecutionDBError::NewMissingAccountInfo(address)),
//                     Ok(Some(some)) => Ok(some),
//                     Err(err) => Err(ExecutionDBError::Store(err)),
//                 };
//                 Ok((address, account?))
//             })
//             .collect::<Result<HashMap<_, _>, ExecutionDBError>>()?;
//         let code = already_existing_accounts
//             .clone()
//             .map(|(_, account)| {
//                 // return error if code is missing
//                 let hash = H256::from(account.info.code_hash.0);
//                 Ok((
//                     hash,
//                     store_wrapper
//                         .store
//                         .get_account_code(hash)?
//                         .ok_or(ExecutionDBError::NewMissingCode(hash))?,
//                 ))
//             })
//             .collect::<Result<_, ExecutionDBError>>()?;
//         let storage = already_existing_accounts
//             .map(|(address, account)| {
//                 // return error if storage is missing
//                 Ok((
//                     address,
//                     account
//                         .storage
//                         .keys()
//                         .map(|key| {
//                             let key = H256::from(key.to_be_bytes());
//                             let value = store_wrapper
//                                 .store
//                                 .get_storage_at_hash(parent_hash, address, key)
//                                 .map_err(ExecutionDBError::Store)?
//                                 .ok_or(ExecutionDBError::NewMissingStorage(address, key))?;
//                             Ok((key, value))
//                         })
//                         .collect::<Result<HashMap<_, _>, ExecutionDBError>>()?,
//                 ))
//             })
//             .collect::<Result<HashMap<_, _>, ExecutionDBError>>()?;
//         let block_hashes = cache
//             .block_hashes
//             .into_iter()
//             .map(|(num, hash)| (num.try_into().unwrap(), H256::from(hash.0)))
//             .collect();
//         // WARN: unwrapping because revm wraps a u64 as a U256
//
//         // get proofs
//         let state_trie = self
//             .store
//             .state_trie(parent_hash)?
//             .ok_or(ExecutionDBError::NewMissingStateTrie(parent_hash))?;
//
//         let state_proofs =
//             state_trie.get_proofs(&accounts.keys().map(hash_address).collect::<Vec<_>>())?;
//
//         let mut storage_proofs = HashMap::new();
//         for (address, storages) in &storage {
//             let storage_trie = self.store.storage_trie(parent_hash, *address)?.ok_or(
//                 ExecutionDBError::NewMissingStorageTrie(parent_hash, *address),
//             )?;
//
//             let paths = storages.keys().map(hash_key).collect::<Vec<_>>();
//             storage_proofs.insert(*address, storage_trie.get_proofs(&paths)?);
//         }
//
//         Ok(ExecutionDB {
//             accounts,
//             code,
//             storage,
//             block_hashes,
//             chain_config,
//             state_proofs,
//             storage_proofs,
//         })
//     }
// }
//
// fn download_accounts(block_number: u64) {
//
//         let rate_limiter = RateLimiter::new(std::time::Duration::from_secs(1));
//         let mut fetched_accs = 0;
//         for request_chunk in already_existing_accounts.chunks(RPC_RATE_LIMIT) {
//             let account_futures = request_chunk.iter().map(|(address, account)| async {
//                 Ok((
//                     *address,
//                     get_account(
//                         &self.rpc_url,
//                         block_number - 1,
//                         &address.clone(),
//                         &storage_keys.clone(),
//                     )
//                     .await?,
//                 ))
//             });
//
//             let fetched_accounts = rate_limiter
//                 .throttle(|| async { join_all(account_futures).await })
//                 .await
//                 .into_iter()
//                 .collect::<Result<Vec<_>, String>>()
//                 .expect("failed to fetch accounts");
//             for (
//                 address,
//                 Account {
//                     account_state,
//                     storage,
//                     account_proof,
//                     storage_proofs,
//                     code,
//                 },
//             ) in fetched_accounts
//             {
//                 accounts.insert(address.to_owned(), account_state);
//                 storages.insert(address.to_owned(), storage);
//                 if let Some(code) = code {
//                     codes.push(code);
//                 }
//                 account_proofs.extend(account_proof);
//                 storages_proofs
//                     .entry(address)
//                     .or_default()
//                     .extend(storage_proofs.into_iter().flatten());
//             }
//
//             fetched_accs += request_chunk.len();
//             println!(
//                 "fetched {} accounts of {}",
//                 fetched_accs,
//                 touched_state.len()
//             );
//         }
//
// }
