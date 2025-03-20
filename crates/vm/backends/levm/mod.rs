pub(crate) mod db;

use super::BlockExecutionResult;
use crate::constants::{
    BEACON_ROOTS_ADDRESS, CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS, HISTORY_STORAGE_ADDRESS,
    SYSTEM_ADDRESS, WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
};
use crate::db::{get_potential_child_nodes, ExecutionDB, StoreWrapper};
use crate::errors::ExecutionDBError;
use crate::execution_result::ExecutionResult;
use crate::EvmError;
use bytes::Bytes;
use ethrex_common::types::requests::Requests;
use ethrex_common::types::{AuthorizationTuple, Fork, GenericTransaction, INITIAL_BASE_FEE};
use ethrex_common::{
    types::{
        code_hash, AccountInfo, Block, BlockHeader, ChainConfig, Receipt, Transaction, TxKind,
        Withdrawal, GWEI_TO_WEI,
    },
    Address, H256, U256,
};
use ethrex_levm::{
    db::Database as LevmDatabase,
    errors::{ExecutionReport, TxResult, VMError},
    vm::{EVMConfig, VM},
    Account, AccountInfo as LevmAccountInfo, Environment,
};
use ethrex_storage::{hash_address, hash_key, AccountUpdate, Store};
use ethrex_trie::{NodeRLP, TrieError};
use std::cmp::min;
use std::{collections::HashMap, sync::Arc};

// Export needed types
pub use ethrex_levm::db::CacheDB;
/// The struct implements the following functions:
/// [LEVM::execute_block]
/// [LEVM::execute_tx]
/// [LEVM::get_state_transitions]
/// [LEVM::process_withdrawals]
#[derive(Debug)]
pub struct LEVM;

impl LEVM {
    pub fn execute_block(
        block: &Block,
        db: Arc<dyn LevmDatabase>,
    ) -> Result<BlockExecutionResult, EvmError> {
        let mut block_cache: CacheDB = HashMap::new();
        let config = db.get_chain_config();
        cfg_if::cfg_if! {
            if #[cfg(not(feature = "l2"))] {
                let block_header = &block.header;
                let fork = config.fork(block_header.timestamp);
                if block_header.parent_beacon_block_root.is_some() && fork >= Fork::Cancun {
                    Self::beacon_root_contract_call(block_header, db.clone(), &mut block_cache)?;
                }

                if fork >= Fork::Prague {
                    //eip 2935: stores parent block hash in system contract
                    Self::process_block_hash_history(block_header, db.clone(), &mut block_cache)?;
                }
            }
        }

        // Account updates are initialized like this because of the beacon_root_contract_call, it is going to be empty if it wasn't called.
        // Here we get the state_transitions from the db and then we get the state_transitions from the cache_db.
        let mut account_updates =
            Self::get_state_transitions(None, db.clone(), &block.header, &block_cache)?;
        let mut receipts = Vec::new();
        let mut cumulative_gas_used = 0;

        for tx in block.body.transactions.iter() {
            let report =
                Self::execute_tx(tx, &block.header, db.clone(), block_cache.clone(), &config)
                    .map_err(EvmError::from)?;

            let mut new_state = report.new_state.clone();
            // Now original_value is going to be the same as the current_value, for the next transaction.
            // It should have only one value but it is convenient to keep on using our CacheDB structure
            for account in new_state.values_mut() {
                for storage_slot in account.storage.values_mut() {
                    storage_slot.original_value = storage_slot.current_value;
                }
            }

            block_cache.extend(new_state);

            // Currently, in LEVM, we don't substract refunded gas to used gas, but that can change in the future.
            let gas_used = report.gas_used - report.gas_refunded;
            cumulative_gas_used += gas_used;
            let receipt = Receipt::new(
                tx.tx_type(),
                matches!(report.result.clone(), TxResult::Success),
                cumulative_gas_used,
                report.logs.clone(),
            );

            receipts.push(receipt);
        }

        // Here we update block_cache with balance increments caused by withdrawals.
        if let Some(withdrawals) = &block.body.withdrawals {
            // For every withdrawal we increment the target account's balance
            for (address, increment) in withdrawals
                .iter()
                .filter(|withdrawal| withdrawal.amount > 0)
                .map(|w| (w.address, u128::from(w.amount) * u128::from(GWEI_TO_WEI)))
            {
                // We check if it was in block_cache, if not, we get it from DB.
                let mut account = block_cache.get(&address).cloned().unwrap_or({
                    let acc_info = db.get_account_info(address);
                    Account::from(acc_info)
                });

                account.info.balance += increment.into();

                block_cache.insert(address, account);
            }
        }

        let requests =
            extract_all_requests_levm(&receipts, db.clone(), &block.header, &mut block_cache)?;

        account_updates.extend(Self::get_state_transitions(
            None,
            db.clone(),
            &block.header,
            &block_cache,
        )?);

        Ok(BlockExecutionResult {
            receipts,
            requests,
            account_updates,
        })
    }

    pub fn execute_tx(
        // The transaction to execute.
        tx: &Transaction,
        // The block header for the current block.
        block_header: &BlockHeader,
        // The database to use for EVM state access.  This is wrapped in an `Arc` for shared ownership.
        db: Arc<dyn LevmDatabase>,
        // A cache database for intermediate state changes during execution.
        block_cache: CacheDB,
        // The EVM configuration to use.
        chain_config: &ChainConfig,
    ) -> Result<ExecutionReport, EvmError> {
        let gas_price: U256 = tx
            .effective_gas_price(block_header.base_fee_per_gas)
            .ok_or(VMError::InvalidTransaction)?
            .into();

        let config = EVMConfig::new_from_chain_config(chain_config, block_header);
        let env = Environment {
            origin: tx.sender(),
            refunded_gas: 0,
            gas_limit: tx.gas_limit(),
            config,
            block_number: block_header.number.into(),
            coinbase: block_header.coinbase,
            timestamp: block_header.timestamp.into(),
            prev_randao: Some(block_header.prev_randao),
            chain_id: tx.chain_id().unwrap_or_default().into(),
            base_fee_per_gas: block_header.base_fee_per_gas.unwrap_or_default().into(),
            gas_price,
            block_excess_blob_gas: block_header.excess_blob_gas.map(U256::from),
            block_blob_gas_used: block_header.blob_gas_used.map(U256::from),
            tx_blob_hashes: tx.blob_versioned_hashes(),
            tx_max_priority_fee_per_gas: tx.max_priority_fee().map(U256::from),
            tx_max_fee_per_gas: tx.max_fee_per_gas().map(U256::from),
            tx_max_fee_per_blob_gas: tx.max_fee_per_blob_gas().map(U256::from),
            tx_nonce: tx.nonce(),
            block_gas_limit: block_header.gas_limit,
            transient_storage: HashMap::new(),
            difficulty: block_header.difficulty,
        };

        let mut vm = VM::new(
            tx.to(),
            env,
            tx.value(),
            tx.data().clone(),
            db.clone(),
            block_cache.clone(),
            tx.access_list(),
            tx.authorization_list(),
        )?;

        vm.execute().map_err(VMError::into)
    }
    pub fn simulate_tx_from_generic(
        // The transaction to execute.
        tx: &GenericTransaction,
        // The block header for the current block.
        block_header: &BlockHeader,
        // The database to use for EVM state access.  This is wrapped in an `Arc` for shared ownership.
        db: Arc<dyn LevmDatabase>,
        // A cache database for intermediate state changes during execution.
        block_cache: CacheDB,
        // The EVM configuration to use.
        chain_config: &ChainConfig,
    ) -> Result<ExecutionResult, EvmError> {
        let gas_price: U256 = calculate_gas_price(
            tx,
            block_header.base_fee_per_gas.unwrap_or(INITIAL_BASE_FEE),
        );

        let config = EVMConfig::new_from_chain_config(chain_config, block_header);
        let mut env = Environment {
            origin: tx.from.0.into(),
            refunded_gas: 0,
            gas_limit: tx.gas.unwrap_or(u64::MAX), // Ensure tx doesn't fail due to gas limit
            config,
            block_number: block_header.number.into(),
            coinbase: block_header.coinbase,
            timestamp: block_header.timestamp.into(),
            prev_randao: Some(block_header.prev_randao),
            chain_id: tx.chain_id.unwrap_or(chain_config.chain_id).into(),
            base_fee_per_gas: block_header.base_fee_per_gas.unwrap_or_default().into(),
            gas_price,
            block_excess_blob_gas: block_header.excess_blob_gas.map(U256::from),
            block_blob_gas_used: block_header.blob_gas_used.map(U256::from),
            tx_blob_hashes: tx.blob_versioned_hashes.clone(),
            tx_max_priority_fee_per_gas: tx.max_priority_fee_per_gas.map(U256::from),
            tx_max_fee_per_gas: tx.max_fee_per_gas.map(U256::from),
            tx_max_fee_per_blob_gas: tx.max_fee_per_blob_gas,
            tx_nonce: tx.nonce.unwrap_or_default(),
            block_gas_limit: u64::MAX, // disable block gas limit
            transient_storage: HashMap::new(),
            difficulty: block_header.difficulty,
        };

        adjust_disabled_base_fee(&mut env);

        let mut vm = VM::new(
            tx.to.clone(),
            env,
            tx.value,
            tx.input.clone(),
            db,
            block_cache.clone(),
            tx.access_list
                .iter()
                .map(|list| (list.address, list.storage_keys.clone()))
                .collect(),
            tx.authorization_list.clone().map(|list| {
                list.iter()
                    .map(|list| Into::<AuthorizationTuple>::into(list.clone()))
                    .collect()
            }),
        )?;

        vm.execute()
            .map(|value| value.into())
            .map_err(VMError::into)
    }

    pub fn get_state_transitions(
        // Warning only pass the fork if running the ef-tests.
        // ISSUE #2021: https://github.com/lambdaclass/ethrex/issues/2021
        ef_tests: Option<Fork>,
        db: Arc<dyn LevmDatabase>,
        block_header: &BlockHeader,
        new_state: &CacheDB,
    ) -> Result<Vec<AccountUpdate>, EvmError> {
        let mut account_updates: Vec<AccountUpdate> = vec![];
        for (new_state_account_address, new_state_account) in new_state {
            let initial_account_state = db.get_account_info(*new_state_account_address);
            let mut updates = 0;
            if initial_account_state.balance != new_state_account.info.balance {
                updates += 1;
            }
            if initial_account_state.nonce != new_state_account.info.nonce {
                updates += 1;
            }
            let code = if new_state_account.info.bytecode.is_empty() {
                // The new state account has no code
                None
            } else {
                // Look into the current database to see if the bytecode hash is already present
                let current_bytecode = db.get_account_info(*new_state_account_address).bytecode;
                let code = new_state_account.info.bytecode.clone();
                // The code is present in the current database
                if current_bytecode != Bytes::new() {
                    if current_bytecode != code {
                        // The code has changed
                        Some(code)
                    } else {
                        // The code has not changed
                        None
                    }
                } else {
                    // The new state account code is not present in the current
                    // database, then it must be new
                    Some(code)
                }
            };
            if code.is_some() {
                updates += 1;
            }
            let mut added_storage = HashMap::new();
            for (key, value) in &new_state_account.storage {
                added_storage.insert(*key, value.current_value);
                updates += 1;
            }

            if updates == 0 && !new_state_account.is_empty() {
                continue;
            }

            let account_update = AccountUpdate {
                address: *new_state_account_address,
                removed: new_state_account.is_empty(),
                info: Some(AccountInfo {
                    code_hash: code_hash(&new_state_account.info.bytecode),
                    balance: new_state_account.info.balance,
                    nonce: new_state_account.info.nonce,
                }),
                code,
                added_storage,
            };

            let fork_from_config = db.get_chain_config().fork(block_header.timestamp);
            // Here we take the passed fork through the ef_tests variable, or we set it to the fork based on the timestamp.
            let fork = ef_tests.unwrap_or(fork_from_config);
            let old_info = db.get_account_info(account_update.address);
            // https://eips.ethereum.org/EIPS/eip-161
            // if an account was empty and is now empty, after spurious dragon, it should be removed
            if account_update.removed
                && old_info.balance.is_zero()
                && old_info.nonce == 0
                && old_info.bytecode_hash() == code_hash(&Bytes::new())
                && fork < Fork::SpuriousDragon
            {
                continue;
            }

            account_updates.push(account_update);
        }
        Ok(account_updates)
    }

    pub fn process_withdrawals(
        block_cache: &mut CacheDB,
        withdrawals: &[Withdrawal],
        store: &Store,
        parent_hash: H256,
    ) -> Result<(), ethrex_storage::error::StoreError> {
        // For every withdrawal we increment the target account's balance
        for (address, increment) in withdrawals
            .iter()
            .filter(|withdrawal| withdrawal.amount > 0)
            .map(|w| (w.address, u128::from(w.amount) * u128::from(GWEI_TO_WEI)))
        {
            // We check if it was in block_cache, if not, we get it from DB.
            let mut account = block_cache.get(&address).cloned().unwrap_or({
                let acc_info = store
                    .get_account_info_by_hash(parent_hash, address)?
                    .unwrap_or_default();
                let acc_code = store
                    .get_account_code(acc_info.code_hash)?
                    .unwrap_or_default();

                Account {
                    info: LevmAccountInfo {
                        balance: acc_info.balance,
                        bytecode: acc_code,
                        nonce: acc_info.nonce,
                    },
                    // This is the added_storage for the withdrawal.
                    // If not involved in the TX, there won't be any updates in the storage
                    storage: HashMap::new(),
                }
            });

            account.info.balance += increment.into();
            block_cache.insert(address, account);
        }
        Ok(())
    }

    // SYSTEM CONTRACTS
    /// `new_state` is being modified inside [generic_system_contract_levm].
    pub fn beacon_root_contract_call(
        block_header: &BlockHeader,
        db: Arc<dyn LevmDatabase>,
        new_state: &mut CacheDB,
    ) -> Result<(), EvmError> {
        let beacon_root = match block_header.parent_beacon_block_root {
            None => {
                return Err(EvmError::Header(
                    "parent_beacon_block_root field is missing".to_string(),
                ))
            }
            Some(beacon_root) => beacon_root,
        };

        generic_system_contract_levm(
            block_header,
            Bytes::copy_from_slice(beacon_root.as_bytes()),
            db,
            new_state,
            *BEACON_ROOTS_ADDRESS,
            *SYSTEM_ADDRESS,
        )?;
        Ok(())
    }
    /// `new_state` is being modified inside [generic_system_contract_levm].
    pub fn process_block_hash_history(
        block_header: &BlockHeader,
        db: Arc<dyn LevmDatabase>,
        new_state: &mut CacheDB,
    ) -> Result<(), EvmError> {
        generic_system_contract_levm(
            block_header,
            Bytes::copy_from_slice(block_header.parent_hash.as_bytes()),
            db.clone(),
            new_state,
            *HISTORY_STORAGE_ADDRESS,
            *SYSTEM_ADDRESS,
        )?;
        Ok(())
    }
    pub(crate) fn read_withdrawal_requests(
        block_header: &BlockHeader,
        db: Arc<dyn LevmDatabase>,
        new_state: &mut CacheDB,
    ) -> Option<ExecutionReport> {
        let report = generic_system_contract_levm(
            block_header,
            Bytes::new(),
            db.clone(),
            new_state,
            *WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
            *SYSTEM_ADDRESS,
        )
        .ok()?;

        match report.result {
            TxResult::Success => Some(report),
            _ => None,
        }
    }
    pub(crate) fn dequeue_consolidation_requests(
        block_header: &BlockHeader,
        db: Arc<dyn LevmDatabase>,
        new_state: &mut CacheDB,
    ) -> Option<ExecutionReport> {
        let report = generic_system_contract_levm(
            block_header,
            Bytes::new(),
            db.clone(),
            new_state,
            *CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS,
            *SYSTEM_ADDRESS,
        )
        .ok()?;

        match report.result {
            TxResult::Success => Some(report),
            _ => None,
        }
    }

    pub fn to_exec_db(
        store_wrapper: &StoreWrapper,
        block: &Block,
    ) -> Result<ExecutionDB, ExecutionDBError> {
        // TODO: Simplify this function and potentially merge with the implementation for
        // RpcDB.

        let parent_hash = block.header.parent_hash;
        let chain_config = store_wrapper.store.get_chain_config()?;

        // pre-execute and get all state changes
        let (execution_updates, logger) = ExecutionDB::pre_execute_levm(block, store_wrapper)
            .map_err(|err| Box::new(EvmError::from(err)))?; // TODO: ugly error handling

        // index read and touched account addresses and storage keys
        let index = execution_updates.iter().map(|update| {
            // CHECK if we only need the touched storage keys
            let address = update.address;
            let storage_keys: Vec<_> = update
                .added_storage
                .keys()
                .map(|key| H256::from_slice(&key.to_fixed_bytes()))
                .collect();
            (address, storage_keys)
        });

        // fetch all read/written values from store
        let cache_accounts = execution_updates.iter().filter_map(|update| {
            let address = update.address;
            // filter new accounts (accounts that didn't exist before) assuming our store is
            // correct (based on the success of the pre-execution).
            if logger
                .db
                .store
                .get_account_info_by_hash(parent_hash, address)
                .is_ok_and(|account| account.is_some())
            {
                Some((address, update.info.clone()))
            } else {
                None
            }
        });
        let accounts = cache_accounts
            .clone()
            .map(|(address, _)| {
                // return error if account is missing
                let account = match logger
                    .db
                    .store
                    .get_account_info_by_hash(parent_hash, address)
                {
                    Ok(Some(some)) => Ok(some),
                    Err(err) => Err(ExecutionDBError::Store(err)),
                    Ok(None) => unreachable!(), // we are filtering out accounts that are not present
                                                // in the store
                };
                Ok((address, account?))
            })
            .collect::<Result<HashMap<_, _>, ExecutionDBError>>()?;
        let code = execution_updates
            .clone()
            .iter()
            .map(|update| {
                // return error if code is missing
                let hash = update.info.clone().unwrap_or_default().code_hash;
                Ok((
                    hash,
                    logger
                        .db
                        .store
                        .get_account_code(hash)?
                        .ok_or(ExecutionDBError::NewMissingCode(hash))?,
                ))
            })
            .collect::<Result<_, ExecutionDBError>>()?;
        let storage = execution_updates
            .iter()
            .map(|update| {
                // return error if storage is missing
                Ok((
                    update.address,
                    update
                        .added_storage
                        .keys()
                        .map(|key| {
                            let key = H256::from(key.to_fixed_bytes());
                            let value = logger
                                .db
                                .store
                                .get_storage_at_hash(parent_hash, update.address, key)
                                .map_err(ExecutionDBError::Store)?
                                .ok_or(ExecutionDBError::NewMissingStorage(update.address, key))?;
                            Ok((key, value))
                        })
                        .collect::<Result<HashMap<_, _>, ExecutionDBError>>()?,
                ))
            })
            .collect::<Result<HashMap<_, _>, ExecutionDBError>>()?;
        let block_hashes = logger
            .block_hashes_accessed
            .lock()
            .unwrap()
            .clone()
            .into_iter()
            .map(|(num, hash)| (num, H256::from(hash.0)))
            .collect();

        // get account proofs
        let state_trie = store_wrapper
            .store
            .state_trie(block.hash())?
            .ok_or(ExecutionDBError::NewMissingStateTrie(parent_hash))?;
        let parent_state_trie = store_wrapper
            .store
            .state_trie(parent_hash)?
            .ok_or(ExecutionDBError::NewMissingStateTrie(parent_hash))?;
        let hashed_addresses: Vec<_> = index
            .clone()
            .map(|(address, _)| hash_address(&address))
            .collect();
        let initial_state_proofs = parent_state_trie.get_proofs(&hashed_addresses)?;
        let final_state_proofs: Vec<_> = hashed_addresses
            .iter()
            .map(|hashed_address| Ok((hashed_address, state_trie.get_proof(hashed_address)?)))
            .collect::<Result<_, TrieError>>()?;
        let potential_account_child_nodes = final_state_proofs
            .iter()
            .filter_map(|(hashed_address, proof)| get_potential_child_nodes(proof, hashed_address))
            .flat_map(|nodes| nodes.into_iter().map(|node| node.encode_raw()))
            .collect();
        let state_proofs = (
            initial_state_proofs.0,
            [initial_state_proofs.1, potential_account_child_nodes].concat(),
        );

        // get storage proofs
        let mut storage_proofs = HashMap::new();
        let mut final_storage_proofs = HashMap::new();
        for (address, storage_keys) in index {
            let Some(parent_storage_trie) =
                store_wrapper.store.storage_trie(parent_hash, address)?
            else {
                // the storage of this account was empty or the account is newly created, either
                // way the storage trie was initially empty so there aren't any proofs to add.
                continue;
            };
            let storage_trie = store_wrapper
                .store
                .storage_trie(block.hash(), address)?
                .ok_or(ExecutionDBError::NewMissingStorageTrie(
                    block.hash(),
                    address,
                ))?;
            let paths = storage_keys.iter().map(hash_key).collect::<Vec<_>>();

            let initial_proofs = parent_storage_trie.get_proofs(&paths)?;
            let final_proofs: Vec<(_, Vec<_>)> = storage_keys
                .iter()
                .map(|key| {
                    let hashed_key = hash_key(key);
                    let proof = storage_trie.get_proof(&hashed_key)?;
                    Ok((hashed_key, proof))
                })
                .collect::<Result<_, TrieError>>()?;

            let potential_child_nodes: Vec<NodeRLP> = final_proofs
                .iter()
                .filter_map(|(hashed_key, proof)| get_potential_child_nodes(proof, hashed_key))
                .flat_map(|nodes| nodes.into_iter().map(|node| node.encode_raw()))
                .collect();
            let proofs = (
                initial_proofs.0,
                [initial_proofs.1, potential_child_nodes].concat(),
            );

            storage_proofs.insert(address, proofs);
            final_storage_proofs.insert(address, final_proofs);
        }

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

/// `new_state` is being modified at the end.
pub fn generic_system_contract_levm(
    block_header: &BlockHeader,
    calldata: Bytes,
    db: Arc<dyn LevmDatabase>,
    new_state: &mut CacheDB,
    contract_address: Address,
    system_address: Address,
) -> Result<ExecutionReport, EvmError> {
    let chain_config = db.get_chain_config();
    let config = EVMConfig::new_from_chain_config(&chain_config, block_header);
    let env = Environment {
        origin: system_address,
        gas_limit: 30_000_000,
        block_number: block_header.number.into(),
        coinbase: block_header.coinbase,
        timestamp: block_header.timestamp.into(),
        prev_randao: Some(block_header.prev_randao),
        base_fee_per_gas: U256::zero(),
        gas_price: U256::zero(),
        block_excess_blob_gas: block_header.excess_blob_gas.map(U256::from),
        block_blob_gas_used: block_header.blob_gas_used.map(U256::from),
        block_gas_limit: 30_000_000,
        transient_storage: HashMap::new(),
        config,
        ..Default::default()
    };

    let mut vm = VM::new(
        TxKind::Call(contract_address),
        env,
        U256::zero(),
        calldata,
        db.clone(),
        new_state.clone(),
        vec![],
        None,
    )
    .map_err(EvmError::from)?;

    let mut report = vm.execute().map_err(EvmError::from)?;

    report.new_state.remove(&system_address);

    match report.result {
        TxResult::Success => {}
        _ => {
            return Err(EvmError::Custom(
                "ERROR in generic_system_contract_levm(). TX didn't succeed.".to_owned(),
            ))
        }
    }

    // new_state is a CacheDB coming from outside the function
    for (address, account) in report.new_state.iter_mut() {
        if let Some(existing_account) = new_state.get(address) {
            let mut existing_storage = existing_account.storage.clone();
            existing_storage.extend(account.storage.clone());
            account.storage = existing_storage;
            account.info.balance = existing_account.info.balance;
        }
    }
    new_state.extend(report.new_state.clone());

    Ok(report)
}

#[allow(unreachable_code)]
#[allow(unused_variables)]
pub fn extract_all_requests_levm(
    receipts: &[Receipt],
    db: Arc<dyn LevmDatabase>,
    header: &BlockHeader,
    cache: &mut CacheDB,
) -> Result<Vec<Requests>, EvmError> {
    let config = db.get_chain_config();
    let fork = config.fork(header.timestamp);

    if fork < Fork::Prague {
        return Ok(Default::default());
    }

    cfg_if::cfg_if! {
        if #[cfg(feature = "l2")] {
            return Ok(Default::default());
        }
    }

    let withdrawals_data: Vec<u8> = match LEVM::read_withdrawal_requests(header, db.clone(), cache)
    {
        Some(report) => {
            // the cache is updated inside the generic_system_call
            report.output.into()
        }
        None => Default::default(),
    };

    let consolidation_data: Vec<u8> =
        match LEVM::dequeue_consolidation_requests(header, db.clone(), cache) {
            Some(report) => {
                // the cache is updated inside the generic_system_call
                report.output.into()
            }
            None => Default::default(),
        };

    let deposits = Requests::from_deposit_receipts(config.deposit_contract_address, receipts);
    let withdrawals = Requests::from_withdrawals_data(withdrawals_data);
    let consolidation = Requests::from_consolidation_data(consolidation_data);

    Ok(vec![deposits, withdrawals, consolidation])
}

/// Calculating gas_price according to EIP-1559 rules
/// See https://github.com/ethereum/go-ethereum/blob/7ee9a6e89f59cee21b5852f5f6ffa2bcfc05a25f/internal/ethapi/transaction_args.go#L430
pub fn calculate_gas_price(tx: &GenericTransaction, basefee: u64) -> U256 {
    if tx.gas_price != 0 {
        // Legacy gas field was specified, use it
        tx.gas_price.into()
    } else {
        // Backfill the legacy gas price for EVM execution, (zero if max_fee_per_gas is zero)
        min(
            tx.max_priority_fee_per_gas.unwrap_or(0) + basefee,
            tx.max_fee_per_gas.unwrap_or(0),
        )
        .into()
    }
}

/// When basefee tracking is disabled  (ie. env.disable_base_fee = true; env.disable_block_gas_limit = true;)
/// and no gas prices were specified, lower the basefee to 0 to avoid breaking EVM invariants (basefee < feecap)
/// See https://github.com/ethereum/go-ethereum/blob/00294e9d28151122e955c7db4344f06724295ec5/core/vm/evm.go#L137
fn adjust_disabled_base_fee(env: &mut Environment) {
    if env.gas_price == U256::zero() {
        env.base_fee_per_gas = U256::zero();
    }
    if env
        .tx_max_fee_per_blob_gas
        .is_some_and(|v| v == U256::zero())
    {
        env.block_excess_blob_gas = None;
    }
}
