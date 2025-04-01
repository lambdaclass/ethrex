pub(crate) mod db;

use super::BlockExecutionResult;
use crate::constants::{
    BEACON_ROOTS_ADDRESS, CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS, HISTORY_STORAGE_ADDRESS,
    SYSTEM_ADDRESS, WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
};
use crate::{EvmError, ExecutionResult, StoreWrapper};
use bytes::Bytes;
use ethrex_common::types::requests::Requests;
use ethrex_common::types::{
    AccessList, Account, AuthorizationTuple, Fork, GenericTransaction, INITIAL_BASE_FEE,
};
use ethrex_common::{
    types::{
        code_hash, AccountInfo, Block, BlockHeader, ChainConfig, Receipt, Transaction, TxKind,
        Withdrawal, GWEI_TO_WEI,
    },
    Address, H256, U256,
};
use ethrex_levm::utils::get_account_mut_vm;
use ethrex_levm::vm::Substate;
use ethrex_levm::{
    db::Database as LevmDatabase,
    errors::{ExecutionReport, TxResult, VMError},
    vm::{EVMConfig, VM},
    Environment,
};
use ethrex_storage::{AccountUpdate, Store};
use std::cmp::min;
use std::{collections::HashMap, sync::Arc};

// Export needed types
pub use ethrex_levm::db::cache::CacheDB;
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
        chain_config: ChainConfig,
    ) -> Result<BlockExecutionResult, EvmError> {
        let mut block_cache: CacheDB = CacheDB::default();
        cfg_if::cfg_if! {
            if #[cfg(not(feature = "l2"))] {
                let block_header = &block.header;
                let fork = chain_config.fork(block_header.timestamp);
                if block_header.parent_beacon_block_root.is_some() && fork >= Fork::Cancun {
                    Self::beacon_root_contract_call(block_header, chain_config, db.clone(), &mut block_cache)?;
                }

                if fork >= Fork::Prague {
                    //eip 2935: stores parent block hash in system contract
                    Self::process_block_hash_history(block_header, chain_config, db.clone(), &mut block_cache)?;
                }
            }
        }

        // Account updates are initialized like this because of the beacon_root_contract_call, it is going to be empty if it wasn't called.
        // Here we get the state_transitions from the db and then we get the state_transitions from the cache_db.
        let mut account_updates = Self::get_state_transitions(
            None,
            db.clone(),
            chain_config,
            &block.header,
            &block_cache,
        )?;
        let mut receipts = Vec::new();
        let mut cumulative_gas_used = 0;

        for (tx, tx_sender) in block.body.get_transactions_with_sender() {
            let report = Self::execute_tx(
                tx,
                tx_sender,
                &block.header,
                db.clone(),
                block_cache.clone(),
                &chain_config,
            )
            .map_err(EvmError::from)?;

            let mut new_state = report.new_state.clone();
            // Now original_value is going to be the same as the current_value, for the next transaction.
            // It should have only one value but it is convenient to keep on using our CacheDB structure
            for (address, _account) in new_state.cached_accounts.iter_mut() {
                if let Some(account_storage) = new_state.cached_storages.get_mut(address) {
                    for storage_slot in account_storage.values_mut() {
                        storage_slot.original_value = storage_slot.current_value;
                    }
                }
            }

            block_cache.extend_cache(new_state);

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
                let (account, _storage) =
                    get_account_mut_vm(&mut block_cache, db.clone(), address)?;
                account.info.balance += increment.into();
            }
        }

        let requests = extract_all_requests_levm(
            &receipts,
            db.clone(),
            chain_config,
            &block.header,
            &mut block_cache,
        )?;

        account_updates.extend(Self::get_state_transitions(
            None,
            db.clone(),
            chain_config,
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
        // The transactions recovered address
        tx_sender: Address,
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
        // EVM StoreWrapper database.
        store: &StoreWrapper,
        // A cache database for intermediate state changes during execution.
        block_cache: CacheDB,
    ) -> Result<ExecutionResult, EvmError> {
        let mut env = env_from_generic(tx, block_header, store)?;

        env.block_gas_limit = u64::MAX; // disable block gas limit

        adjust_disabled_base_fee(&mut env);

        let mut vm = vm_from_generic(tx, env, store, block_cache.clone())?;

        vm.execute()
            .map(|value| value.into())
            .map_err(VMError::into)
    }

    pub fn get_state_transitions(
        // Warning only pass the fork if running the ef-tests.
        // ISSUE #2021: https://github.com/lambdaclass/ethrex/issues/2021
        ef_tests: Option<Fork>,
        db: Arc<dyn LevmDatabase>,
        chain_config: ChainConfig,
        block_header: &BlockHeader,
        new_state: &CacheDB,
    ) -> Result<Vec<AccountUpdate>, EvmError> {
        let mut account_updates: Vec<AccountUpdate> = vec![];
        for (new_state_account_address, new_state_account) in &new_state.cached_accounts {
            let initial_account_state = db.get_account(*new_state_account_address);
            let mut updates = 0;
            if initial_account_state.info.balance != new_state_account.info.balance {
                updates += 1;
            }
            if initial_account_state.info.nonce != new_state_account.info.nonce {
                updates += 1;
            }
            let code = if new_state_account.code.is_empty() {
                // The new state account has no code
                None
            } else {
                // Look into the current database to see if the bytecode hash is already present
                let current_bytecode = &initial_account_state.code;
                let code = new_state_account.code.clone();
                // The code is present in the current database
                if *current_bytecode != Bytes::new() {
                    if *current_bytecode != code {
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
            if let Some((_account, storage)) = new_state.get_account(new_state_account_address) {
                for (key, value) in storage {
                    added_storage.insert(*key, value.current_value);
                    updates += 1;
                }
            }

            if updates == 0 && !new_state_account.is_empty() {
                continue;
            }

            let account_update = AccountUpdate {
                address: *new_state_account_address,
                removed: new_state_account.is_empty(),
                info: Some(AccountInfo {
                    code_hash: code_hash(&new_state_account.code),
                    balance: new_state_account.info.balance,
                    nonce: new_state_account.info.nonce,
                }),
                code,
                added_storage,
            };

            let fork_from_config = chain_config.fork(block_header.timestamp);
            // Here we take the passed fork through the ef_tests variable, or we set it to the fork based on the timestamp.
            let fork = ef_tests.unwrap_or(fork_from_config);
            // https://eips.ethereum.org/EIPS/eip-161
            // if an account was empty and is now empty, after spurious dragon, it should be removed
            if account_update.removed
                && initial_account_state.info.balance.is_zero()
                && initial_account_state.info.nonce == 0
                && initial_account_state.info.code_hash == code_hash(&Bytes::new())
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
            let mut account = block_cache
                .cached_accounts
                .get(&address)
                .cloned()
                .unwrap_or({
                    let acc_info = store
                        .get_account_info_by_hash(parent_hash, address)?
                        .unwrap_or_default();
                    let acc_code = store
                        .get_account_code(acc_info.code_hash)?
                        .unwrap_or_default();

                    // We handle storage updates separately
                    Account::new(acc_info.balance, acc_code, acc_info.nonce, HashMap::new())
                });

            account.info.balance += increment.into();
            block_cache.cached_accounts.insert(address, account);
        }
        Ok(())
    }

    // SYSTEM CONTRACTS
    /// `new_state` is being modified inside [generic_system_contract_levm].
    pub fn beacon_root_contract_call(
        block_header: &BlockHeader,
        chain_config: ChainConfig,
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
            chain_config,
            new_state,
            *BEACON_ROOTS_ADDRESS,
            *SYSTEM_ADDRESS,
        )?;
        Ok(())
    }
    /// `new_state` is being modified inside [generic_system_contract_levm].
    pub fn process_block_hash_history(
        block_header: &BlockHeader,
        chain_config: ChainConfig,
        db: Arc<dyn LevmDatabase>,
        new_state: &mut CacheDB,
    ) -> Result<(), EvmError> {
        generic_system_contract_levm(
            block_header,
            Bytes::copy_from_slice(block_header.parent_hash.as_bytes()),
            db.clone(),
            chain_config,
            new_state,
            *HISTORY_STORAGE_ADDRESS,
            *SYSTEM_ADDRESS,
        )?;
        Ok(())
    }
    pub(crate) fn read_withdrawal_requests(
        block_header: &BlockHeader,
        db: Arc<dyn LevmDatabase>,
        chain_config: ChainConfig,
        new_state: &mut CacheDB,
    ) -> Option<ExecutionReport> {
        let report = generic_system_contract_levm(
            block_header,
            Bytes::new(),
            db.clone(),
            chain_config,
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
        chain_config: ChainConfig,
        new_state: &mut CacheDB,
    ) -> Option<ExecutionReport> {
        let report = generic_system_contract_levm(
            block_header,
            Bytes::new(),
            db.clone(),
            chain_config,
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

    pub fn create_access_list(
        mut tx: GenericTransaction,
        header: &BlockHeader,
        store: &StoreWrapper,
        block_cache: &CacheDB,
    ) -> Result<(ExecutionResult, AccessList), VMError> {
        let mut env = env_from_generic(&tx, header, store)?;

        adjust_disabled_base_fee(&mut env);

        let mut vm = vm_from_generic(&tx, env.clone(), store, block_cache.clone())?;

        vm.execute()?;
        let access_list = build_access_list(&vm.accrued_substate);

        // Execute the tx again, now with the created access list.
        tx.access_list = access_list.iter().map(|item| item.into()).collect();
        let mut vm = vm_from_generic(&tx, env.clone(), store, block_cache.clone())?;

        let report = vm.execute()?;

        Ok((report.into(), access_list))
    }
}

/// `new_state` is being modified at the end.
pub fn generic_system_contract_levm(
    block_header: &BlockHeader,
    calldata: Bytes,
    db: Arc<dyn LevmDatabase>,
    chain_config: ChainConfig,
    new_state: &mut CacheDB,
    contract_address: Address,
    system_address: Address,
) -> Result<ExecutionReport, EvmError> {
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

    report.new_state.remove_account(&system_address);

    match report.result {
        TxResult::Success => {}
        _ => {
            return Err(EvmError::Custom(
                "ERROR in generic_system_contract_levm(). TX didn't succeed.".to_owned(),
            ))
        }
    }

    // new_state is a CacheDB coming from outside the function
    for (address, storage) in report.new_state.cached_storages.iter_mut() {
        if let Some(existing_storage) = new_state.cached_storages.get(address) {
            let mut existing_storage = existing_storage.clone();

            existing_storage.extend(storage.clone());
            *storage = existing_storage;
        }
    }

    for (address, account) in report.new_state.cached_accounts.iter_mut() {
        if let Some(existing_account) = new_state.cached_accounts.get(address) {
            account.info.balance = existing_account.info.balance;
        }
    }
    new_state.extend_cache(report.new_state.clone());

    Ok(report)
}

#[allow(unreachable_code)]
#[allow(unused_variables)]
pub fn extract_all_requests_levm(
    receipts: &[Receipt],
    db: Arc<dyn LevmDatabase>,
    chain_config: ChainConfig,
    header: &BlockHeader,
    cache: &mut CacheDB,
) -> Result<Vec<Requests>, EvmError> {
    let fork = chain_config.fork(header.timestamp);

    if fork < Fork::Prague {
        return Ok(Default::default());
    }

    cfg_if::cfg_if! {
        if #[cfg(feature = "l2")] {
            return Ok(Default::default());
        }
    }

    let withdrawals_data: Vec<u8> =
        match LEVM::read_withdrawal_requests(header, db.clone(), chain_config, cache) {
            Some(report) => {
                // the cache is updated inside the generic_system_call
                report.output.into()
            }
            None => Default::default(),
        };

    let consolidation_data: Vec<u8> =
        match LEVM::dequeue_consolidation_requests(header, db.clone(), chain_config, cache) {
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
    store: &StoreWrapper,
) -> Result<Environment, VMError> {
    let chain_config = &store
        .store
        .get_chain_config()
        .map_err(|_| VMError::FatalError)?;
    let gas_price = calculate_gas_price(tx, header.base_fee_per_gas.unwrap_or(INITIAL_BASE_FEE));
    let config = EVMConfig::new_from_chain_config(chain_config, header);
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

fn vm_from_generic(
    tx: &GenericTransaction,
    env: Environment,
    store: &StoreWrapper,
    block_cache: CacheDB,
) -> Result<VM, VMError> {
    VM::new(
        tx.to.clone(),
        env,
        tx.value,
        tx.input.clone(),
        Arc::new(store.clone()),
        block_cache,
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
