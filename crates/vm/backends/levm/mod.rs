pub(crate) mod db;

use super::BlockExecutionResult;
use crate::constants::{
    BEACON_ROOTS_ADDRESS, CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS, HISTORY_STORAGE_ADDRESS,
    SYSTEM_ADDRESS, WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
};
use crate::{EvmError, ExecutionResult};
use bytes::Bytes;
use ethrex_common::types::requests::Requests;
use ethrex_common::types::{
    AccessList, AuthorizationTuple, Fork, GenericTransaction, INITIAL_BASE_FEE,
};
use ethrex_common::{
    types::{
        code_hash, AccountInfo, Block, BlockHeader, Receipt, Transaction, TxKind, Withdrawal,
        GWEI_TO_WEI,
    },
    Address, H256, U256,
};
use ethrex_levm::vm::{GeneralizedDatabase, Substate};
use ethrex_levm::AccountInfo as LevmAccountInfo;
use ethrex_levm::{
    errors::{ExecutionReport, TxResult, VMError},
    vm::{EVMConfig, VM},
    Account, Environment,
};
use ethrex_storage::error::StoreError;
use ethrex_storage::AccountUpdate;
use std::cmp::min;
use std::collections::HashMap;

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
        db: &mut GeneralizedDatabase,
    ) -> Result<BlockExecutionResult, EvmError> {
        let chain_config = db.store.get_chain_config();
        let block_header = &block.header;
        let fork = chain_config.fork(block_header.timestamp);
        cfg_if::cfg_if! {
            if #[cfg(not(feature = "l2"))] {
                if block_header.parent_beacon_block_root.is_some() && fork >= Fork::Cancun {
                    Self::beacon_root_contract_call(block_header, db)?;
                }

                if fork >= Fork::Prague {
                    //eip 2935: stores parent block hash in system contract
                    Self::process_block_hash_history(block_header, db)?;
                }
            }
        }

        let mut receipts = Vec::new();
        let mut cumulative_gas_used = 0;

        for (tx, tx_sender) in block.body.get_transactions_with_sender() {
            let report =
                Self::execute_tx(tx, tx_sender, &block.header, db).map_err(EvmError::from)?;

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

        if let Some(withdrawals) = &block.body.withdrawals {
            Self::process_withdrawals(db, withdrawals, block.header.parent_hash)?;
        }

        cfg_if::cfg_if! {
            if #[cfg(not(feature = "l2"))] {
                let requests = extract_all_requests_levm(&receipts, db, &block.header)?;
            } else {
                let requests = Default::default();
            }
        }

        let account_updates = Self::get_state_transitions(db)?;

        Ok(BlockExecutionResult {
            receipts,
            requests,
            account_updates,
        })
    }

    pub fn execute_tx(
        // The transaction to execute.
        tx: &Transaction,
        // The transactions recovered address
        tx_sender: Address,
        // The block header for the current block.
        block_header: &BlockHeader,
        db: &mut GeneralizedDatabase,
    ) -> Result<ExecutionReport, EvmError> {
        let chain_config = db.store.get_chain_config();
        let gas_price: U256 = tx
            .effective_gas_price(block_header.base_fee_per_gas)
            .ok_or(VMError::InvalidTransaction)?
            .into();

        let config = EVMConfig::new_from_chain_config(&chain_config, block_header);
        let env = Environment {
            origin: tx_sender,
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
        db: &mut GeneralizedDatabase,
    ) -> Result<ExecutionResult, EvmError> {
        let mut env = env_from_generic(tx, block_header, db)?;

        env.block_gas_limit = u64::MAX; // disable block gas limit

        adjust_disabled_base_fee(&mut env);

        let mut vm = vm_from_generic(tx, env, db)?;

        vm.execute()
            .map(|value| value.into())
            .map_err(VMError::into)
    }

    pub fn get_state_transitions(
        db: &mut GeneralizedDatabase,
    ) -> Result<Vec<AccountUpdate>, EvmError> {
        let mut account_updates: Vec<AccountUpdate> = vec![];
        for (address, new_state_account) in db.cache.drain() {
            let initial_state_account = db.store.get_account_info(address)?;
            // let mut updates = 0;
            let mut acc_info_updated = false;
            let mut storage_updated = false;

            // Account Info has been updated if balance, nonce or bytecode changed.
            if initial_state_account.balance != new_state_account.info.balance {
                acc_info_updated = true;
            }

            if initial_state_account.nonce != new_state_account.info.nonce {
                acc_info_updated = true;
            }

            let new_state_code_hash = code_hash(&new_state_account.info.bytecode);
            let code = if initial_state_account.bytecode_hash() != new_state_code_hash {
                acc_info_updated = true;
                Some(new_state_account.info.bytecode.clone())
            } else {
                None
            };

            let mut added_storage = HashMap::new();
            for (key, value) in &new_state_account.storage {
                let storage_before_block = db.store.get_storage_slot(address, *key)?;
                if value.current_value != storage_before_block {
                    added_storage.insert(*key, value.current_value);
                    storage_updated = true;
                }
            }

            let info = if acc_info_updated {
                Some(AccountInfo {
                    code_hash: new_state_code_hash,
                    balance: new_state_account.info.balance,
                    nonce: new_state_account.info.nonce,
                })
            } else {
                None
            };

            let removed = !initial_state_account.is_empty() && new_state_account.is_empty();

            if !removed && !acc_info_updated && !storage_updated {
                // Account hasn't been updated
                continue;
            }

            let account_update = AccountUpdate {
                address,
                removed,
                info,
                code,
                added_storage,
            };

            account_updates.push(account_update);
        }
        Ok(account_updates)
    }

    pub fn process_withdrawals(
        db: &mut GeneralizedDatabase,
        withdrawals: &[Withdrawal],
        parent_hash: H256,
    ) -> Result<(), ethrex_storage::error::StoreError> {
        // For every withdrawal we increment the target account's balance
        for (address, increment) in withdrawals
            .iter()
            .filter(|withdrawal| withdrawal.amount > 0)
            .map(|w| (w.address, u128::from(w.amount) * u128::from(GWEI_TO_WEI)))
        {
            // We check if it was in block_cache, if not, we get it from DB.
            let mut account = db.cache.get(&address).cloned().unwrap_or({
                let acc_info = db
                    .store
                    .get_account_info_by_hash(parent_hash, address)
                    .map_err(|e| StoreError::Custom(e.to_string()))?
                    .unwrap_or_default();
                let acc_code = db
                    .store
                    .get_account_code(acc_info.code_hash)
                    .map_err(|e| StoreError::Custom(e.to_string()))?
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
            db.cache.insert(address, account);
        }
        Ok(())
    }

    // SYSTEM CONTRACTS
    pub fn beacon_root_contract_call(
        block_header: &BlockHeader,
        db: &mut GeneralizedDatabase,
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
            *BEACON_ROOTS_ADDRESS,
            *SYSTEM_ADDRESS,
        )?;
        Ok(())
    }

    pub fn process_block_hash_history(
        block_header: &BlockHeader,
        db: &mut GeneralizedDatabase,
    ) -> Result<(), EvmError> {
        generic_system_contract_levm(
            block_header,
            Bytes::copy_from_slice(block_header.parent_hash.as_bytes()),
            db,
            *HISTORY_STORAGE_ADDRESS,
            *SYSTEM_ADDRESS,
        )?;
        Ok(())
    }
    pub(crate) fn read_withdrawal_requests(
        block_header: &BlockHeader,
        db: &mut GeneralizedDatabase,
    ) -> Option<ExecutionReport> {
        let report = generic_system_contract_levm(
            block_header,
            Bytes::new(),
            db,
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
        db: &mut GeneralizedDatabase,
    ) -> Option<ExecutionReport> {
        let report = generic_system_contract_levm(
            block_header,
            Bytes::new(),
            db,
            *CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS,
            *SYSTEM_ADDRESS,
        )
        .ok()?;

        match report.result {
            TxResult::Success => Some(report),
            _ => None,
        }
    }

    pub fn create_access_list(
        mut tx: GenericTransaction,
        header: &BlockHeader,
        db: &mut GeneralizedDatabase,
    ) -> Result<(ExecutionResult, AccessList), VMError> {
        let mut env = env_from_generic(&tx, header, db)?;

        adjust_disabled_base_fee(&mut env);

        let mut vm = vm_from_generic(&tx, env.clone(), db)?;

        vm.stateless_execute()?;
        let access_list = build_access_list(&vm.accrued_substate);

        // Execute the tx again, now with the created access list.
        tx.access_list = access_list.iter().map(|item| item.into()).collect();
        let mut vm = vm_from_generic(&tx, env.clone(), db)?;

        let report = vm.stateless_execute()?;

        Ok((report.into(), access_list))
    }
}

pub fn generic_system_contract_levm(
    block_header: &BlockHeader,
    calldata: Bytes,
    db: &mut GeneralizedDatabase,
    contract_address: Address,
    system_address: Address,
) -> Result<ExecutionReport, EvmError> {
    let chain_config = db.store.get_chain_config();
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
        db,
        vec![],
        None,
    )
    .map_err(EvmError::from)?;

    let report = vm.execute().map_err(EvmError::from)?;
    db.cache.remove(&system_address);

    Ok(report)
}

#[allow(unreachable_code)]
#[allow(unused_variables)]
pub fn extract_all_requests_levm(
    receipts: &[Receipt],
    db: &mut GeneralizedDatabase,
    header: &BlockHeader,
) -> Result<Vec<Requests>, EvmError> {
    let chain_config = db.store.get_chain_config();
    let fork = chain_config.fork(header.timestamp);

    if fork < Fork::Prague {
        return Ok(Default::default());
    }

    cfg_if::cfg_if! {
        if #[cfg(feature = "l2")] {
            return Ok(Default::default());
        }
    }

    let withdrawals_data: Vec<u8> = match LEVM::read_withdrawal_requests(header, db) {
        Some(report) => {
            // the cache is updated inside the generic_system_call
            report.output.into()
        }
        None => Default::default(),
    };

    let consolidation_data: Vec<u8> = match LEVM::dequeue_consolidation_requests(header, db) {
        Some(report) => {
            // the cache is updated inside the generic_system_call
            report.output.into()
        }
        None => Default::default(),
    };

    let deposits = Requests::from_deposit_receipts(chain_config.deposit_contract_address, receipts);
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

pub fn build_access_list(substate: &Substate) -> AccessList {
    let access_list: AccessList = substate
        .touched_storage_slots
        .iter()
        .map(|(address, slots)| (*address, slots.iter().cloned().collect::<Vec<H256>>()))
        .collect();

    access_list
}

fn env_from_generic(
    tx: &GenericTransaction,
    header: &BlockHeader,
    db: &GeneralizedDatabase,
) -> Result<Environment, VMError> {
    let chain_config = db.store.get_chain_config();
    let gas_price = calculate_gas_price(tx, header.base_fee_per_gas.unwrap_or(INITIAL_BASE_FEE));
    let config = EVMConfig::new_from_chain_config(&chain_config, header);
    Ok(Environment {
        origin: tx.from.0.into(),
        refunded_gas: 0,
        gas_limit: tx.gas.unwrap_or(header.gas_limit), // Ensure tx doesn't fail due to gas limit
        config,
        block_number: header.number.into(),
        coinbase: header.coinbase,
        timestamp: header.timestamp.into(),
        prev_randao: Some(header.prev_randao),
        chain_id: tx.chain_id.unwrap_or(chain_config.chain_id).into(),
        base_fee_per_gas: header.base_fee_per_gas.unwrap_or_default().into(),
        gas_price,
        block_excess_blob_gas: header.excess_blob_gas.map(U256::from),
        block_blob_gas_used: header.blob_gas_used.map(U256::from),
        tx_blob_hashes: tx.blob_versioned_hashes.clone(),
        tx_max_priority_fee_per_gas: tx.max_priority_fee_per_gas.map(U256::from),
        tx_max_fee_per_gas: tx.max_fee_per_gas.map(U256::from),
        tx_max_fee_per_blob_gas: tx.max_fee_per_blob_gas,
        tx_nonce: tx.nonce.unwrap_or_default(),
        block_gas_limit: header.gas_limit,
        transient_storage: HashMap::new(),
        difficulty: header.difficulty,
    })
}

fn vm_from_generic<'a>(
    tx: &GenericTransaction,
    env: Environment,
    db: &'a mut GeneralizedDatabase,
) -> Result<VM<'a>, VMError> {
    VM::new(
        tx.to.clone(),
        env,
        tx.value,
        tx.input.clone(),
        db,
        tx.access_list
            .iter()
            .map(|list| (list.address, list.storage_keys.clone()))
            .collect(),
        tx.authorization_list.clone().map(|list| {
            list.iter()
                .map(|list| Into::<AuthorizationTuple>::into(list.clone()))
                .collect()
        }),
    )
}
