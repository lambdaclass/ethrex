use super::constants::{BEACON_ROOTS_ADDRESS_STR, HISTORY_STORAGE_ADDRESS_STR, SYSTEM_ADDRESS_STR};
use crate::spec_id;
use crate::EvmError;
use crate::EvmState;
use crate::ExecutionResult;
use ethrex_storage::error::StoreError;
use lazy_static::lazy_static;
use revm::{
    inspectors::TracerEip3155,
    precompile::{PrecompileSpecId, Precompiles},
    primitives::{BlobExcessGasAndPrice, BlockEnv, TxEnv, B256},
    Database, DatabaseCommit, Evm,
};
use revm_inspectors::access_list::AccessListInspector;
// Rename imported types for clarity
use ethrex_core::{
    types::{
        Block, BlockHeader, GenericTransaction, PrivilegedTxType, Receipt, Transaction, TxKind,
        Withdrawal, GWEI_TO_WEI, INITIAL_BASE_FEE,
    },
    Address,
};
use revm_primitives::{
    ruint::Uint, AccessList as RevmAccessList, AccessListItem, Address as RevmAddress,
    Authorization as RevmAuthorization, Bytes, FixedBytes, SignedAuthorization, SpecId,
    TxKind as RevmTxKind, U256 as RevmU256,
};
use std::cmp::min;

#[cfg(feature = "l2")]
use crate::mods;

/// Executes all transactions in a block and returns their receipts.
pub fn execute_block(block: &Block, state: &mut EvmState) -> Result<Vec<Receipt>, EvmError> {
    let block_header = &block.header;
    let spec_id = spec_id(&state.chain_config()?, block_header.timestamp);
    //eip 4788: execute beacon_root_contract_call before block transactions
    cfg_if::cfg_if! {
        if #[cfg(not(feature = "l2"))] {
            //eip 4788: execute beacon_root_contract_call before block transactions
            if block_header.parent_beacon_block_root.is_some() && spec_id >= SpecId::CANCUN {
                beacon_root_contract_call(state, block_header, spec_id)?;
            }

            //eip 2935: stores parent block hash in system contract
            if spec_id >= SpecId::PRAGUE {
                process_block_hash_history(state, block_header, spec_id)?;
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

/// Runs the transaction and returns the result, but does not commit it.
pub fn run_without_commit(
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

// Used for the L2
pub const WITHDRAWAL_MAGIC_DATA: &[u8] = b"burn";
pub const DEPOSIT_MAGIC_DATA: &[u8] = b"mint";
pub fn tx_env(tx: &Transaction) -> TxEnv {
    let max_fee_per_blob_gas = tx
        .max_fee_per_blob_gas()
        .map(|x| RevmU256::from_be_bytes(x.to_big_endian()));
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
        // EIP7702
        // https://eips.ethereum.org/EIPS/eip-7702
        // The latest version of revm(19.3.0) is needed to run with the latest changes.
        // NOTE:
        // - rust 1.82.X is needed
        // - rust-toolchain 1.82.X is needed (this can be found in ethrex/crates/vm/levm/rust-toolchain.toml)
        authorization_list: tx.authorization_list().map(|list| {
            list.into_iter()
                .map(|auth_t| {
                    SignedAuthorization::new_unchecked(
                        RevmAuthorization {
                            chain_id: auth_t.chain_id.as_u64(),
                            address: RevmAddress(auth_t.address.0.into()),
                            nonce: auth_t.nonce,
                        },
                        auth_t.y_parity.as_u32() as u8,
                        RevmU256::from_le_bytes(auth_t.r_signature.to_little_endian()),
                        RevmU256::from_le_bytes(auth_t.s_signature.to_little_endian()),
                    )
                })
                .collect::<Vec<SignedAuthorization>>()
                .into()
        }),
    }
}

// Used to estimate gas and create access lists
pub(crate) fn tx_env_from_generic(tx: &GenericTransaction, basefee: u64) -> TxEnv {
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
        // EIP7702
        // https://eips.ethereum.org/EIPS/eip-7702
        // The latest version of revm(19.3.0) is needed to run with the latest changes.
        // NOTE:
        // - rust 1.82.X is needed
        // - rust-toolchain 1.82.X is needed (this can be found in ethrex/crates/vm/levm/rust-toolchain.toml)
        authorization_list: tx.authorization_list.clone().map(|list| {
            list.into_iter()
                .map(|auth_t| {
                    SignedAuthorization::new_unchecked(
                        RevmAuthorization {
                            //chain_id: RevmU256::from_le_bytes(auth_t.chain_id.to_little_endian()),
                            chain_id: auth_t.chain_id.as_u64(),
                            address: RevmAddress(auth_t.address.0.into()),
                            nonce: auth_t.nonce,
                        },
                        auth_t.y_parity.as_u32() as u8,
                        RevmU256::from_le_bytes(auth_t.r.to_little_endian()),
                        RevmU256::from_le_bytes(auth_t.s.to_little_endian()),
                    )
                })
                .collect::<Vec<SignedAuthorization>>()
                .into()
        }),
    }
}

// Creates an AccessListInspector that will collect the accesses used by the evm execution
pub(crate) fn access_list_inspector(
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

/// Calls the eip4788 beacon block root system call contract
/// As of the Cancun hard-fork, parent_beacon_block_root needs to be present in the block header.
pub fn beacon_root_contract_call(
    state: &mut EvmState,
    header: &BlockHeader,
    spec_id: SpecId,
) -> Result<ExecutionResult, EvmError> {
    lazy_static! {
        static ref SYSTEM_ADDRESS: RevmAddress =
            RevmAddress::from_slice(&hex::decode(SYSTEM_ADDRESS_STR).unwrap());
        static ref CONTRACT_ADDRESS: RevmAddress =
            RevmAddress::from_slice(&hex::decode(BEACON_ROOTS_ADDRESS_STR).unwrap());
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

/// Calls the EIP-2935 process block hashes history system call contract
/// NOTE: This was implemented by making use of an EVM system contract, but can be changed to a
/// direct state trie update after the verkle fork, as explained in https://eips.ethereum.org/EIPS/eip-2935
pub fn process_block_hash_history(
    state: &mut EvmState,
    header: &BlockHeader,
    spec_id: SpecId,
) -> Result<ExecutionResult, EvmError> {
    lazy_static! {
        static ref SYSTEM_ADDRESS: RevmAddress =
            RevmAddress::from_slice(&hex::decode(SYSTEM_ADDRESS_STR).unwrap());
        static ref CONTRACT_ADDRESS: RevmAddress =
            RevmAddress::from_slice(&hex::decode(HISTORY_STORAGE_ADDRESS_STR).unwrap(),);
    };
    let tx_env = TxEnv {
        caller: *SYSTEM_ADDRESS,
        transact_to: RevmTxKind::Call(*CONTRACT_ADDRESS),
        gas_limit: 30_000_000,
        data: revm::primitives::Bytes::copy_from_slice(header.parent_hash.as_bytes()),
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
