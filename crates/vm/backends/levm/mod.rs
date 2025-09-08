pub mod db;
mod tracing;

use super::BlockExecutionResult;
use crate::constants::{
    BEACON_ROOTS_ADDRESS, CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS, HISTORY_STORAGE_ADDRESS,
    SYSTEM_ADDRESS, WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
};
use crate::{DynVmDatabase, Evm, EvmError, ExecutionResult, VmDatabase};
use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    types::{
        AccessList, AccountUpdate, AuthorizationTuple, Block, BlockHeader, EIP1559Transaction,
        EIP7702Transaction, Fork, GWEI_TO_WEI, GenericTransaction, INITIAL_BASE_FEE, Receipt,
        Transaction, TxKind, Withdrawal, requests::Requests,
    },
};
use ethrex_levm::EVMConfig;
use ethrex_levm::constants::{SYS_CALL_GAS_LIMIT, TX_BASE_COST};
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::errors::{InternalError, TxValidationError};
use ethrex_levm::tracing::LevmCallTracer;
use ethrex_levm::vm::VMType;
use ethrex_levm::{
    Environment,
    errors::{ExecutionReport, TxResult, VMError},
    vm::{Substate, VM},
};
use std::cmp::min;
use std::sync::Arc;

/// The struct implements the following functions:
/// [LEVM::execute_block]
/// [LEVM::execute_tx]
/// [LEVM::get_state_transitions]
/// [LEVM::process_withdrawals]
pub struct LEVM {
    pub db: GeneralizedDatabase,
    pub vm_type: VMType,
}

impl Evm for LEVM {
    fn new_for_l1(db: impl VmDatabase + 'static) -> Self {
        let wrapped_db: DynVmDatabase = Box::new(db);
        LEVM {
            db: GeneralizedDatabase::new(Arc::new(wrapped_db)),
            vm_type: VMType::L1,
        }
    }

    fn new_for_l2(db: impl VmDatabase + 'static) -> Self {
        let wrapped_db: DynVmDatabase = Box::new(db);
        LEVM {
            db: GeneralizedDatabase::new(Arc::new(wrapped_db)),
            vm_type: VMType::L2,
        }
    }

    fn execute_tx(
        &mut self,
        tx: &Transaction,
        block_header: &BlockHeader,
        remaining_gas: &mut u64,
        sender: Address,
    ) -> Result<(Receipt, u64), EvmError> {
        let execution_report = self.execute_tx_aux(tx, sender, block_header)?;

        *remaining_gas = remaining_gas.saturating_sub(execution_report.gas_used);

        let receipt = Receipt::new(
            tx.tx_type(),
            execution_report.is_success(),
            block_header.gas_limit - *remaining_gas,
            execution_report.logs.clone(),
        );

        Ok((receipt, execution_report.gas_used))
    }
    fn execute_block(&mut self, block: &Block) -> Result<BlockExecutionResult, EvmError> {
        self.prepare_block(block)?;

        let mut receipts = Vec::new();
        let mut cumulative_gas_used = 0;

        for (tx, tx_sender) in block.body.get_transactions_with_sender().map_err(|error| {
            EvmError::Transaction(format!("Couldn't recover addresses with error: {error}"))
        })? {
            let report = self.execute_tx_aux(tx, tx_sender, &block.header)?;

            cumulative_gas_used += report.gas_used;
            let receipt = Receipt::new(
                tx.tx_type(),
                matches!(report.result.clone(), TxResult::Success),
                cumulative_gas_used,
                report.logs.clone(),
            );

            receipts.push(receipt);
        }

        if let Some(withdrawals) = &block.body.withdrawals {
            self.process_withdrawals(withdrawals)?;
        }

        // TODO: I don't like deciding the behavior based on the VMType here.
        // TODO2: Revise this, apparently extract_all_requests_levm is not called
        // in L2 execution, but its implementation behaves differently based on this.
        let requests = match self.vm_type {
            VMType::L1 => self.extract_requests(&receipts, &block.header)?,
            VMType::L2 => Default::default(),
        };

        Ok(BlockExecutionResult { receipts, requests })
    }

    fn undo_last_tx(&mut self) -> Result<(), EvmError> {
        self.db.undo_last_transaction()?;
        Ok(())
    }

    fn simulate_tx_from_generic(
        &mut self,
        // The transaction to execute.
        tx: &GenericTransaction,
        // The block header for the current block.
        block_header: &BlockHeader,
        _fork: Fork,
    ) -> Result<ExecutionResult, EvmError> {
        let mut env = env_from_generic(tx, block_header, &self.db)?;

        env.block_gas_limit = u64::MAX; // disable block gas limit

        adjust_disabled_base_fee(&mut env);

        let mut vm = self.vm_from_generic(tx, env)?;

        vm.execute()
            .map(|value| value.into())
            .map_err(VMError::into)
    }

    fn get_state_transitions(&mut self) -> Result<Vec<AccountUpdate>, EvmError> {
        Ok(self.db.get_state_transitions()?)
    }

    fn process_withdrawals(&mut self, withdrawals: &[Withdrawal]) -> Result<(), EvmError> {
        // For every withdrawal we increment the target account's balance
        for (address, increment) in withdrawals
            .iter()
            .filter(|withdrawal| withdrawal.amount > 0)
            .map(|w| (w.address, u128::from(w.amount) * u128::from(GWEI_TO_WEI)))
        {
            let account = self
                .db
                .get_account_mut(address)
                .map_err(|_| EvmError::DB(format!("Withdrawal account {address} not found")))?;

            account.info.balance += increment.into();
        }
        Ok(())
    }

    fn create_access_list_inner(
        &mut self,
        tx: &GenericTransaction,
        header: &BlockHeader,
        _fork: Fork,
    ) -> Result<(ExecutionResult, AccessList), EvmError> {
        let mut env = env_from_generic(&tx, header, &mut self.db)?;

        adjust_disabled_base_fee(&mut env);

        let mut vm = self.vm_from_generic(&tx, env.clone())?;

        vm.stateless_execute()?;
        let access_list = build_access_list(&vm.substate);

        // Execute the tx again, now with the created access list.
        let mut tx_with_list = tx.clone();
        tx_with_list.access_list = access_list.iter().map(|item| item.into()).collect();
        let mut vm = self.vm_from_generic(&tx, env.clone())?;

        let report = vm.stateless_execute()?;

        Ok((report.into(), access_list))
    }

    #[allow(unreachable_code)]
    #[allow(unused_variables)]
    fn extract_requests(
        &mut self,
        receipts: &[Receipt],
        header: &BlockHeader,
    ) -> Result<Vec<Requests>, EvmError> {
        if let VMType::L2 = self.vm_type {
            return Err(EvmError::InvalidEVM(
                "extract_all_requests_levm should not be called for L2 VM".to_string(),
            ));
        }

        let chain_config = self.db.store.get_chain_config()?;
        let fork = chain_config.fork(header.timestamp);

        if fork < Fork::Prague {
            return Ok(Default::default());
        }

        let withdrawals_data: Vec<u8> = self.read_withdrawal_requests(header)?.output.into();
        let consolidation_data: Vec<u8> =
            self.dequeue_consolidation_requests(header)?.output.into();

        let deposits =
            Requests::from_deposit_receipts(chain_config.deposit_contract_address, receipts)
                .ok_or(EvmError::InvalidDepositRequest)?;
        let withdrawals = Requests::from_withdrawals_data(withdrawals_data);
        let consolidation = Requests::from_consolidation_data(consolidation_data);

        Ok(vec![deposits, withdrawals, consolidation])
    }

    fn apply_system_calls(&mut self, block_header: &BlockHeader) -> Result<(), EvmError> {
        let chain_config = self.db.store.get_chain_config()?;
        let fork = chain_config.fork(block_header.timestamp);

        if block_header.parent_beacon_block_root.is_some() && fork >= Fork::Cancun {
            self.beacon_root_contract_call(block_header)?;
        }

        if fork >= Fork::Prague {
            self.process_block_hash_history(block_header)?;
        }

        Ok(())
    }
}

impl LEVM {
    fn setup_env(
        &mut self,
        tx: &Transaction,
        tx_sender: Address,
        block_header: &BlockHeader,
    ) -> Result<Environment, EvmError> {
        let chain_config = self.db.store.get_chain_config()?;
        let gas_price: U256 = tx
            .effective_gas_price(block_header.base_fee_per_gas)
            .ok_or(VMError::TxValidation(
                TxValidationError::InsufficientMaxFeePerGas,
            ))?
            .into();

        let config = EVMConfig::new_from_chain_config(&chain_config, block_header);
        let env = Environment {
            origin: tx_sender,
            gas_limit: tx.gas_limit(),
            config,
            block_number: block_header.number.into(),
            coinbase: block_header.coinbase,
            timestamp: block_header.timestamp.into(),
            prev_randao: Some(block_header.prev_randao),
            chain_id: chain_config.chain_id.into(),
            base_fee_per_gas: block_header.base_fee_per_gas.unwrap_or_default().into(),
            gas_price,
            block_excess_blob_gas: block_header.excess_blob_gas.map(U256::from),
            block_blob_gas_used: block_header.blob_gas_used.map(U256::from),
            tx_blob_hashes: tx.blob_versioned_hashes(),
            tx_max_priority_fee_per_gas: tx.max_priority_fee().map(U256::from),
            tx_max_fee_per_gas: tx.max_fee_per_gas().map(U256::from),
            tx_max_fee_per_blob_gas: tx.max_fee_per_blob_gas(),
            tx_nonce: tx.nonce(),
            block_gas_limit: block_header.gas_limit,
            difficulty: block_header.difficulty,
            is_privileged: matches!(tx, Transaction::PrivilegedL2Transaction(_)),
        };

        Ok(env)
    }

    pub fn execute_tx_aux(
        &mut self,
        // The transaction to execute.
        tx: &Transaction,
        // The transactions recovered address
        tx_sender: Address,
        // The block header for the current block.
        block_header: &BlockHeader,
    ) -> Result<ExecutionReport, EvmError> {
        let env = self.setup_env(tx, tx_sender, block_header)?;
        let db = &mut self.db;
        let mut vm = VM::new(env, db, tx, LevmCallTracer::disabled(), self.vm_type)?;

        vm.execute().map_err(VMError::into)
    }

    pub fn prepare_block(&mut self, block: &Block) -> Result<(), EvmError> {
        let chain_config = self.db.store.get_chain_config()?;
        let block_header = &block.header;
        let fork = chain_config.fork(block_header.timestamp);

        // TODO: I don't like deciding the behavior based on the VMType here.
        if let VMType::L2 = self.vm_type {
            return Ok(());
        }

        if block_header.parent_beacon_block_root.is_some() && fork >= Fork::Cancun {
            self.beacon_root_contract_call(block_header)?;
        }

        if fork >= Fork::Prague {
            //eip 2935: stores parent block hash in system contract
            self.process_block_hash_history(block_header)?;
        }
        Ok(())
    }

    // SYSTEM CONTRACTS
    pub fn beacon_root_contract_call(
        &mut self,
        block_header: &BlockHeader,
    ) -> Result<(), EvmError> {
        if let VMType::L2 = self.vm_type {
            return Err(EvmError::InvalidEVM(
                "beacon_root_contract_call should not be called for L2 VM".to_string(),
            ));
        }

        let beacon_root = block_header.parent_beacon_block_root.ok_or_else(|| {
            EvmError::Header("parent_beacon_block_root field is missing".to_string())
        })?;

        self.generic_system_contract_levm(
            block_header,
            Bytes::copy_from_slice(beacon_root.as_bytes()),
            *BEACON_ROOTS_ADDRESS,
            *SYSTEM_ADDRESS,
        )?;
        Ok(())
    }

    pub fn process_block_hash_history(
        &mut self,
        block_header: &BlockHeader,
    ) -> Result<(), EvmError> {
        if let VMType::L2 = self.vm_type {
            return Err(EvmError::InvalidEVM(
                "process_block_hash_history should not be called for L2 VM".to_string(),
            ));
        }

        self.generic_system_contract_levm(
            block_header,
            Bytes::copy_from_slice(block_header.parent_hash.as_bytes()),
            *HISTORY_STORAGE_ADDRESS,
            *SYSTEM_ADDRESS,
        )?;
        Ok(())
    }
    pub(crate) fn read_withdrawal_requests(
        &mut self,
        block_header: &BlockHeader,
    ) -> Result<ExecutionReport, EvmError> {
        if let VMType::L2 = self.vm_type {
            return Err(EvmError::InvalidEVM(
                "read_withdrawal_requests should not be called for L2 VM".to_string(),
            ));
        }

        let report = self.generic_system_contract_levm(
            block_header,
            Bytes::new(),
            *WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
            *SYSTEM_ADDRESS,
        )?;

        match report.result {
            TxResult::Success => Ok(report),
            // EIP-7002 specifies that a failed system call invalidates the entire block.
            TxResult::Revert(vm_error) => Err(EvmError::SystemContractCallFailed(format!(
                "REVERT when reading withdrawal requests with error: {vm_error:?}. According to EIP-7002, the revert of this system call invalidates the block.",
            ))),
        }
    }

    pub(crate) fn dequeue_consolidation_requests(
        &mut self,
        block_header: &BlockHeader,
    ) -> Result<ExecutionReport, EvmError> {
        if let VMType::L2 = self.vm_type {
            return Err(EvmError::InvalidEVM(
                "dequeue_consolidation_requests should not be called for L2 VM".to_string(),
            ));
        }

        let report = self.generic_system_contract_levm(
            block_header,
            Bytes::new(),
            *CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS,
            *SYSTEM_ADDRESS,
        )?;

        match report.result {
            TxResult::Success => Ok(report),
            // EIP-7251 specifies that a failed system call invalidates the entire block.
            TxResult::Revert(vm_error) => Err(EvmError::SystemContractCallFailed(format!(
                "REVERT when dequeuing consolidation requests with error: {vm_error:?}. According to EIP-7251, the revert of this system call invalidates the block.",
            ))),
        }
    }

    fn generic_system_contract_levm(
        &mut self,
        block_header: &BlockHeader,
        calldata: Bytes,
        contract_address: Address,
        system_address: Address,
    ) -> Result<ExecutionReport, EvmError> {
        let chain_config = self.db.store.get_chain_config()?;
        let config = EVMConfig::new_from_chain_config(&chain_config, block_header);
        let system_account_backup = self.db.current_accounts_state.get(&system_address).cloned();
        let coinbase_backup = self
            .db
            .current_accounts_state
            .get(&block_header.coinbase)
            .cloned();
        let env = Environment {
            origin: system_address,
            // EIPs 2935, 4788, 7002 and 7251 dictate that the system calls have a gas limit of 30 million and they do not use intrinsic gas.
            // So we add the base cost that will be taken in the execution.
            gas_limit: SYS_CALL_GAS_LIMIT + TX_BASE_COST,
            block_number: block_header.number.into(),
            coinbase: block_header.coinbase,
            timestamp: block_header.timestamp.into(),
            prev_randao: Some(block_header.prev_randao),
            base_fee_per_gas: U256::zero(),
            gas_price: U256::zero(),
            block_excess_blob_gas: block_header.excess_blob_gas.map(U256::from),
            block_blob_gas_used: block_header.blob_gas_used.map(U256::from),
            block_gas_limit: u64::MAX, // System calls, have no constraint on the block's gas limit.
            config,
            ..Default::default()
        };

        let tx = &Transaction::EIP1559Transaction(EIP1559Transaction {
            to: TxKind::Call(contract_address),
            value: U256::zero(),
            data: calldata,
            ..Default::default()
        });
        let mut vm = VM::new(
            env,
            &mut self.db,
            tx,
            LevmCallTracer::disabled(),
            self.vm_type,
        )
        .map_err(EvmError::from)?;

        let report = vm.execute().map_err(EvmError::from)?;

        if let Some(system_account) = system_account_backup {
            self.db
                .current_accounts_state
                .insert(system_address, system_account);
        } else {
            // If the system account was not in the cache, we need to remove it
            self.db.current_accounts_state.remove(&system_address);
        }

        if let Some(coinbase_account) = coinbase_backup {
            self.db
                .current_accounts_state
                .insert(block_header.coinbase, coinbase_account);
        } else {
            // If the coinbase account was not in the cache, we need to remove it
            self.db
                .current_accounts_state
                .remove(&block_header.coinbase);
        }

        Ok(report)
    }

    fn vm_from_generic(
        &mut self,
        tx: &GenericTransaction,
        env: Environment,
    ) -> Result<VM, VMError> {
        let tx = match &tx.authorization_list {
            Some(authorization_list) => Transaction::EIP7702Transaction(EIP7702Transaction {
                to: match tx.to {
                    TxKind::Call(to) => to,
                    TxKind::Create => {
                        return Err(InternalError::msg("Generic Tx cannot be create type").into());
                    }
                },
                value: tx.value,
                data: tx.input.clone(),
                access_list: tx
                    .access_list
                    .iter()
                    .map(|list| (list.address, list.storage_keys.clone()))
                    .collect(),
                authorization_list: authorization_list
                    .iter()
                    .map(|auth| Into::<AuthorizationTuple>::into(auth.clone()))
                    .collect(),
                ..Default::default()
            }),
            None => Transaction::EIP1559Transaction(EIP1559Transaction {
                to: tx.to.clone(),
                value: tx.value,
                data: tx.input.clone(),
                access_list: tx
                    .access_list
                    .iter()
                    .map(|list| (list.address, list.storage_keys.clone()))
                    .collect(),
                ..Default::default()
            }),
        };
        VM::new(
            env,
            &mut self.db,
            &tx,
            LevmCallTracer::disabled(),
            self.vm_type,
        )
    }
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
        .accessed_storage_slots
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
    let chain_config = db.store.get_chain_config()?;
    let gas_price = calculate_gas_price(tx, header.base_fee_per_gas.unwrap_or(INITIAL_BASE_FEE));
    let config = EVMConfig::new_from_chain_config(&chain_config, header);
    Ok(Environment {
        origin: tx.from.0.into(),
        gas_limit: tx.gas.unwrap_or(header.gas_limit), // Ensure tx doesn't fail due to gas limit
        config,
        block_number: header.number.into(),
        coinbase: header.coinbase,
        timestamp: header.timestamp.into(),
        prev_randao: Some(header.prev_randao),
        chain_id: chain_config.chain_id.into(),
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
        difficulty: header.difficulty,
        is_privileged: false,
    })
}

fn vm_from_generic<'a>(
    tx: &GenericTransaction,
    env: Environment,
    db: &'a mut GeneralizedDatabase,
    vm_type: VMType,
) -> Result<VM<'a>, VMError> {
    let tx = match &tx.authorization_list {
        Some(authorization_list) => Transaction::EIP7702Transaction(EIP7702Transaction {
            to: match tx.to {
                TxKind::Call(to) => to,
                TxKind::Create => {
                    return Err(InternalError::msg("Generic Tx cannot be create type").into());
                }
            },
            value: tx.value,
            data: tx.input.clone(),
            access_list: tx
                .access_list
                .iter()
                .map(|list| (list.address, list.storage_keys.clone()))
                .collect(),
            authorization_list: authorization_list
                .iter()
                .map(|auth| Into::<AuthorizationTuple>::into(auth.clone()))
                .collect(),
            ..Default::default()
        }),
        None => Transaction::EIP1559Transaction(EIP1559Transaction {
            to: tx.to.clone(),
            value: tx.value,
            data: tx.input.clone(),
            access_list: tx
                .access_list
                .iter()
                .map(|list| (list.address, list.storage_keys.clone()))
                .collect(),
            ..Default::default()
        }),
    };
    VM::new(env, db, &tx, LevmCallTracer::disabled(), vm_type)
}
