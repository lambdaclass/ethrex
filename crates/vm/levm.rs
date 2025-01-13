use crate::{db::StoreWrapper, revm, revm::RevmSpecId, BlockExecutionOutput, EvmError, EvmState};
use ethrex_core::types::code_hash;
use ethrex_core::{
    types::{AccountInfo, Block, BlockHeader, Receipt, Transaction, GWEI_TO_WEI},
    H256, U256,
};
use ethrex_levm::{
    db::{CacheDB, Database as LevmDatabase},
    errors::{TransactionReport, TxResult, VMError},
    vm::VM,
    Account, Environment,
};
use ethrex_storage::AccountUpdate;
use std::{collections::HashMap, sync::Arc};

pub fn get_state_transitions(
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
        // This stores things that have changed in the account.
        let mut account_update = AccountUpdate::new(*new_state_account_address);

        // Account state before block execution.
        let initial_account_state = current_db
            .get_account_info_by_hash(block_hash, *new_state_account_address)
            .expect("Error getting account info by address")
            .unwrap_or_default();
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

/// Executes all transactions in a block and returns their receipts.
pub fn execute_block(
    block: &Block,
    state: &mut EvmState,
) -> Result<BlockExecutionOutput, EvmError> {
    let block_header = &block.header;
    let spec_id = revm::spec_id(&state.chain_config()?, block_header.timestamp);
    //eip 4788: execute beacon_root_contract_call before block transactions
    cfg_if::cfg_if! {
        if #[cfg(not(feature = "l2"))] {
            if block_header.parent_beacon_block_root.is_some() && spec_id == RevmSpecId::CANCUN {
                revm::beacon_root_contract_call(state, block_header, spec_id)?;
            }
        }
    }

    let store_wrapper = Arc::new(StoreWrapper {
        store: state.database().unwrap().clone(),
        block_hash: block.header.parent_hash,
    });

    // Account updates are initialized like this because of the beacon_root_contract_call, it is going to be empty if it wasn't called.
    let mut account_updates = revm::get_state_transitions(state);

    let mut receipts = Vec::new();
    let mut cumulative_gas_used = 0;
    let mut block_cache: CacheDB = HashMap::new();

    for tx in block.body.transactions.iter() {
        let report = execute_tx(
            tx,
            block_header,
            store_wrapper.clone(),
            block_cache.clone(),
            spec_id,
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

    account_updates.extend(get_state_transitions(
        state,
        block.header.parent_hash,
        &block_cache,
    ));

    Ok((receipts, account_updates))
}

pub fn execute_tx(
    tx: &Transaction,
    block_header: &BlockHeader,
    db: Arc<dyn LevmDatabase>,
    block_cache: CacheDB,
    spec_id: RevmSpecId,
) -> Result<TransactionReport, VMError> {
    let gas_price: U256 = tx
        .effective_gas_price(block_header.base_fee_per_gas)
        .ok_or(VMError::InvalidTransaction)?
        .into();

    let env = Environment {
        origin: tx.sender(),
        refunded_gas: 0,
        gas_limit: tx.gas_limit(),
        spec_id,
        block_number: block_header.number.into(),
        coinbase: block_header.coinbase,
        timestamp: block_header.timestamp.into(),
        prev_randao: Some(block_header.prev_randao),
        chain_id: tx.chain_id().unwrap().into(),
        base_fee_per_gas: block_header.base_fee_per_gas.unwrap_or_default().into(),
        gas_price,
        block_excess_blob_gas: block_header.excess_blob_gas.map(U256::from),
        block_blob_gas_used: block_header.blob_gas_used.map(U256::from),
        tx_blob_hashes: tx.blob_versioned_hashes(),
        tx_max_priority_fee_per_gas: tx.max_priority_fee().map(U256::from),
        tx_max_fee_per_gas: tx.max_fee_per_gas().map(U256::from),
        tx_max_fee_per_blob_gas: tx.max_fee_per_blob_gas().map(U256::from),
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
    )?;

    vm.transact()
}
