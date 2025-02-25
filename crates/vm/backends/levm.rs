use super::constants::{
    BEACON_ROOTS_ADDRESS, CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS, HISTORY_STORAGE_ADDRESS,
    SYSTEM_ADDRESS, WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
};
use super::BlockExecutionResult;
use crate::backends::get_state_transitions;

use crate::db::StoreWrapper;
use crate::EvmError;
use crate::EvmState;
use ethrex_common::types::requests::Requests;
use ethrex_common::types::Fork;
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
use ethrex_storage::{error::StoreError, AccountUpdate, Store};
use revm_primitives::bitvec::store;
use revm_primitives::Bytes;
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
    pub fn execute_block(block: &Block, store: Store) -> Result<BlockExecutionResult, EvmError> {
        let store_wrapper = Arc::new(StoreWrapper {
            store: store.clone(),
            block_hash: block.header.parent_hash,
        });

        let mut block_cache: CacheDB = HashMap::new();
        let block_header = &block.header;
        let config = store.get_chain_config()?;
        cfg_if::cfg_if! {
            if #[cfg(not(feature = "l2"))] {
                let fork = config.fork(block_header.timestamp);
                if block_header.parent_beacon_block_root.is_some() && fork >= Fork::Cancun {
                    Self::beacon_root_contract_call(block_header, &store, &mut block_cache)?;
                }

                if fork >= Fork::Prague {
                    //eip 2935: stores parent block hash in system contract
                    Self::process_block_hash_history(block_header, &store, &mut block_cache)?;
                }
            }
        }

        // Account updates are initialized like this because of the beacon_root_contract_call, it is going to be empty if it wasn't called.
        // Here we get the state_transitions from the db and then we get the state_transitions from the cache_db.
        let mut account_updates =
            Self::get_state_transitions(None, &store, block.header.parent_hash, &block_cache)?;
        let mut receipts = Vec::new();
        let mut cumulative_gas_used = 0;

        for tx in block.body.transactions.iter() {
            let report = Self::execute_tx(
                tx,
                block_header,
                store_wrapper.clone(),
                block_cache.clone(),
                &config,
            )
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
                    let acc_info = store_wrapper.get_account_info(address);
                    Account::from(acc_info)
                });

                account.info.balance += increment.into();

                block_cache.insert(address, account);
            }
        }

        let requests =
            extract_all_requests_levm(&receipts, &store, &block.header, &mut block_cache)?;

        account_updates.extend(Self::get_state_transitions(
            None,
            &store,
            block.header.parent_hash,
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
            db,
            block_cache.clone(),
            tx.access_list(),
            tx.authorization_list(),
        )?;

        vm.execute().map_err(VMError::into)
    }

    pub fn get_state_transitions(
        // Warning only pass the fork if running the ef-tests.
        // ISSUE #2021: https://github.com/lambdaclass/ethrex/issues/2021
        ef_tests: Option<Fork>,
        store: &Store,
        block_hash: H256,
        new_state: &CacheDB,
    ) -> Result<Vec<AccountUpdate>, EvmError> {
        let current_db = store.clone();
        let mut account_updates: Vec<AccountUpdate> = vec![];
        for (new_state_account_address, new_state_account) in new_state {
            let initial_account_state = current_db
                .get_account_info_by_hash(block_hash, *new_state_account_address)
                .expect("Error getting account info by address")
                .unwrap_or_default();
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
                // Get the code hash of the new state account bytecode
                let potential_new_bytecode_hash = code_hash(&new_state_account.info.bytecode);
                // Look into the current database to see if the bytecode hash is already present
                let current_bytecode = current_db
                    .get_account_code(potential_new_bytecode_hash)
                    .expect("Error getting account code by hash");
                let code = new_state_account.info.bytecode.clone();
                // The code is present in the current database
                if let Some(current_bytecode) = current_bytecode {
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

            let block_header = current_db
                .get_block_header_by_hash(block_hash)?
                .ok_or(StoreError::MissingStore)?;
            let fork_from_config = store.get_chain_config()?.fork(block_header.timestamp);
            // Here we take the passed fork through the ef_tests variable, or we set it to the fork based on the timestamp.
            let fork = ef_tests.unwrap_or(fork_from_config);
            if let Some(old_info) =
                current_db.get_account_info_by_hash(block_hash, account_update.address)?
            {
                // https://eips.ethereum.org/EIPS/eip-161
                // if an account was empty and is now empty, after spurious dragon, it should be removed
                if account_update.removed
                    && old_info.balance.is_zero()
                    && old_info.nonce == 0
                    && old_info.code_hash == code_hash(&Bytes::new())
                    && fork < Fork::SpuriousDragon
                {
                    continue;
                }
            }

            account_updates.push(account_update);
        }
        Ok(account_updates)
    }

    pub fn process_withdrawals(
        block_cache: &mut CacheDB,
        withdrawals: &[Withdrawal],
        store: Option<&Store>,
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
                    .ok_or(StoreError::MissingStore)?
                    .get_account_info_by_hash(parent_hash, address)?
                    .unwrap_or_default();
                let acc_code = store
                    .ok_or(StoreError::MissingStore)?
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
        store: &Store,
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
            store,
            new_state,
            *BEACON_ROOTS_ADDRESS,
            *SYSTEM_ADDRESS,
        )?;
        Ok(())
    }
    /// `new_state` is being modified inside [generic_system_contract_levm].
    pub fn process_block_hash_history(
        block_header: &BlockHeader,
        store: &Store,
        new_state: &mut CacheDB,
    ) -> Result<(), EvmError> {
        generic_system_contract_levm(
            block_header,
            Bytes::copy_from_slice(block_header.parent_hash.as_bytes()),
            store,
            new_state,
            *HISTORY_STORAGE_ADDRESS,
            *SYSTEM_ADDRESS,
        )?;
        Ok(())
    }
    pub(crate) fn read_withdrawal_requests(
        block_header: &BlockHeader,
        store: &Store,
        new_state: &mut CacheDB,
    ) -> Option<ExecutionReport> {
        let report = generic_system_contract_levm(
            block_header,
            Bytes::new(),
            store,
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
        store: &Store,
        new_state: &mut CacheDB,
    ) -> Option<ExecutionReport> {
        let report = generic_system_contract_levm(
            block_header,
            Bytes::new(),
            store,
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
}

/// `new_state` is being modified at the end.
pub fn generic_system_contract_levm(
    block_header: &BlockHeader,
    calldata: Bytes,
    store: &Store,
    new_state: &mut CacheDB,
    contract_address: Address,
    system_address: Address,
) -> Result<ExecutionReport, EvmError> {
    let store_wrapper = Arc::new(StoreWrapper {
        store: store.clone(),
        block_hash: block_header.parent_hash,
    });

    let chain_config = store.get_chain_config()?;
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
        calldata.into(),
        store_wrapper,
        CacheDB::new(),
        vec![],
        None,
    )
    .map_err(EvmError::from)?;

    let mut report = vm.execute().map_err(EvmError::from)?;

    report.new_state.remove(&system_address);

    match report.result {
        TxResult::Success => {
            new_state.extend(report.new_state.clone());
        }
        _ => {
            return Err(EvmError::Custom(
                "ERROR in generic_system_contract_levm(). TX didn't succeed.".to_owned(),
            ))
        }
    }

    // new_state is a CacheDB coming from outside the function
    new_state.extend(report.new_state.clone());

    Ok(report)
}

#[allow(unreachable_code)]
#[allow(unused_variables)]
pub fn extract_all_requests_levm(
    receipts: &[Receipt],
    store: &Store,
    header: &BlockHeader,
    cache: &mut CacheDB,
) -> Result<Vec<Requests>, EvmError> {
    let config = store.get_chain_config()?;
    let fork = config.fork(header.timestamp);

    if fork < Fork::Prague {
        return Ok(Default::default());
    }

    cfg_if::cfg_if! {
        if #[cfg(feature = "l2")] {
            return Ok(Default::default());
        }
    }

    let deposit_contract_address = config.deposit_contract_address.ok_or(EvmError::Custom(
        "deposit_contract_address config is missing".to_string(),
    ))?;

    let withdrawals_data: Vec<u8> = match LEVM::read_withdrawal_requests(header, store, cache) {
        Some(report) => {
            // the cache is updated inside the generic_system_call
            report.output.into()
        }
        None => Default::default(),
    };

    let consolidation_data: Vec<u8> =
        match LEVM::dequeue_consolidation_requests(header, store, cache) {
            Some(report) => {
                // the cache is updated inside the generic_system_call
                report.output.into()
            }
            None => Default::default(),
        };

    let deposits = Requests::from_deposit_receipts(deposit_contract_address, receipts);
    let withdrawals = Requests::from_withdrawals_data(withdrawals_data);
    let consolidation = Requests::from_consolidation_data(consolidation_data);

    Ok(vec![deposits, withdrawals, consolidation])
}
