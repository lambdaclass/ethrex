pub mod db;
pub mod errors;
pub mod execution_db;
mod execution_result;
#[cfg(feature = "l2")]
mod mods;

use db::StoreWrapper;
use execution_db::ExecutionDB;
use std::cmp::min;

use ethrex_core::{
    types::{
        AccountInfo, Block, BlockHash, BlockHeader, ChainConfig, Fork, GenericTransaction,
        PrivilegedTxType, Receipt, Transaction, TxKind, Withdrawal, GWEI_TO_WEI, INITIAL_BASE_FEE,
    },
    Address, BigEndianHash, H256, U256,
};
use ethrex_storage::{error::StoreError, AccountUpdate, Store};
use lazy_static::lazy_static;
use revm::{
    db::{states::bundle_state::BundleRetention, AccountState, AccountStatus},
    inspector_handle_register,
    inspectors::TracerEip3155,
    precompile::{PrecompileSpecId, Precompiles},
    primitives::{BlobExcessGasAndPrice, BlockEnv, TxEnv, B256},
    Database, DatabaseCommit, Evm,
};
use revm_inspectors::access_list::AccessListInspector;
// Rename imported types for clarity
use revm_primitives::{
    ruint::Uint, AccessList as RevmAccessList, AccessListItem, Bytes, FixedBytes,
    TxKind as RevmTxKind,
};
// Export needed types
pub use errors::EvmError;
pub use execution_result::*;
pub use revm::primitives::{Address as RevmAddress, SpecId, U256 as RevmU256};

type AccessList = Vec<(Address, Vec<H256>)>;

pub const WITHDRAWAL_MAGIC_DATA: &[u8] = b"burn";
pub const DEPOSIT_MAGIC_DATA: &[u8] = b"mint";

/// State used when running the EVM. The state can be represented with a [StoreWrapper] database, or
/// with a [ExecutionDB] in case we only want to store the necessary data for some particular
/// execution, for example when proving in L2 mode.
///
/// Encapsulates state behaviour to be agnostic to the evm implementation for crate users.
pub enum EvmState {
    Store(revm::db::State<StoreWrapper>),
    Execution(revm::db::CacheDB<ExecutionDB>),
}

impl EvmState {
    /// Get a reference to inner `Store` database
    pub fn database(&self) -> Option<&Store> {
        if let EvmState::Store(db) = self {
            Some(&db.database.store)
        } else {
            None
        }
    }

    /// Gets the stored chain config
    pub fn chain_config(&self) -> Result<ChainConfig, EvmError> {
        match self {
            EvmState::Store(db) => db.database.store.get_chain_config().map_err(EvmError::from),
            EvmState::Execution(db) => Ok(db.db.get_chain_config()),
        }
    }
}

impl From<ExecutionDB> for EvmState {
    fn from(value: ExecutionDB) -> Self {
        EvmState::Execution(revm::db::CacheDB::new(value))
    }
}

cfg_if::cfg_if! {
    if #[cfg(feature = "levm")] {
        use ethrex_levm::{
            db::{CacheDB, Database as LevmDatabase},
            errors::{TransactionReport, TxResult, VMError},
            vm::VM,
            Environment,
        };
        use std::{collections::HashMap, sync::Arc};
        use ethrex_core::types::code_hash;

        /// Executes all transactions in a block and returns their receipts.
        pub fn execute_block(
            block: &Block,
            state: &mut EvmState,
        ) -> Result<(Vec<Receipt>, Vec<AccountUpdate>), EvmError> {
            let block_header = &block.header;
            //eip 4788: execute beacon_root_contract_call before block transactions
            cfg_if::cfg_if! {
                if #[cfg(not(feature = "l2"))] {
                    let spec_id = spec_id(&state.chain_config()?, block_header.timestamp);
                    if block_header.parent_beacon_block_root.is_some() && spec_id == SpecId::CANCUN {
                        beacon_root_contract_call(state, block_header, spec_id)?;
                    }
                }
            }
            let mut receipts = Vec::new();
            let mut cumulative_gas_used = 0;

            let store_wrapper = Arc::new(StoreWrapper {
                store: state.database().unwrap().clone(),
                block_hash: block.header.parent_hash,
            });

            let mut account_updates: Vec<AccountUpdate> = vec![];

            for transaction in block.body.transactions.iter() {
                let result = execute_tx_levm(transaction, block_header, store_wrapper.clone()).unwrap();
                cumulative_gas_used += result.gas_used;
                let receipt = Receipt::new(
                    transaction.tx_type(),
                    matches!(result.result, TxResult::Success),
                    cumulative_gas_used,
                    result.logs,
                );
                receipts.push(receipt);

                for (address, account) in result.new_state {
                    let mut added_storage = HashMap::new();

                    for (key, value) in account.storage {
                        added_storage.insert(key, value.current_value);
                    }

                    let code = if account.info.bytecode.is_empty() {
                        None
                    } else {
                        Some(account.info.bytecode.clone())
                    };

                    let account_update = AccountUpdate {
                        address,
                        removed: false,
                        info: Some(AccountInfo {
                            code_hash: code_hash(&account.info.bytecode),
                            balance: account.info.balance,
                            nonce: account.info.nonce,
                        }),
                        code,
                        added_storage,
                    };

                    account_updates.push(account_update);
                }
            }

            if let Some(withdrawals) = &block.body.withdrawals {
                process_withdrawals(state, withdrawals)?;
            }

            Ok((receipts, account_updates))
        }

        pub fn execute_tx_levm(
            tx: &Transaction,
            block_header: &BlockHeader,
            db: Arc<dyn LevmDatabase>,
        ) -> Result<TransactionReport, VMError> {
            let gas_price : U256 = tx.effective_gas_price(block_header.base_fee_per_gas).ok_or(VMError::InvalidTransaction)?.into();

            let env = Environment {
                origin: tx.sender(),
                consumed_gas: U256::zero(),
                refunded_gas: U256::zero(),
                gas_limit: tx.gas_limit().into(),
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
                block_gas_limit: block_header.gas_limit.into(),
            };

            let mut vm = VM::new(
                tx.to(),
                env,
                tx.value(),
                tx.data().clone(),
                db,
                CacheDB::default(),
            )?;

            vm.transact()
        }
    } else if #[cfg(not(feature = "levm"))] {
        /// Executes all transactions in a block and returns their receipts.
        pub fn execute_block(block: &Block, state: &mut EvmState) -> Result<Vec<Receipt>, EvmError> {
            let block_header = &block.header;
            let spec_id = spec_id(&state.chain_config()?, block_header.timestamp);
            //eip 4788: execute beacon_root_contract_call before block transactions
            cfg_if::cfg_if! {
                if #[cfg(not(feature = "l2"))] {
                    //eip 4788: execute beacon_root_contract_call before block transactions
                    if block_header.parent_beacon_block_root.is_some() && spec_id == SpecId::CANCUN {
                        beacon_root_contract_call(state, block_header, spec_id)?;
                    }
                }
            }
            let mut receipts = Vec::new();
            let mut cumulative_gas_used = 0;

            for transaction in block.body.transactions.iter() {
                let result = execute_tx(transaction, block_header, state, spec_id)?;
                cumulative_gas_used += result.gas_used();
                let receipt = Receipt::new(
                    transaction.tx_type(),
                    result.is_success(),
                    cumulative_gas_used,
                    result.logs(),
                );
                receipts.push(receipt);
            }

            if let Some(withdrawals) = &block.body.withdrawals {
                process_withdrawals(state, withdrawals)?;
            }

            Ok(receipts)
        }
    }
}

// Executes a single tx, doesn't perform state transitions
pub fn execute_tx(
    tx: &Transaction,
    header: &BlockHeader,
    state: &mut EvmState,
    spec_id: SpecId,
) -> Result<ExecutionResult, EvmError> {
    let block_env = block_env(header);
    let tx_env = tx_env(tx);
    run_evm(tx_env, block_env, state, spec_id)
}

// Executes a single GenericTransaction, doesn't commit the result or perform state transitions
pub fn simulate_tx_from_generic(
    tx: &GenericTransaction,
    header: &BlockHeader,
    state: &mut EvmState,
    spec_id: SpecId,
) -> Result<ExecutionResult, EvmError> {
    let block_env = block_env(header);
    let tx_env = tx_env_from_generic(tx, header.base_fee_per_gas.unwrap_or(INITIAL_BASE_FEE));
    run_without_commit(tx_env, block_env, state, spec_id)
}

/// When basefee tracking is disabled  (ie. env.disable_base_fee = true; env.disable_block_gas_limit = true;)
/// and no gas prices were specified, lower the basefee to 0 to avoid breaking EVM invariants (basefee < feecap)
/// See https://github.com/ethereum/go-ethereum/blob/00294e9d28151122e955c7db4344f06724295ec5/core/vm/evm.go#L137
fn adjust_disabled_base_fee(
    block_env: &mut BlockEnv,
    tx_gas_price: Uint<256, 4>,
    tx_blob_gas_price: Option<Uint<256, 4>>,
) {
    if tx_gas_price == RevmU256::from(0) {
        block_env.basefee = RevmU256::from(0);
    }
    if tx_blob_gas_price.is_some_and(|v| v == RevmU256::from(0)) {
        block_env.blob_excess_gas_and_price = None;
    }
}

/// Runs EVM, doesn't perform state transitions, but stores them
fn run_evm(
    tx_env: TxEnv,
    block_env: BlockEnv,
    state: &mut EvmState,
    spec_id: SpecId,
) -> Result<ExecutionResult, EvmError> {
    let tx_result = {
        let chain_spec = state.chain_config()?;
        #[allow(unused_mut)]
        let mut evm_builder = Evm::builder()
            .with_block_env(block_env)
            .with_tx_env(tx_env)
            .modify_cfg_env(|cfg| cfg.chain_id = chain_spec.chain_id)
            .with_spec_id(spec_id)
            .with_external_context(
                TracerEip3155::new(Box::new(std::io::stderr())).without_summary(),
            );
        cfg_if::cfg_if! {
            if #[cfg(feature = "l2")] {
                use revm::{Handler, primitives::{CancunSpec, HandlerCfg}};
                use std::sync::Arc;

                evm_builder = evm_builder.with_handler({
                    let mut evm_handler = Handler::new(HandlerCfg::new(SpecId::LATEST));
                    evm_handler.pre_execution.deduct_caller = Arc::new(mods::deduct_caller::<CancunSpec, _, _>);
                    evm_handler.validation.tx_against_state = Arc::new(mods::validate_tx_against_state::<CancunSpec, _, _>);
                    evm_handler.execution.last_frame_return = Arc::new(mods::last_frame_return::<CancunSpec, _, _>);
                    // TODO: Override `end` function. We should deposit even if we revert.
                    // evm_handler.pre_execution.end
                    evm_handler
                });
            }
        }

        match state {
            EvmState::Store(db) => {
                let mut evm = evm_builder.with_db(db).build();
                evm.transact_commit().map_err(EvmError::from)?
            }
            EvmState::Execution(db) => {
                let mut evm = evm_builder.with_db(db).build();
                evm.transact_commit().map_err(EvmError::from)?
            }
        }
    };
    Ok(tx_result.into())
}

/// Runs the transaction and returns the access list and estimated gas use (when running the tx with said access list)
pub fn create_access_list(
    tx: &GenericTransaction,
    header: &BlockHeader,
    state: &mut EvmState,
    spec_id: SpecId,
) -> Result<(ExecutionResult, AccessList), EvmError> {
    let mut tx_env = tx_env_from_generic(tx, header.base_fee_per_gas.unwrap_or(INITIAL_BASE_FEE));
    let block_env = block_env(header);
    // Run tx with access list inspector

    let (execution_result, access_list) =
        create_access_list_inner(tx_env.clone(), block_env.clone(), state, spec_id)?;

    // Run the tx with the resulting access list and estimate its gas used
    let execution_result = if execution_result.is_success() {
        tx_env.access_list.extend(access_list.0.clone());

        run_without_commit(tx_env, block_env, state, spec_id)?
    } else {
        execution_result
    };
    let access_list: Vec<(Address, Vec<H256>)> = access_list
        .iter()
        .map(|item| {
            (
                Address::from_slice(item.address.0.as_slice()),
                item.storage_keys
                    .iter()
                    .map(|v| H256::from_slice(v.as_slice()))
                    .collect(),
            )
        })
        .collect();
    Ok((execution_result, access_list))
}

/// Runs the transaction and returns the access list for it
fn create_access_list_inner(
    tx_env: TxEnv,
    block_env: BlockEnv,
    state: &mut EvmState,
    spec_id: SpecId,
) -> Result<(ExecutionResult, RevmAccessList), EvmError> {
    let mut access_list_inspector = access_list_inspector(&tx_env, state, spec_id)?;
    #[allow(unused_mut)]
    let mut evm_builder = Evm::builder()
        .with_block_env(block_env)
        .with_tx_env(tx_env)
        .with_spec_id(spec_id)
        .modify_cfg_env(|env| {
            env.disable_base_fee = true;
            env.disable_block_gas_limit = true
        })
        .with_external_context(&mut access_list_inspector);

    let tx_result = {
        match state {
            EvmState::Store(db) => {
                let mut evm = evm_builder
                    .with_db(db)
                    .append_handler_register(inspector_handle_register)
                    .build();
                evm.transact().map_err(EvmError::from)?
            }
            EvmState::Execution(db) => {
                let mut evm = evm_builder
                    .with_db(db)
                    .append_handler_register(inspector_handle_register)
                    .build();
                evm.transact().map_err(EvmError::from)?
            }
        }
    };

    let access_list = access_list_inspector.into_access_list();
    Ok((tx_result.result.into(), access_list))
}

/// Runs the transaction and returns the result, but does not commit it.
fn run_without_commit(
    tx_env: TxEnv,
    mut block_env: BlockEnv,
    state: &mut EvmState,
    spec_id: SpecId,
) -> Result<ExecutionResult, EvmError> {
    adjust_disabled_base_fee(
        &mut block_env,
        tx_env.gas_price,
        tx_env.max_fee_per_blob_gas,
    );
    let chain_config = state.chain_config()?;
    #[allow(unused_mut)]
    let mut evm_builder = Evm::builder()
        .with_block_env(block_env)
        .with_tx_env(tx_env)
        .with_spec_id(spec_id)
        .modify_cfg_env(|env| {
            env.disable_base_fee = true;
            env.disable_block_gas_limit = true;
            env.chain_id = chain_config.chain_id;
        });
    let tx_result = match state {
        EvmState::Store(db) => {
            let mut evm = evm_builder.with_db(db).build();
            evm.transact().map_err(EvmError::from)?
        }
        EvmState::Execution(db) => {
            let mut evm = evm_builder.with_db(db).build();
            evm.transact().map_err(EvmError::from)?
        }
    };
    Ok(tx_result.result.into())
}

/// Merges transitions stored when executing transactions and returns the resulting account updates
/// Doesn't update the DB
pub fn get_state_transitions(state: &mut EvmState) -> Vec<AccountUpdate> {
    match state {
        EvmState::Store(db) => {
            db.merge_transitions(BundleRetention::PlainState);
            let bundle = db.take_bundle();

            // Update accounts
            let mut account_updates = Vec::new();
            for (address, account) in bundle.state() {
                if account.status.is_not_modified() {
                    continue;
                }
                let address = Address::from_slice(address.0.as_slice());
                // Remove account from DB if destroyed (Process DestroyedChanged as changed account)
                if matches!(
                    account.status,
                    AccountStatus::Destroyed | AccountStatus::DestroyedAgain
                ) {
                    account_updates.push(AccountUpdate::removed(address));
                    continue;
                }

                // If account is empty, do not add to the database
                if account
                    .account_info()
                    .is_some_and(|acc_info| acc_info.is_empty())
                {
                    continue;
                }

                // Apply account changes to DB
                let mut account_update = AccountUpdate::new(address);
                // If the account was changed then both original and current info will be present in the bundle account
                if account.is_info_changed() {
                    // Update account info in DB
                    if let Some(new_acc_info) = account.account_info() {
                        let code_hash = H256::from_slice(new_acc_info.code_hash.as_slice());
                        let account_info = AccountInfo {
                            code_hash,
                            balance: U256::from_little_endian(new_acc_info.balance.as_le_slice()),
                            nonce: new_acc_info.nonce,
                        };
                        account_update.info = Some(account_info);
                        if account.is_contract_changed() {
                            // Update code in db
                            if let Some(code) = new_acc_info.code {
                                account_update.code = Some(code.original_bytes().clone().0);
                            }
                        }
                    }
                }
                // Update account storage in DB
                for (key, slot) in account.storage.iter() {
                    if slot.is_changed() {
                        // TODO check if we need to remove the value from our db when value is zero
                        // if slot.present_value().is_zero() {
                        //     account_update.removed_keys.push(H256::from_uint(&U256::from_little_endian(key.as_le_slice())))
                        // }
                        account_update.added_storage.insert(
                            H256::from_uint(&U256::from_little_endian(key.as_le_slice())),
                            U256::from_little_endian(slot.present_value().as_le_slice()),
                        );
                    }
                }
                account_updates.push(account_update)
            }
            account_updates
        }
        EvmState::Execution(db) => {
            // Update accounts
            let mut account_updates = Vec::new();
            for (revm_address, account) in &db.accounts {
                if account.account_state == AccountState::None {
                    // EVM didn't interact with this account
                    continue;
                }

                let address = Address::from_slice(revm_address.0.as_slice());
                // Remove account from DB if destroyed
                if account.account_state == AccountState::NotExisting {
                    account_updates.push(AccountUpdate::removed(address));
                    continue;
                }

                // If account is empty, do not add to the database
                if account.info().is_some_and(|acc_info| acc_info.is_empty()) {
                    continue;
                }

                // Apply account changes to DB
                let mut account_update = AccountUpdate::new(address);
                // Update account info in DB
                if let Some(new_acc_info) = account.info() {
                    let code_hash = H256::from_slice(new_acc_info.code_hash.as_slice());

                    // If code changed, update
                    if matches!(db.db.accounts.get(revm_address), Some(account) if account.code_hash != code_hash)
                    {
                        account_update.code = new_acc_info
                            .code
                            .map(|code| bytes::Bytes::copy_from_slice(code.bytes_slice()));
                    }

                    let account_info = AccountInfo {
                        code_hash,
                        balance: U256::from_little_endian(new_acc_info.balance.as_le_slice()),
                        nonce: new_acc_info.nonce,
                    };
                    account_update.info = Some(account_info);
                }
                // Update account storage in DB
                for (key, slot) in account.storage.iter() {
                    // TODO check if we need to remove the value from our db when value is zero
                    // if slot.present_value().is_zero() {
                    //     account_update.removed_keys.push(H256::from_uint(&U256::from_little_endian(key.as_le_slice())))
                    // }
                    account_update.added_storage.insert(
                        H256::from_uint(&U256::from_little_endian(key.as_le_slice())),
                        U256::from_little_endian(slot.as_le_slice()),
                    );
                }
                account_updates.push(account_update)
            }
            account_updates
        }
    }
}

/// Processes a block's withdrawals, updating the account balances in the state
pub fn process_withdrawals(
    state: &mut EvmState,
    withdrawals: &[Withdrawal],
) -> Result<(), StoreError> {
    match state {
        EvmState::Store(db) => {
            //balance_increments is a vector of tuples (Address, increment as u128)
            let balance_increments = withdrawals
                .iter()
                .filter(|withdrawal| withdrawal.amount > 0)
                .map(|withdrawal| {
                    (
                        RevmAddress::from_slice(withdrawal.address.as_bytes()),
                        (withdrawal.amount as u128 * GWEI_TO_WEI as u128),
                    )
                })
                .collect::<Vec<_>>();

            db.increment_balances(balance_increments)?;
        }
        EvmState::Execution(_) => {
            // TODO: We should check withdrawals are valid
            // (by checking that accounts exist if this is the only error) but there's no state to
            // change.
        }
    }
    Ok(())
}

/// Builds EvmState from a Store
pub fn evm_state(store: Store, block_hash: BlockHash) -> EvmState {
    EvmState::Store(
        revm::db::State::builder()
            .with_database(StoreWrapper { store, block_hash })
            .with_bundle_update()
            .without_state_clear()
            .build(),
    )
}

/// Calls the eip4788 beacon block root system call contract
/// As of the Cancun hard-fork, parent_beacon_block_root needs to be present in the block header.
pub fn beacon_root_contract_call(
    state: &mut EvmState,
    header: &BlockHeader,
    spec_id: SpecId,
) -> Result<ExecutionResult, EvmError> {
    lazy_static! {
        static ref SYSTEM_ADDRESS: RevmAddress = RevmAddress::from_slice(
            &hex::decode("fffffffffffffffffffffffffffffffffffffffe").unwrap()
        );
        static ref CONTRACT_ADDRESS: RevmAddress = RevmAddress::from_slice(
            &hex::decode("000F3df6D732807Ef1319fB7B8bB8522d0Beac02").unwrap(),
        );
    };
    let beacon_root = match header.parent_beacon_block_root {
        None => {
            return Err(EvmError::Header(
                "parent_beacon_block_root field is missing".to_string(),
            ))
        }
        Some(beacon_root) => beacon_root,
    };

    let tx_env = TxEnv {
        caller: *SYSTEM_ADDRESS,
        transact_to: RevmTxKind::Call(*CONTRACT_ADDRESS),
        gas_limit: 30_000_000,
        data: revm::primitives::Bytes::copy_from_slice(beacon_root.as_bytes()),
        ..Default::default()
    };
    let mut block_env = block_env(header);
    block_env.basefee = RevmU256::ZERO;
    block_env.gas_limit = RevmU256::from(30_000_000);

    match state {
        EvmState::Store(db) => {
            let mut evm = Evm::builder()
                .with_db(db)
                .with_block_env(block_env)
                .with_tx_env(tx_env)
                .with_spec_id(spec_id)
                .build();

            let transaction_result = evm.transact()?;
            let mut result_state = transaction_result.state;
            result_state.remove(&*SYSTEM_ADDRESS);
            result_state.remove(&evm.block().coinbase);

            evm.context.evm.db.commit(result_state);

            Ok(transaction_result.result.into())
        }
        EvmState::Execution(db) => {
            let mut evm = Evm::builder()
                .with_db(db)
                .with_block_env(block_env)
                .with_tx_env(tx_env)
                .with_spec_id(spec_id)
                .build();

            // Not necessary to commit to DB
            let transaction_result = evm.transact()?;
            Ok(transaction_result.result.into())
        }
    }
}

pub fn block_env(header: &BlockHeader) -> BlockEnv {
    BlockEnv {
        number: RevmU256::from(header.number),
        coinbase: RevmAddress(header.coinbase.0.into()),
        timestamp: RevmU256::from(header.timestamp),
        gas_limit: RevmU256::from(header.gas_limit),
        basefee: RevmU256::from(header.base_fee_per_gas.unwrap_or(INITIAL_BASE_FEE)),
        difficulty: RevmU256::from_limbs(header.difficulty.0),
        prevrandao: Some(header.prev_randao.as_fixed_bytes().into()),
        blob_excess_gas_and_price: Some(BlobExcessGasAndPrice::new(
            header.excess_blob_gas.unwrap_or_default(),
        )),
    }
}

pub fn tx_env(tx: &Transaction) -> TxEnv {
    let mut max_fee_per_blob_gas_bytes: [u8; 32] = [0; 32];
    let max_fee_per_blob_gas = match tx.max_fee_per_blob_gas() {
        Some(x) => {
            x.to_big_endian(&mut max_fee_per_blob_gas_bytes);
            Some(RevmU256::from_be_bytes(max_fee_per_blob_gas_bytes))
        }
        None => None,
    };
    TxEnv {
        caller: match tx {
            Transaction::PrivilegedL2Transaction(tx) if tx.tx_type == PrivilegedTxType::Deposit => {
                RevmAddress::ZERO
            }
            _ => RevmAddress(tx.sender().0.into()),
        },
        gas_limit: tx.gas_limit(),
        gas_price: RevmU256::from(tx.gas_price()),
        transact_to: match tx {
            Transaction::PrivilegedL2Transaction(tx)
                if tx.tx_type == PrivilegedTxType::Withdrawal =>
            {
                RevmTxKind::Call(RevmAddress::ZERO)
            }
            _ => match tx.to() {
                TxKind::Call(address) => RevmTxKind::Call(address.0.into()),
                TxKind::Create => RevmTxKind::Create,
            },
        },
        value: RevmU256::from_limbs(tx.value().0),
        data: match tx {
            Transaction::PrivilegedL2Transaction(tx) => match tx.tx_type {
                PrivilegedTxType::Deposit => DEPOSIT_MAGIC_DATA.into(),
                PrivilegedTxType::Withdrawal => {
                    let to = match tx.to {
                        TxKind::Call(to) => to,
                        _ => Address::zero(),
                    };
                    [Bytes::from(WITHDRAWAL_MAGIC_DATA), Bytes::from(to.0)]
                        .concat()
                        .into()
                }
            },
            _ => tx.data().clone().into(),
        },
        nonce: Some(tx.nonce()),
        chain_id: tx.chain_id(),
        access_list: tx
            .access_list()
            .into_iter()
            .map(|(addr, list)| {
                let (address, storage_keys) = (
                    RevmAddress(addr.0.into()),
                    list.into_iter()
                        .map(|a| FixedBytes::from_slice(a.as_bytes()))
                        .collect(),
                );
                AccessListItem {
                    address,
                    storage_keys,
                }
            })
            .collect(),
        gas_priority_fee: tx.max_priority_fee().map(RevmU256::from),
        blob_hashes: tx
            .blob_versioned_hashes()
            .into_iter()
            .map(|hash| B256::from(hash.0))
            .collect(),
        max_fee_per_blob_gas,
        // TODO revise
        // https://eips.ethereum.org/EIPS/eip-7702
        authorization_list: None,
    }
}

// Used to estimate gas and create access lists
fn tx_env_from_generic(tx: &GenericTransaction, basefee: u64) -> TxEnv {
    let gas_price = calculate_gas_price(tx, basefee);
    TxEnv {
        caller: RevmAddress(tx.from.0.into()),
        gas_limit: tx.gas.unwrap_or(u64::MAX), // Ensure tx doesn't fail due to gas limit
        gas_price,
        transact_to: match tx.to {
            TxKind::Call(address) => RevmTxKind::Call(address.0.into()),
            TxKind::Create => RevmTxKind::Create,
        },
        value: RevmU256::from_limbs(tx.value.0),
        data: tx.input.clone().into(),
        nonce: tx.nonce,
        chain_id: tx.chain_id,
        access_list: tx
            .access_list
            .iter()
            .map(|list| {
                let (address, storage_keys) = (
                    RevmAddress::from_slice(list.address.as_bytes()),
                    list.storage_keys
                        .iter()
                        .map(|a| FixedBytes::from_slice(a.as_bytes()))
                        .collect(),
                );
                AccessListItem {
                    address,
                    storage_keys,
                }
            })
            .collect(),
        gas_priority_fee: tx.max_priority_fee_per_gas.map(RevmU256::from),
        blob_hashes: tx
            .blob_versioned_hashes
            .iter()
            .map(|hash| B256::from(hash.0))
            .collect(),
        max_fee_per_blob_gas: tx.max_fee_per_blob_gas.map(|x| RevmU256::from_limbs(x.0)),
        // TODO revise
        // https://eips.ethereum.org/EIPS/eip-7702
        authorization_list: None,
    }
}

// Creates an AccessListInspector that will collect the accesses used by the evm execution
fn access_list_inspector(
    tx_env: &TxEnv,
    state: &mut EvmState,
    spec_id: SpecId,
) -> Result<AccessListInspector, EvmError> {
    // Access list provided by the transaction
    let current_access_list = RevmAccessList(tx_env.access_list.clone());
    // Addresses accessed when using precompiles
    let precompile_addresses = Precompiles::new(PrecompileSpecId::from_spec_id(spec_id))
        .addresses()
        .cloned();
    // Address that is either called or created by the transaction
    let to = match tx_env.transact_to {
        RevmTxKind::Call(address) => address,
        RevmTxKind::Create => {
            let nonce = match state {
                EvmState::Store(db) => db.basic(tx_env.caller)?,
                EvmState::Execution(db) => db.basic(tx_env.caller)?,
            }
            .map(|info| info.nonce)
            .unwrap_or_default();
            tx_env.caller.create(nonce)
        }
    };
    Ok(AccessListInspector::new(
        current_access_list,
        tx_env.caller,
        to,
        precompile_addresses,
    ))
}

/// Returns the spec id according to the block timestamp and the stored chain config
/// WARNING: Assumes at least Merge fork is active
pub fn spec_id(chain_config: &ChainConfig, block_timestamp: u64) -> SpecId {
    match chain_config.get_fork(block_timestamp) {
        Fork::Cancun => SpecId::CANCUN,
        Fork::Shanghai => SpecId::SHANGHAI,
        Fork::Paris => SpecId::MERGE,
    }
}

/// Calculating gas_price according to EIP-1559 rules
/// See https://github.com/ethereum/go-ethereum/blob/7ee9a6e89f59cee21b5852f5f6ffa2bcfc05a25f/internal/ethapi/transaction_args.go#L430
fn calculate_gas_price(tx: &GenericTransaction, basefee: u64) -> Uint<256, 4> {
    if tx.gas_price != 0 {
        // Legacy gas field was specified, use it
        RevmU256::from(tx.gas_price)
    } else {
        // Backfill the legacy gas price for EVM execution, (zero if max_fee_per_gas is zero)
        RevmU256::from(min(
            tx.max_priority_fee_per_gas.unwrap_or(0) + basefee,
            tx.max_fee_per_gas.unwrap_or(0),
        ))
    }
}
