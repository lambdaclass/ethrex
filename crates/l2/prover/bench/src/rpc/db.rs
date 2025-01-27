use std::collections::hash_map::Entry;
use std::collections::HashMap;

use crate::constants::{CANCUN_CONFIG, RPC_RATE_LIMIT};
use crate::rpc::{get_account, get_block, get_storage, retry};

use ethrex_core::types::AccountInfo;
use ethrex_core::{
    types::{Account as CoreAccount, Block, TxKind},
    Address, H256,
};
use ethrex_vm::execution_db::{ExecutionDB, ToExecDB};
use ethrex_vm::spec_id;
use futures_util::future::join_all;
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
    pub accounts: HashMap<Address, Account>,
    pub block_hashes: HashMap<u64, H256>,
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
            block_hashes: HashMap::new(),
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

        let account = match self.accounts.get(&address) {
            Some(account) => account.clone(),
            None => {
                println!("retrieving account info for address {address}");
                let handle = tokio::runtime::Handle::current();
                tokio::task::block_in_place(|| {
                    handle.block_on(get_account(&self.rpc_url, self.block_number, &address, &[]))
                })?
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
                    handle.block_on(get_storage(
                        &self.rpc_url,
                        self.block_number,
                        &address,
                        index,
                    ))
                })?
            }
        };

        Ok(RevmU256::from_limbs(value.0))
    }
    fn block_hash_ref(&self, number: u64) -> Result<RevmB256, Self::Error> {
        println!("retrieving block hash for block number {number}");
        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| handle.block_on(get_block(&self.rpc_url, number as usize)))
            .map(|block| RevmB256::from(block.hash().0))
    }
}

impl ToExecDB for RpcDB {
    fn to_exec_db(
        &self,
        block: &Block,
    ) -> Result<ethrex_vm::execution_db::ExecutionDB, ethrex_vm::errors::ExecutionDBError> {
        let parent_hash = block.header.parent_hash;
        let chain_config = CANCUN_CONFIG;

        // pre-execute and get all state changes
        let cache = ExecutionDB::pre_execute(
            block,
            chain_config.chain_id,
            spec_id(&chain_config, block.header.timestamp),
            self,
        )
        .unwrap(); // TODO: fix evm, executiondb errors to remove this unwrap

        // fetch all read/written values from store
        let already_existing_accounts: Vec<_> = cache
            .accounts
            .iter()
            // filter out new accounts, we're only interested in already existing accounts.
            // new accounts are storage cleared, self-destructed accounts too but they're marked with "not
            // existing" status instead.
            .filter_map(|(address, account)| {
                if !account.account_state.is_storage_cleared() {
                    Some((
                        Address::from(address.0.as_ref()),
                        account
                            .storage
                            .keys()
                            .map(|key| H256::from_slice(&key.to_be_bytes_vec()))
                            .collect::<Vec<_>>(),
                    ))
                } else {
                    None
                }
            })
            .collect();

        let handle = tokio::runtime::Handle::current();
        let rpc_accounts = tokio::task::block_in_place(|| {
            handle.block_on(self.fetch_accounts(&already_existing_accounts))
        })
        .unwrap(); // TODO: remove unwrap

        let accounts = rpc_accounts
            .iter()
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
        let code = rpc_accounts
            .values()
            .map(|account| {
                (
                    account.account_state.code_hash,
                    account.code.clone().unwrap_or_default(),
                )
            })
            .collect();
        let storage = rpc_accounts
            .iter()
            .map(|(address, account)| (*address, account.storage.clone()))
            .collect();
        let block_hashes = self
            .block_hashes
            .iter()
            .map(|(num, hash)| (*num, *hash))
            .collect();
        // WARN: unwrapping because revm wraps a u64 as a U256

        let state_root = rpc_accounts
            .values()
            .next()
            .and_then(|account| account.account_proof.first().cloned());
        let other_state_nodes = rpc_accounts
            .values()
            .flat_map(|(account)| account.account_proof.clone())
            .collect();
        let state_proofs = (state_root, other_state_nodes);

        let storage_proofs = rpc_accounts
            .iter()
            .map(|(address, account)| {
                let proofs: Vec<NodeRLP> =
                    account.storage_proofs.iter().flatten().cloned().collect();
                (*address, (proofs.first().cloned(), proofs))
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
