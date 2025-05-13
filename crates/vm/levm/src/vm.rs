use crate::{
    call_frame::{CallFrame, CallFrameBackup},
    constants::*,
    db::gen_db::GeneralizedDatabase,
    environment::Environment,
    errors::{ExecutionReport, InternalError, OpcodeResult, TxResult, VMError},
    hooks::hook::Hook,
    precompiles::{
        execute_precompile, is_precompile, SIZE_PRECOMPILES_CANCUN, SIZE_PRECOMPILES_PRAGUE,
        SIZE_PRECOMPILES_PRE_CANCUN,
    },
    utils::*,
    TransientStorage,
};
use bytes::Bytes;
use ethrex_common::{
    types::{
        tx_fields::{AccessList, AuthorizationList},
        BlockHeader, ChainConfig, Fork, ForkBlobSchedule, Transaction, TxKind,
    },
    Address, H256, U256,
};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fmt::Debug,
    sync::Arc,
};

#[cfg(not(feature = "l2"))]
use crate::hooks::DefaultHook;
#[cfg(feature = "l2")]
use {crate::hooks::L2Hook, ethrex_common::types::PrivilegedL2Transaction};

pub type Storage = HashMap<U256, H256>;

#[derive(Debug, Clone, Default)]
/// Information that changes during transaction execution
pub struct Substate {
    pub selfdestruct_set: HashSet<Address>,
    pub touched_accounts: HashSet<Address>,
    pub touched_storage_slots: HashMap<Address, BTreeSet<H256>>,
    pub created_accounts: HashSet<Address>,
}

/// Backup if sub-context is reverted. It consists of a copy of:
///   - Substate
///   - Gas Refunds
///   - Transient Storage
pub struct StateBackup {
    pub substate: Substate,
    pub refunded_gas: u64,
    pub transient_storage: TransientStorage,
}

impl StateBackup {
    pub fn new(
        substate: Substate,
        refunded_gas: u64,
        transient_storage: TransientStorage,
    ) -> StateBackup {
        StateBackup {
            substate,
            refunded_gas,
            transient_storage,
        }
    }
}

#[derive(Debug, Clone, Copy)]
/// This struct holds special configuration variables specific to the
/// EVM. In most cases, at least at the time of writing (February
/// 2025), you want to use the default blob_schedule values for the
/// specified Fork. The "intended" way to do this is by using the `EVMConfig::canonical_values(fork: Fork)` function.
///
/// However, that function should NOT be used IF you want to use a
/// custom `ForkBlobSchedule`, like it's described in [EIP-7840](https://eips.ethereum.org/EIPS/eip-7840)
/// Values are determined by [EIP-7691](https://eips.ethereum.org/EIPS/eip-7691#specification)
pub struct EVMConfig {
    pub fork: Fork,
    pub blob_schedule: ForkBlobSchedule,
}

impl EVMConfig {
    pub fn new(fork: Fork, blob_schedule: ForkBlobSchedule) -> EVMConfig {
        EVMConfig {
            fork,
            blob_schedule,
        }
    }

    pub fn new_from_chain_config(chain_config: &ChainConfig, block_header: &BlockHeader) -> Self {
        let fork = chain_config.fork(block_header.timestamp);

        let blob_schedule = chain_config
            .get_fork_blob_schedule(block_header.timestamp)
            .unwrap_or_else(|| EVMConfig::canonical_values(fork));

        EVMConfig::new(fork, blob_schedule)
    }

    /// This function is used for running the EF tests. If you don't
    /// have acces to a EVMConfig (mainly in the form of a
    /// genesis.json file) you can use this function to get the
    /// "Default" ForkBlobSchedule for that specific Fork.
    /// NOTE: This function could potentially be expanded to include
    /// other types of "default"s.
    pub fn canonical_values(fork: Fork) -> ForkBlobSchedule {
        let max_blobs_per_block: u64 = Self::max_blobs_per_block(fork);
        let target: u64 = Self::get_target_blob_gas_per_block_(fork);
        let base_fee_update_fraction: u64 = Self::get_blob_base_fee_update_fraction_value(fork);

        ForkBlobSchedule {
            target,
            max: max_blobs_per_block,
            base_fee_update_fraction,
        }
    }

    const fn max_blobs_per_block(fork: Fork) -> u64 {
        match fork {
            Fork::Prague => MAX_BLOB_COUNT_ELECTRA,
            Fork::Osaka => MAX_BLOB_COUNT_ELECTRA,
            _ => MAX_BLOB_COUNT,
        }
    }

    const fn get_blob_base_fee_update_fraction_value(fork: Fork) -> u64 {
        match fork {
            Fork::Prague | Fork::Osaka => BLOB_BASE_FEE_UPDATE_FRACTION_PRAGUE,
            _ => BLOB_BASE_FEE_UPDATE_FRACTION,
        }
    }

    const fn get_target_blob_gas_per_block_(fork: Fork) -> u64 {
        match fork {
            Fork::Prague | Fork::Osaka => TARGET_BLOB_GAS_PER_BLOCK_PECTRA,
            _ => TARGET_BLOB_GAS_PER_BLOCK,
        }
    }
}

impl Default for EVMConfig {
    /// The default EVMConfig depends on the default Fork.
    fn default() -> Self {
        let fork = core::default::Default::default();
        EVMConfig {
            fork,
            blob_schedule: Self::canonical_values(fork),
        }
    }
}

pub struct VM<'a> {
    pub call_frames: Vec<CallFrame>,
    pub env: Environment,
    pub accrued_substate: Substate,
    pub db: &'a mut GeneralizedDatabase,
    pub tx_kind: TxKind,
    pub access_list: AccessList,
    pub authorization_list: Option<AuthorizationList>,
    pub hooks: Vec<Arc<dyn Hook>>,
    pub return_data: Vec<RetData>,
    pub backups: Vec<StateBackup>,
    /// Original storage values before the transaction. Used for gas calculations in SSTORE.
    pub storage_original_values: HashMap<Address, HashMap<H256, U256>>,
}

pub struct RetData {
    pub is_create: bool,
    pub ret_offset: U256,
    pub ret_size: usize,
    pub should_transfer_value: bool,
    pub to: Address,
    pub msg_sender: Address,
    pub value: U256,
    pub max_message_call_gas: u64,
}

impl<'a> VM<'a> {
    pub fn new(
        env: Environment,
        db: &'a mut GeneralizedDatabase,
        tx: &Transaction,
    ) -> Result<Self, VMError> {
        // Add sender and recipient (in the case of a Call) to cache [https://www.evm.codes/about#access_list]
        let mut default_touched_accounts = HashSet::from_iter([env.origin].iter().cloned());

        // [EIP-3651] - Add coinbase to cache if the spec is SHANGHAI or higher
        if env.config.fork >= Fork::Shanghai {
            default_touched_accounts.insert(env.coinbase);
        }

        let mut default_touched_storage_slots: HashMap<Address, BTreeSet<H256>> = HashMap::new();

        // Add access lists contents to cache
        for (address, keys) in tx.access_list() {
            default_touched_accounts.insert(address);
            let mut warm_slots = BTreeSet::new();
            for slot in keys {
                warm_slots.insert(slot);
            }
            default_touched_storage_slots.insert(address, warm_slots);
        }

        // Add precompiled contracts addresses to cache.
        let max_precompile_address = match env.config.fork {
            spec if spec >= Fork::Prague => SIZE_PRECOMPILES_PRAGUE,
            spec if spec >= Fork::Cancun => SIZE_PRECOMPILES_CANCUN,
            spec if spec < Fork::Cancun => SIZE_PRECOMPILES_PRE_CANCUN,
            _ => return Err(VMError::Internal(InternalError::InvalidSpecId)),
        };
        for i in 1..=max_precompile_address {
            default_touched_accounts.insert(Address::from_low_u64_be(i));
        }

        #[cfg(not(feature = "l2"))]
        let hooks: Vec<Arc<dyn Hook>> = vec![Arc::new(DefaultHook)];
        #[cfg(feature = "l2")]
        let hooks: Vec<Arc<dyn Hook>> = {
            let recipient = if let Transaction::PrivilegedL2Transaction(PrivilegedL2Transaction {
                recipient,
                ..
            }) = tx
            {
                Some(*recipient)
            } else {
                None
            };
            vec![Arc::new(L2Hook { recipient })]
        };

        let mut substate = Substate {
            selfdestruct_set: HashSet::new(),
            touched_accounts: default_touched_accounts,
            touched_storage_slots: default_touched_storage_slots,
            created_accounts: HashSet::new(),
        };

        let bytecode;
        let destination_and_code_address;

        match tx.to() {
            TxKind::Call(address_to) => {
                substate.touched_accounts.insert(address_to);

                let (_is_delegation, _eip7702_gas_consumed, _code_address, bytes) =
                    eip7702_get_code(db, &mut substate, address_to)?;
                destination_and_code_address = address_to;
                bytecode = bytes;
            }

            TxKind::Create => {
                let sender_nonce = db.get_account(env.origin)?.info.nonce;

                // In this case, the destination address which also holds the code would be the address of the newly created contract
                destination_and_code_address = calculate_create_address(env.origin, sender_nonce)
                    .map_err(|_| {
                    VMError::Internal(InternalError::CouldNotComputeCreateAddress)
                })?;

                substate
                    .touched_accounts
                    .insert(destination_and_code_address);

                substate
                    .created_accounts
                    .insert(destination_and_code_address);

                bytecode = Bytes::new() //Bytecode will be later assigned from the calldata after passing validations;
            }
        }

        let initial_call_frame = CallFrame::new(
            env.origin,
            destination_and_code_address,
            destination_and_code_address,
            bytecode,
            tx.value(),
            tx.data().clone(),
            false,
            env.gas_limit,
            0,
            0,
            false,
        );

        Ok(Self {
            call_frames: vec![initial_call_frame],
            env,
            accrued_substate: substate,
            db,
            tx_kind: tx.to(),
            access_list: tx.access_list(),
            authorization_list: tx.authorization_list(),
            hooks,
            return_data: vec![],
            backups: vec![],
            storage_original_values: HashMap::new(),
        })
    }

    pub fn run_execution(&mut self) -> Result<ExecutionReport, VMError> {
        let fork = self.env.config.fork;

        if is_precompile(&self.current_call_frame()?.code_address, fork) {
            let mut current_call_frame = self
                .call_frames
                .pop()
                .ok_or(VMError::Internal(InternalError::CouldNotPopCallframe))?;
            let precompile_result = execute_precompile(&mut current_call_frame);
            let backup = self
                .backups
                .pop()
                .ok_or(VMError::Internal(InternalError::CouldNotPopCallframe))?;
            let report =
                self.handle_precompile_result(precompile_result, backup, &mut current_call_frame)?;
            self.handle_return(&current_call_frame, &report)?;
            self.current_call_frame_mut()?.increment_pc_by(1)?;
            return Ok(report);
        }

        loop {
            let opcode = self.current_call_frame()?.next_opcode();

            let op_result = self.handle_current_opcode(opcode);

            match op_result {
                Ok(OpcodeResult::Continue { pc_increment }) => self
                    .current_call_frame_mut()?
                    .increment_pc_by(pc_increment)?,
                Ok(OpcodeResult::Halt) => {
                    let mut executed_call_frame = self
                        .call_frames
                        .pop()
                        .ok_or(VMError::Internal(InternalError::CouldNotPopCallframe))?;
                    let report = self.handle_opcode_result(&mut executed_call_frame)?;
                    if self.handle_return(&executed_call_frame, &report)? {
                        self.current_call_frame_mut()?.increment_pc_by(1)?;
                    } else {
                        return Ok(report);
                    }
                }
                Err(error) => {
                    let mut executed_call_frame = self
                        .call_frames
                        .pop()
                        .ok_or(VMError::Internal(InternalError::CouldNotPopCallframe))?;
                    let report = self.handle_opcode_error(error, &mut executed_call_frame)?;
                    if self.handle_return(&executed_call_frame, &report)? {
                        self.current_call_frame_mut()?.increment_pc_by(1)?;
                    } else {
                        return Ok(report);
                    }
                }
            }
        }
    }

    pub fn restore_state(
        &mut self,
        backup: StateBackup,
        current_call_frame: &mut CallFrame,
    ) -> Result<(), VMError> {
        current_call_frame.restore_cache_state(self.db)?;
        self.accrued_substate = backup.substate;
        self.env.refunded_gas = backup.refunded_gas;
        self.env.transient_storage = backup.transient_storage;
        Ok(())
    }

    pub fn is_create(&self) -> bool {
        matches!(self.tx_kind, TxKind::Create)
    }

    /// Executes without making changes to the cache.
    pub fn stateless_execute(&mut self) -> Result<ExecutionReport, VMError> {
        let cache_backup = self.db.cache.clone();
        let report = self.execute()?;
        // Restore the cache to its original state
        self.db.cache = cache_backup;
        Ok(report)
    }

    /// Main function for executing an external transaction
    pub fn execute(&mut self) -> Result<ExecutionReport, VMError> {
        if let Err(e) = self.prepare_execution() {
            // We need to do a cleanup of the cache so that it doesn't interfere with next transaction's execution
            let mut current_call_frame = self
                .call_frames
                .pop()
                .ok_or(VMError::Internal(InternalError::CouldNotPopCallframe))?;
            current_call_frame.restore_cache_state(self.db)?;
            self.call_frames.push(current_call_frame);
            return Err(e);
        }

        // Here we clear the cache backup because if prepare_execution succeeded we don't want to
        // revert the changes it made.
        // Even if the transaction reverts we want to apply these kind of changes!
        // These are: Incrementing sender nonce, transferring value to a delegate account, decreasing sender account balance
        self.current_call_frame_mut()?.call_frame_backup = CallFrameBackup {
            original_accounts_info: HashMap::new(),
            original_account_storage_slots: HashMap::new(),
        };

        // In CREATE type transactions:
        //  Add created contract to cache, reverting transaction if the address is already occupied
        if self.is_create() {
            let new_contract_address = self.current_call_frame()?.to;
            let new_account = self.get_account_mut(new_contract_address)?;

            if new_account.has_code_or_nonce() {
                return self.handle_create_non_empty_account();
            }

            self.increase_account_balance(
                new_contract_address,
                self.current_call_frame()?.msg_value,
            )?;

            // https://eips.ethereum.org/EIPS/eip-161
            self.increment_account_nonce(new_contract_address)?;
        }

        // Backup of Substate, Gas Refunds and Transient Storage if sub-context is reverted
        let backup = StateBackup::new(
            self.accrued_substate.clone(),
            self.env.refunded_gas,
            self.env.transient_storage.clone(),
        );

        self.backups.push(backup);

        let mut report = self.run_execution()?;

        self.finalize_execution(&mut report)?;
        Ok(report)
    }

    pub fn current_call_frame_mut(&mut self) -> Result<&mut CallFrame, VMError> {
        self.call_frames.last_mut().ok_or(VMError::Internal(
            InternalError::CouldNotAccessLastCallframe,
        ))
    }

    pub fn current_call_frame(&self) -> Result<&CallFrame, VMError> {
        self.call_frames.last().ok_or(VMError::Internal(
            InternalError::CouldNotAccessLastCallframe,
        ))
    }

    fn handle_create_non_empty_account(&mut self) -> Result<ExecutionReport, VMError> {
        let mut report = ExecutionReport {
            result: TxResult::Revert(VMError::AddressAlreadyOccupied),
            gas_used: self.env.gas_limit,
            gas_refunded: 0,
            logs: vec![],
            output: Bytes::new(),
        };

        self.finalize_execution(&mut report)?;

        Ok(report)
    }

    fn prepare_execution(&mut self) -> Result<(), VMError> {
        // NOTE: ATTOW the default hook is created in VM::new(), so
        // (in theory) _at least_ the default prepare execution should
        // run
        for hook in self.hooks.clone() {
            hook.prepare_execution(self)?;
        }
        Ok(())
    }

    fn finalize_execution(&mut self, report: &mut ExecutionReport) -> Result<(), VMError> {
        // NOTE: ATTOW the default hook is created in VM::new(), so
        // (in theory) _at least_ the default finalize execution should
        // run
        for hook in self.hooks.clone() {
            hook.finalize_execution(self, report)?;
        }

        Ok(())
    }

    // The CallFrameBackup of the current callframe has to be merged with the backup of its parent, in the following way:
    //   - For every account that's present in the parent backup, do nothing (i.e. keep the one that's already there).
    //   - For every account that's NOT present in the parent backup but is on the child backup, add the child backup to it.
    //   - Do the same for every individual storage slot.
    pub fn merge_call_frame_backup_with_parent(
        &mut self,
        child_call_frame_backup: &CallFrameBackup,
    ) -> Result<(), VMError> {
        let parent_backup_accounts = &mut self
            .current_call_frame_mut()?
            .call_frame_backup
            .original_accounts_info;
        for (address, account) in child_call_frame_backup.original_accounts_info.iter() {
            if parent_backup_accounts.get(address).is_none() {
                parent_backup_accounts.insert(*address, account.clone());
            }
        }

        let parent_backup_storage = &mut self
            .current_call_frame_mut()?
            .call_frame_backup
            .original_account_storage_slots;
        for (address, storage) in child_call_frame_backup
            .original_account_storage_slots
            .iter()
        {
            let parent_storage = parent_backup_storage
                .entry(*address)
                .or_insert(HashMap::new());
            for (key, value) in storage {
                if parent_storage.get(key).is_none() {
                    parent_storage.insert(*key, *value);
                }
            }
        }

        Ok(())
    }
}
