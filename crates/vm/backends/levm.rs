use super::constants::{
    BEACON_ROOTS_ADDRESS_STR, CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS, HISTORY_STORAGE_ADDRESS_STR,
    SYSTEM_ADDRESS_STR, WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
};
use crate::db::StoreWrapper;
use crate::EvmError;
use crate::EvmState;
use ethrex_common::types::requests::Requests;
use ethrex_common::types::Fork;
use ethrex_common::{
    types::{
        code_hash, AccountInfo, Block, BlockHeader, Receipt, Transaction, TxKind, GWEI_TO_WEI,
    },
    Address, Bytes as CoreBytes, H256, U256,
};

use ethrex_levm::{
    db::{CacheDB, Database as LevmDatabase},
    errors::{ExecutionReport, TxResult, VMError},
    vm::{EVMConfig, VM},
    Account, Environment,
};
use ethrex_storage::AccountUpdate;
use lazy_static::lazy_static;
use revm_primitives::Bytes;
use std::{collections::HashMap, sync::Arc};

pub struct BlockExecutionResult {
    pub receipts: Vec<Receipt>,
    pub requests: Vec<Requests>,
    pub account_updates: Vec<AccountUpdate>,
}

/// Executes all transactions in a block and returns their receipts.
pub fn execute_block(
    block: &Block,
    state: &mut EvmState,
) -> Result<BlockExecutionResult, EvmError> {
    let store_wrapper = Arc::new(StoreWrapper {
        store: state.database().unwrap().clone(),
        block_hash: block.header.parent_hash,
    });

    let mut block_cache: CacheDB = HashMap::new();
    let block_header = &block.header;
    let fork = state.chain_config()?.fork(block_header.timestamp);
    // If there's no blob schedule in chain_config use the
    // default/canonical values
    let blob_schedule = state
        .chain_config()?
        .get_fork_blob_schedule(block_header.timestamp)
        .unwrap_or(EVMConfig::canonical_values(fork));
    let config = EVMConfig::new(fork, blob_schedule);
    cfg_if::cfg_if! {
        if #[cfg(not(feature = "l2"))] {
            if block_header.parent_beacon_block_root.is_some() && fork >= Fork::Cancun {
                let report = beacon_root_contract_call_levm(store_wrapper.clone(), block_header, config)?;
                block_cache.extend(report.new_state);
            }

            if fork >= Fork::Prague {
                //eip 2935: stores parent block hash in system contract
                let report = process_block_hash_history(store_wrapper.clone(), block_header, config)?;
                block_cache.extend(report.new_state);
            }
        }
    }

    // Account updates are initialized like this because of the beacon_root_contract_call, it is going to be empty if it wasn't called.
    let mut account_updates = crate::get_state_transitions(state);

    let mut receipts = Vec::new();
    let mut cumulative_gas_used = 0;

    for tx in block.body.transactions.iter() {
        let report = execute_tx_levm(
            tx,
            block_header,
            store_wrapper.clone(),
            block_cache.clone(),
            config,
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

    let requests = extract_all_requests_levm(&receipts, state, &block.header, &mut block_cache)?;

    account_updates.extend(get_state_transitions_levm(
        state,
        block.header.parent_hash,
        &block_cache,
    ));

    Ok(BlockExecutionResult {
        receipts,
        requests,
        account_updates,
    })
}

pub fn execute_tx_levm(
    tx: &Transaction,
    block_header: &BlockHeader,
    db: Arc<dyn LevmDatabase>,
    block_cache: CacheDB,
    config: EVMConfig,
) -> Result<ExecutionReport, VMError> {
    let gas_price: U256 = tx
        .effective_gas_price(block_header.base_fee_per_gas)
        .ok_or(VMError::InvalidTransaction)?
        .into();

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
    };

    let mut vm = VM::new(
        tx.to(),
        env,
        tx.value(),
        tx.data().clone(),
        db,
        block_cache,
        tx.access_list(),
        tx.authorization_list(),
    )?;

    vm.execute()
}

pub fn get_state_transitions_levm(
    initial_state: &EvmState,
    block_hash: H256,
    new_state: &CacheDB,
) -> Vec<AccountUpdate> {
    let current_db = match initial_state {
        EvmState::Store(state) => state.database.store.clone(),
        EvmState::Execution(_cache_db) => unreachable!("Execution state should not be passed here"),
    };
    let mut account_updates: Vec<AccountUpdate> = vec![];
    for (new_state_account_address, new_state_account) in new_state {
        let initial_account_state = current_db
            .get_account_info_by_hash(block_hash, *new_state_account_address)
            .expect("Error getting account info by address");

        if initial_account_state.is_none() {
            // New account, update everything
            let new_account = AccountUpdate {
                address: *new_state_account_address,
                removed: new_state_account.is_empty(),
                info: Some(AccountInfo {
                    code_hash: code_hash(&new_state_account.info.bytecode),
                    balance: new_state_account.info.balance,
                    nonce: new_state_account.info.nonce,
                }),
                code: Some(new_state_account.info.bytecode.clone()),
                added_storage: new_state_account
                    .storage
                    .iter()
                    .map(|(key, storage_slot)| (*key, storage_slot.current_value))
                    .collect(),
            };

            account_updates.push(new_account);
            continue;
        }

        // This unwrap is safe, just checked upside
        let initial_account_state = initial_account_state.unwrap();
        let mut account_update = AccountUpdate::new(*new_state_account_address);

        // Account state after block execution.
        let new_state_acc_info = AccountInfo {
            code_hash: code_hash(&new_state_account.info.bytecode),
            balance: new_state_account.info.balance,
            nonce: new_state_account.info.nonce,
        };

        // Compare Account Info
        if initial_account_state != new_state_acc_info {
            account_update.info = Some(new_state_acc_info.clone());
        }

        // If code hash is different it means the code is different too.
        if initial_account_state.code_hash != new_state_acc_info.code_hash {
            account_update.code = Some(new_state_account.info.bytecode.clone());
        }

        let mut updated_storage = HashMap::new();
        for (key, storage_slot) in &new_state_account.storage {
            // original_value in storage_slot is not the original_value on the DB, be careful.
            let original_value = current_db
                .get_storage_at_hash(block_hash, *new_state_account_address, *key)
                .unwrap()
                .unwrap_or_default(); // Option inside result, I guess I have to assume it is zero.

            if original_value != storage_slot.current_value {
                updated_storage.insert(*key, storage_slot.current_value);
            }
        }
        account_update.added_storage = updated_storage;

        account_update.removed = new_state_account.is_empty();

        if account_update != AccountUpdate::new(*new_state_account_address) {
            account_updates.push(account_update);
        }
    }
    account_updates
}

/// Calls the eip4788 beacon block root system call contract
/// More info on https://eips.ethereum.org/EIPS/eip-4788
pub fn beacon_root_contract_call_levm(
    store_wrapper: Arc<StoreWrapper>,
    header: &BlockHeader,
    config: EVMConfig,
) -> Result<ExecutionReport, EvmError> {
    lazy_static! {
        static ref CONTRACT_ADDRESS: Address =
            Address::from_slice(&hex::decode(BEACON_ROOTS_ADDRESS_STR).unwrap(),);
    };

    let beacon_root = header.parent_beacon_block_root.ok_or(EvmError::Header(
        "parent_beacon_block_root field is missing".to_string(),
    ))?;

    let calldata = Bytes::copy_from_slice(beacon_root.as_bytes()).into();

    generic_system_call(
        *CONTRACT_ADDRESS,
        calldata,
        store_wrapper,
        header,
        config,
    )
}

/// Calls the EIP-2935 process block hashes history system call contract
/// NOTE: This was implemented by making use of an EVM system contract, but can be changed to a
/// direct state trie update after the verkle fork, as explained in https://eips.ethereum.org/EIPS/eip-2935
pub fn process_block_hash_history(
    store_wrapper: Arc<StoreWrapper>,
    block_header: &BlockHeader,
    config: EVMConfig,
) -> Result<ExecutionReport, EvmError> {
    lazy_static! {
        static ref CONTRACT_ADDRESS: Address =
            Address::from_slice(&hex::decode(HISTORY_STORAGE_ADDRESS_STR).unwrap(),);
    };

    let calldata = Bytes::copy_from_slice(block_header.parent_hash.as_bytes()).into();

    generic_system_call(
        *CONTRACT_ADDRESS,
        calldata,
        store_wrapper,
        block_header,
        config,
    )
}

fn read_withdrawal_requests_levm(
    store_wrapper: Arc<StoreWrapper>,
    block_header: &BlockHeader,
    config: EVMConfig,
) -> Option<ExecutionReport> {
    lazy_static! {
        static ref CONTRACT_ADDRESS: Address =
            Address::from_slice(&hex::decode(WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS).unwrap());
    };

    let report = generic_system_call(
        *CONTRACT_ADDRESS,
        CoreBytes::new(),
        store_wrapper,
        block_header,
        config,
    )
    .ok()?;

    match report.result {
        TxResult::Success => Some(report),
        _ => None,
    }
}

fn dequeue_consolidation_requests(
    store_wrapper: Arc<StoreWrapper>,
    block_header: &BlockHeader,
    config: EVMConfig,
) -> Option<ExecutionReport> {
    lazy_static! {
        static ref CONTRACT_ADDRESS: Address =
            Address::from_slice(&hex::decode(CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS).unwrap());
    };

    let report = generic_system_call(
        *CONTRACT_ADDRESS,
        CoreBytes::new(),
        store_wrapper,
        block_header,
        config,
    )
    .ok()?;

    match report.result {
        TxResult::Success => Some(report),
        _ => None,
    }
}

pub fn generic_system_call(
    contract_address: Address,
    calldata: CoreBytes,
    store_wrapper: Arc<StoreWrapper>,
    block_header: &BlockHeader,
    config: EVMConfig,
) -> Result<ExecutionReport, EvmError> {
    lazy_static! {
        static ref SYSTEM_ADDRESS: Address =
            Address::from_slice(&hex::decode(SYSTEM_ADDRESS_STR).unwrap());
    };

    let env = Environment {
        origin: *SYSTEM_ADDRESS,
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

    // Here execute with LEVM but just return transaction report. And I will handle it in the calling place.

    let mut vm = VM::new(
        TxKind::Call(contract_address),
        env,
        U256::zero(),
        calldata,
        store_wrapper,
        CacheDB::new(),
        vec![],
        None,
    )
    .map_err(EvmError::from)?;

    let mut report = vm.execute().map_err(EvmError::from)?;

    report.new_state.remove(&*SYSTEM_ADDRESS);

    Ok(report)
}

pub fn extract_all_requests_levm(
    receipts: &[Receipt],
    state: &mut EvmState,
    header: &BlockHeader,
    cache: &mut CacheDB,
) -> Result<Vec<Requests>, EvmError> {
    let config = state.chain_config()?;
    let fork = config.fork(header.timestamp);

    if fork < Fork::Prague {
        return Ok(Default::default());
    }

    let blob_schedule = config
        .get_fork_blob_schedule(header.timestamp)
        .unwrap_or(EVMConfig::canonical_values(fork));

    let evm_config = EVMConfig::new(fork, blob_schedule);

    let store_wrapper = Arc::new(StoreWrapper {
        store: state.database().unwrap().clone(),
        block_hash: header.parent_hash,
    });

    let withdrawals_data: Vec<u8> =
        match read_withdrawal_requests_levm(store_wrapper, header, evm_config) {
            Some(report) => {
                cache.extend(report.new_state.clone());
                report.output.into()
            }
            None => Default::default(),
        };

    let store_wrapper = Arc::new(StoreWrapper {
        store: state.database().unwrap().clone(),
        block_hash: header.parent_hash,
    });

    let consolidation_data: Vec<u8> =
        match dequeue_consolidation_requests(store_wrapper, header, evm_config) {
            Some(report) => {
                cache.extend(report.new_state.clone());
                report.output.into()
            }
            None => Default::default(),
        };

    let deposits = Requests::from_deposit_receipts(receipts);
    let withdrawals = Requests::from_withdrawals_data(withdrawals_data);
    let consolidation = Requests::from_consolidation_data(consolidation_data);

    Ok(vec![deposits, withdrawals, consolidation])
}
