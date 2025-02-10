use crate::db::StoreWrapper;
use crate::EvmError;
use crate::EvmState;
#[cfg(not(feature = "l2"))]
use ethrex_core::types::Fork;
use ethrex_core::{
    types::{
        code_hash, AccountInfo, Block, BlockHeader, Receipt, Transaction, TxKind, GWEI_TO_WEI,
    },
    Address, H256, U256,
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

use super::{SystemContracts, IEVM};

/// Input for [LEVM::execute_tx]
pub struct LevmTransactionExecutionIn<'a> {
    /// The transaction to execute.
    tx: &'a Transaction,
    /// The block header for the current block.
    block_header: &'a BlockHeader,
    /// The database to use for EVM state access.  This is wrapped in an `Arc` for shared ownership.
    db: Arc<dyn LevmDatabase>,
    /// A cache database for intermediate state changes during execution.
    block_cache: CacheDB,
    /// The EVM configuration to use.
    config: EVMConfig,
}

impl<'a> LevmTransactionExecutionIn<'a> {
    pub fn new(
        tx: &'a Transaction,
        block_header: &'a BlockHeader,
        db: Arc<dyn LevmDatabase>,
        block_cache: CacheDB,
        config: EVMConfig,
    ) -> Self {
        LevmTransactionExecutionIn {
            tx,
            block_header,
            db,
            block_cache,
            config,
        }
    }
}

/// Input for [LEVM::get_state_transitions]
pub struct LevmGetStateTransitionsIn<'a> {
    initial_state: &'a EvmState,
    block_hash: H256,
    new_state: &'a CacheDB,
}

impl<'a> LevmGetStateTransitionsIn<'a> {
    pub fn new(initial_state: &'a EvmState, block_hash: H256, new_state: &'a CacheDB) -> Self {
        LevmGetStateTransitionsIn {
            initial_state,
            block_hash,
            new_state,
        }
    }
}

#[derive(Debug)]
pub struct LEVM;

impl IEVM for LEVM {
    type Error = EvmError;

    type BlockExecutionOutput = (Vec<Receipt>, Vec<AccountUpdate>);

    type TransactionExecutionInput<'a> = LevmTransactionExecutionIn<'a>;

    type TransactionExecutionResult = ExecutionReport;

    type GetStateTransitionsInput<'a> = LevmGetStateTransitionsIn<'a>;

    fn execute_block(
        block: &Block,
        state: &mut EvmState,
    ) -> Result<Self::BlockExecutionOutput, Self::Error> {
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
                    let report = Self::beacon_root_contract_call(
                        block_header,
                        LevmSystemCallIn::new(store_wrapper.clone(), config),
                    )?;
                    block_cache.extend(report.new_state);
                }
            }
        }

        // Account updates are initialized like this because of the beacon_root_contract_call, it is going to be empty if it wasn't called.
        let mut account_updates = crate::get_state_transitions(state);

        let mut receipts = Vec::new();
        let mut cumulative_gas_used = 0;

        for tx in block.body.transactions.iter() {
            let report = Self::execute_tx(LevmTransactionExecutionIn::new(
                tx,
                block_header,
                store_wrapper.clone(),
                block_cache.clone(),
                config,
            ))
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

        account_updates.extend(Self::get_state_transitions(LevmGetStateTransitionsIn::new(
            state,
            block.header.parent_hash,
            &block_cache,
        )));

        Ok((receipts, account_updates))
    }

    fn execute_tx(
        input: Self::TransactionExecutionInput<'_>,
    ) -> Result<Self::TransactionExecutionResult, Self::Error> {
        let block_header = input.block_header;
        let tx = input.tx;
        let gas_price: U256 = tx
            .effective_gas_price(block_header.base_fee_per_gas)
            .ok_or(VMError::InvalidTransaction)?
            .into();

        let env = Environment {
            origin: tx.sender(),
            refunded_gas: 0,
            gas_limit: tx.gas_limit(),
            config: input.config,
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
            input.db,
            input.block_cache,
            tx.access_list(),
            tx.authorization_list(),
        )?;

        vm.execute().map_err(VMError::into)
    }

    fn get_state_transitions(input: Self::GetStateTransitionsInput<'_>) -> Vec<AccountUpdate> {
        let current_db = match input.initial_state {
            EvmState::Store(state) => state.database.store.clone(),
            EvmState::Execution(_cache_db) => {
                unreachable!("Execution state should not be passed here")
            }
        };
        let mut account_updates: Vec<AccountUpdate> = vec![];
        for (new_state_account_address, new_state_account) in input.new_state {
            let initial_account_state = current_db
                .get_account_info_by_hash(input.block_hash, *new_state_account_address)
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
                    .get_storage_at_hash(input.block_hash, *new_state_account_address, *key)
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
}

pub struct LevmSystemCallIn {
    // CHECK: is it ok to use StoreWrapper.
    store_wrapper: Arc<StoreWrapper>,
    config: EVMConfig,
}

impl LevmSystemCallIn {
    pub fn new(store_wrapper: Arc<StoreWrapper>, config: EVMConfig) -> Self {
        LevmSystemCallIn {
            store_wrapper,
            config,
        }
    }
}

impl SystemContracts for LEVM {
    type Error = EvmError;

    type Evm = LEVM;

    type SystemCallInput<'a> = LevmSystemCallIn;

    fn beacon_root_contract_call(
        block_header: &BlockHeader,
        input: Self::SystemCallInput<'_>,
    ) -> Result<<Self::Evm as IEVM>::TransactionExecutionResult, Self::Error> {
        lazy_static! {
            static ref SYSTEM_ADDRESS: Address = Address::from_slice(
                &hex::decode("fffffffffffffffffffffffffffffffffffffffe").unwrap()
            );
            static ref CONTRACT_ADDRESS: Address = Address::from_slice(
                &hex::decode("000F3df6D732807Ef1319fB7B8bB8522d0Beac02").unwrap(),
            );
        };
        // This is OK
        let beacon_root = match block_header.parent_beacon_block_root {
            None => {
                return Err(EvmError::Header(
                    "parent_beacon_block_root field is missing".to_string(),
                ))
            }
            Some(beacon_root) => beacon_root,
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
            config: input.config,
            ..Default::default()
        };

        let calldata = Bytes::copy_from_slice(beacon_root.as_bytes()).into();

        // Return transaction report. Commit the transaction to the State in the calling place.
        let mut vm = VM::new(
            TxKind::Call(*CONTRACT_ADDRESS),
            env,
            U256::zero(),
            calldata,
            input.store_wrapper,
            CacheDB::new(),
            vec![],
            None,
        )
        .map_err(EvmError::from)?;

        let mut report = vm.execute().map_err(EvmError::from)?;

        report.new_state.remove(&*SYSTEM_ADDRESS);

        Ok(report)
    }
}
