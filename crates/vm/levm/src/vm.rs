use crate::{
    account::StorageSlot,
    call_frame::{CacheBackup, CallFrame},
    constants::*,
    db::{cache, gen_db::GeneralizedDatabase},
    environment::Environment,
    errors::{
        ExecutionReport, InternalError, OpcodeResult, OutOfGasError, TxResult, TxValidationError,
        VMError,
    },
    gas_cost::{
        self, ACCESS_LIST_ADDRESS_COST, ACCESS_LIST_STORAGE_KEY_COST, CREATE_BASE_COST,
        STANDARD_TOKEN_COST, TOTAL_COST_FLOOR_PER_TOKEN,
    },
    hooks::{default_hook::DefaultHook, hook::Hook, l2_hook::L2Hook},
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
pub type Storage = HashMap<U256, H256>;

#[derive(Debug, Clone, Default)]
pub struct Substate {
    pub selfdestruct_set: HashSet<Address>,
    pub touched_accounts: HashSet<Address>,
    pub touched_storage_slots: HashMap<Address, BTreeSet<H256>>,
    pub created_accounts: HashSet<Address>,
}

/// Backup if sub-context is reverted. It consists of a copy of:
///   - Database
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
/// This structs holds special configuration variables specific to the
/// EVM. In most cases, at least at the time of writing (February
/// 2025), you want to use the default blob_schedule values for the
/// specified Fork. The "intended" way to do this is by using the `EVMConfig::canonical_values(fork: Fork)` function.
///
/// However, that function should NOT be used IF you want to use a
/// custom ForkBlobSchedule, like it's described in
/// [EIP-7840](https://eips.ethereum.org/EIPS/eip-7840). For more
/// information read the EIP
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

    /// After EIP-7691 the maximum number of blob hashes changed. For more
    /// information see
    /// [EIP-7691](https://eips.ethereum.org/EIPS/eip-7691#specification).
    const fn max_blobs_per_block(fork: Fork) -> u64 {
        match fork {
            Fork::Prague => MAX_BLOB_COUNT_ELECTRA,
            Fork::Osaka => MAX_BLOB_COUNT_ELECTRA,
            _ => MAX_BLOB_COUNT,
        }
    }

    /// According to [EIP-7691](https://eips.ethereum.org/EIPS/eip-7691#specification):
    ///
    /// "These changes imply that get_base_fee_per_blob_gas and
    /// calc_excess_blob_gas functions defined in EIP-4844 use the new
    /// values for the first block of the fork (and for all subsequent
    /// blocks)."
    const fn get_blob_base_fee_update_fraction_value(fork: Fork) -> u64 {
        match fork {
            Fork::Prague | Fork::Osaka => BLOB_BASE_FEE_UPDATE_FRACTION_PRAGUE,
            _ => BLOB_BASE_FEE_UPDATE_FRACTION,
        }
    }

    /// According to [EIP-7691](https://eips.ethereum.org/EIPS/eip-7691#specification):
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
    /// Information that is acted upon immediately following the
    /// transaction.
    pub accrued_substate: Substate,
    pub db: &'a mut GeneralizedDatabase,
    pub tx_kind: TxKind,
    pub access_list: AccessList,
    pub authorization_list: Option<AuthorizationList>,
    pub hooks: Vec<Arc<dyn Hook>>,
    pub return_data: Vec<RetData>,
    pub backups: Vec<StateBackup>,
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

        // When instantiating a new vm the current value of the storage slots are actually the original values because it is a new transaction
        for account in db.cache.values_mut() {
            for storage_slot in account.storage.values_mut() {
                storage_slot.original_value = storage_slot.current_value;
            }
        }

        let hooks: Vec<Arc<dyn Hook>> = match tx {
            Transaction::PrivilegedL2Transaction(privileged_tx) => vec![Arc::new(L2Hook {
                recipient: privileged_tx.recipient,
            })],
            _ => vec![Arc::new(DefaultHook)],
        };

        match tx.to() {
            TxKind::Call(address_to) => {
                default_touched_accounts.insert(address_to);

                let mut substate = Substate {
                    selfdestruct_set: HashSet::new(),
                    touched_accounts: default_touched_accounts,
                    touched_storage_slots: default_touched_storage_slots,
                    created_accounts: HashSet::new(),
                };

                let (_is_delegation, _eip7702_gas_consumed, _code_address, bytecode) =
                    eip7702_get_code(db, &mut substate, address_to)?;

                let initial_call_frame = CallFrame::new(
                    env.origin,
                    address_to,
                    address_to,
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
                    tx_kind: TxKind::Call(address_to),
                    access_list: tx.access_list(),
                    authorization_list: tx.authorization_list(),
                    hooks,
                    return_data: vec![],
                    backups: vec![],
                })
            }
            TxKind::Create => {
                let sender_nonce = db.get_account(env.origin)?.info.nonce;
                let new_contract_address = calculate_create_address(env.origin, sender_nonce)
                    .map_err(|_| VMError::Internal(InternalError::CouldNotComputeCreateAddress))?;

                default_touched_accounts.insert(new_contract_address);

                let initial_call_frame = CallFrame::new(
                    env.origin,
                    new_contract_address,
                    new_contract_address,
                    Bytes::new(), // Bytecode is assigned after passing validations.
                    tx.value(),
                    tx.data().clone(), // Calldata is removed after passing validations.
                    false,
                    env.gas_limit,
                    0,
                    0,
                    false,
                );

                let substate = Substate {
                    selfdestruct_set: HashSet::new(),
                    touched_accounts: default_touched_accounts,
                    touched_storage_slots: default_touched_storage_slots,
                    created_accounts: HashSet::from([new_contract_address]),
                };

                Ok(Self {
                    call_frames: vec![initial_call_frame],
                    env,
                    accrued_substate: substate,
                    db,
                    tx_kind: TxKind::Create,
                    access_list: tx.access_list(),
                    authorization_list: tx.authorization_list(),
                    hooks,
                    return_data: vec![],
                    backups: vec![],
                })
            }
        }
    }

    pub fn run_execution(&mut self) -> Result<ExecutionReport, VMError> {
        let fork = self.env.config.fork;

        if is_precompile(&self.current_call_frame()?.code_address, fork) {
            let mut current_call_frame = self
                .call_frames
                .pop()
                .ok_or(VMError::Internal(InternalError::CouldNotPopCallframe))?;
            let precompile_result = execute_precompile(&mut current_call_frame, fork);
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
                    let mut current_call_frame = self
                        .call_frames
                        .pop()
                        .ok_or(VMError::Internal(InternalError::CouldNotPopCallframe))?;
                    let report = self.handle_opcode_result(&mut current_call_frame)?;
                    if self.handle_return(&current_call_frame, &report)? {
                        self.current_call_frame_mut()?.increment_pc_by(1)?;
                    } else {
                        return Ok(report);
                    }
                }
                Err(error) => {
                    let mut current_call_frame = self
                        .call_frames
                        .pop()
                        .ok_or(VMError::Internal(InternalError::CouldNotPopCallframe))?;
                    let report = self.handle_opcode_error(error, &mut current_call_frame)?;
                    if self.handle_return(&current_call_frame, &report)? {
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
        call_frame_backup: CacheBackup,
    ) -> Result<(), VMError> {
        self.restore_cache_state(call_frame_backup)?;
        self.accrued_substate = backup.substate;
        self.env.refunded_gas = backup.refunded_gas;
        self.env.transient_storage = backup.transient_storage;
        Ok(())
    }

    pub fn is_create(&self) -> bool {
        matches!(self.tx_kind, TxKind::Create)
    }

    /// Calculates the minimum gas to be consumed in the transaction.
    pub fn get_min_gas_used(&self, initial_call_frame: &CallFrame) -> Result<u64, VMError> {
        // If the transaction is a CREATE transaction, the calldata is emptied and the bytecode is assigned.
        let calldata = if self.is_create() {
            &initial_call_frame.bytecode
        } else {
            &initial_call_frame.calldata
        };

        // tokens_in_calldata = nonzero_bytes_in_calldata * 4 + zero_bytes_in_calldata
        // tx_calldata = nonzero_bytes_in_calldata * 16 + zero_bytes_in_calldata * 4
        // this is actually tokens_in_calldata * STANDARD_TOKEN_COST
        // see it in https://eips.ethereum.org/EIPS/eip-7623
        let tokens_in_calldata: u64 = gas_cost::tx_calldata(calldata, self.env.config.fork)
            .map_err(VMError::OutOfGas)?
            .checked_div(STANDARD_TOKEN_COST)
            .ok_or(VMError::Internal(InternalError::DivisionError))?;

        // min_gas_used = TX_BASE_COST + TOTAL_COST_FLOOR_PER_TOKEN * tokens_in_calldata
        let mut min_gas_used: u64 = tokens_in_calldata
            .checked_mul(TOTAL_COST_FLOOR_PER_TOKEN)
            .ok_or(VMError::Internal(InternalError::GasOverflow))?;

        min_gas_used = min_gas_used
            .checked_add(TX_BASE_COST)
            .ok_or(VMError::Internal(InternalError::GasOverflow))?;

        Ok(min_gas_used)
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
            self.restore_cache_state(self.current_call_frame()?.cache_backup.clone())?;
            return Err(e);
        }

        // Here we clear the cache backup because if prepare_execution succeeded we don't want to
        // revert the changes it made.
        // Even if the transaction reverts we want to apply these kind of changes!
        // These are: Incrementing sender nonce, transferring value to a delegate account, decreasing sender account balance
        self.current_call_frame_mut()?.cache_backup = HashMap::new();

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
            if self.env.config.fork >= Fork::SpuriousDragon {
                self.increment_account_nonce(new_contract_address)?;
            };
        }

        // Backup of Database, Substate, Gas Refunds and Transient Storage if sub-context is reverted
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

    /// Accesses to an account's storage slot.
    ///
    /// Accessed storage slots are stored in the `touched_storage_slots` set.
    /// Accessed storage slots take place in some gas cost computation.
    pub fn access_storage_slot(
        &mut self,
        address: Address,
        key: H256,
    ) -> Result<(StorageSlot, bool), VMError> {
        // [EIP-2929] - Introduced conditional tracking of accessed storage slots for Berlin and later specs.
        let mut storage_slot_was_cold = false;
        if self.env.config.fork >= Fork::Berlin {
            storage_slot_was_cold = self
                .accrued_substate
                .touched_storage_slots
                .entry(address)
                .or_default()
                .insert(key);
        }
        let storage_slot = match cache::get_account(&self.db.cache, &address) {
            Some(account) => match account.storage.get(&key) {
                Some(storage_slot) => storage_slot.clone(),
                None => {
                    let value = self.db.store.get_storage_slot(address, key)?;
                    StorageSlot {
                        original_value: value,
                        current_value: value,
                    }
                }
            },
            None => {
                let value = self.db.store.get_storage_slot(address, key)?;
                StorageSlot {
                    original_value: value,
                    current_value: value,
                }
            }
        };

        // When updating account storage of an account that's not yet cached we need to store the StorageSlot in the account
        // Note: We end up caching the account because it is the most straightforward way of doing it.
        let account = self.get_account_mut(address)?;
        account.storage.insert(key, storage_slot.clone());

        Ok((storage_slot, storage_slot_was_cold))
    }

    pub fn update_account_storage(
        &mut self,
        address: Address,
        key: H256,
        new_value: U256,
    ) -> Result<(), VMError> {
        let account = self.get_account_mut(address)?;
        let account_original_storage_slot_value = account
            .storage
            .get(&key)
            .map_or(U256::zero(), |slot| slot.original_value);
        let slot = account.storage.entry(key).or_insert(StorageSlot {
            original_value: account_original_storage_slot_value,
            current_value: new_value,
        });
        slot.current_value = new_value;
        Ok(())
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
        let call_frame = self
            .call_frames
            .pop()
            .ok_or(VMError::Internal(InternalError::CouldNotPopCallframe))?;
        for hook in self.hooks.clone() {
            hook.finalize_execution(self, &call_frame, report)?;
        }
        self.call_frames.push(call_frame);

        Ok(())
    }

    /// Restores the cache state to the state before changes made during a callframe.
    fn restore_cache_state(&mut self, call_frame_backup: CacheBackup) -> Result<(), VMError> {
        for (address, account_opt) in call_frame_backup {
            if let Some(account) = account_opt {
                // restore the account to the state before the call
                cache::insert_account(&mut self.db.cache, address, account.clone());
            } else {
                // remove from cache if it wasn't there before
                cache::remove_account(&mut self.db.cache, &address);
            }
        }
        Ok(())
    }

    /// Sets the account code as the EIP7702 determines.
    pub fn eip7702_set_access_code(&mut self) -> Result<(), VMError> {
        let mut refunded_gas: u64 = 0;
        // IMPORTANT:
        // If any of the below steps fail, immediately stop processing that tuple and continue to the next tuple in the list. It will in the case of multiple tuples for the same authority, set the code using the address in the last valid occurrence.
        // If transaction execution results in failure (any exceptional condition or code reverting), setting delegation designations is not rolled back.
        for auth_tuple in self.authorization_list.clone().unwrap_or_default() {
            let chain_id_not_equals_this_chain_id = auth_tuple.chain_id != self.env.chain_id;
            let chain_id_not_zero = !auth_tuple.chain_id.is_zero();

            // 1. Verify the chain id is either 0 or the chain’s current ID.
            if chain_id_not_zero && chain_id_not_equals_this_chain_id {
                continue;
            }

            // 2. Verify the nonce is less than 2**64 - 1.
            // NOTE: nonce is a u64, it's always less than or equal to u64::MAX
            if auth_tuple.nonce == u64::MAX {
                continue;
            }

            // 3. authority = ecrecover(keccak(MAGIC || rlp([chain_id, address, nonce])), y_parity, r, s)
            //      s value must be less than or equal to secp256k1n/2, as specified in EIP-2.
            let Some(authority_address) = eip7702_recover_address(&auth_tuple)? else {
                continue;
            };

            // 4. Add authority to accessed_addresses (as defined in EIP-2929).
            self.accrued_substate
                .touched_accounts
                .insert(authority_address);
            let authority_account_info = self.db.get_account_no_push_cache(authority_address)?.info;

            // 5. Verify the code of authority is either empty or already delegated.
            let empty_or_delegated = authority_account_info.bytecode.is_empty()
                || has_delegation(&authority_account_info)?;
            if !empty_or_delegated {
                continue;
            }

            // 6. Verify the nonce of authority is equal to nonce. In case authority does not exist in the trie, verify that nonce is equal to 0.
            // If it doesn't exist, it means the nonce is zero. The access_account() function will return AccountInfo::default()
            // If it has nonce, the account.info.nonce should equal auth_tuple.nonce
            if authority_account_info.nonce != auth_tuple.nonce {
                continue;
            }

            // 7. Add PER_EMPTY_ACCOUNT_COST - PER_AUTH_BASE_COST gas to the global refund counter if authority exists in the trie.
            if cache::is_account_cached(&self.db.cache, &authority_address)
                || account_exists(self.db, authority_address)
            {
                let refunded_gas_if_exists = PER_EMPTY_ACCOUNT_COST - PER_AUTH_BASE_COST;
                refunded_gas = refunded_gas
                    .checked_add(refunded_gas_if_exists)
                    .ok_or(VMError::Internal(InternalError::GasOverflow))?;
            }

            // 8. Set the code of authority to be 0xef0100 || address. This is a delegation designation.
            let delegation_bytes = [
                &SET_CODE_DELEGATION_BYTES[..],
                auth_tuple.address.as_bytes(),
            ]
            .concat();

            // As a special case, if address is 0x0000000000000000000000000000000000000000 do not write the designation.
            // Clear the account’s code and reset the account’s code hash to the empty hash.
            let auth_account = self.get_account_mut(authority_address)?;

            auth_account.info.bytecode = if auth_tuple.address != Address::zero() {
                delegation_bytes.into()
            } else {
                Bytes::new()
            };

            // 9. Increase the nonce of authority by one.
            self.increment_account_nonce(authority_address)
                .map_err(|_| VMError::TxValidation(TxValidationError::NonceIsMax))?;
        }

        let code_address = self.current_call_frame()?.code_address;
        let (code_address_info, _) = self
            .db
            .access_account(&mut self.accrued_substate, code_address)?;

        if has_delegation(&code_address_info)? {
            self.current_call_frame_mut()?.code_address =
                get_authorized_address(&code_address_info)?;
            let code_address = self.current_call_frame()?.code_address;
            let (auth_address_info, _) = self
                .db
                .access_account(&mut self.accrued_substate, code_address)?;

            self.current_call_frame_mut()?.bytecode = auth_address_info.bytecode.clone();
        } else {
            self.current_call_frame_mut()?.bytecode = code_address_info.bytecode.clone();
        }

        self.current_call_frame_mut()?.valid_jump_destinations =
            get_valid_jump_destinations(&self.current_call_frame()?.bytecode).unwrap_or_default();

        self.env.refunded_gas = refunded_gas;

        Ok(())
    }

    pub fn add_intrinsic_gas(&mut self) -> Result<(), VMError> {
        // Intrinsic gas is the gas consumed by the transaction before the execution of the opcodes. Section 6.2 in the Yellow Paper.

        let intrinsic_gas = self.get_intrinsic_gas()?;

        self.current_call_frame_mut()?
            .increase_consumed_gas(intrinsic_gas)
            .map_err(|_| TxValidationError::IntrinsicGasTooLow)?;

        Ok(())
    }

    // ==================== Gas related functions =======================
    pub fn get_intrinsic_gas(&self) -> Result<u64, VMError> {
        // Intrinsic Gas = Calldata cost + Create cost + Base cost + Access list cost
        let mut intrinsic_gas: u64 = 0;

        // Calldata Cost
        // 4 gas for each zero byte in the transaction data 16 gas for each non-zero byte in the transaction.
        let calldata_cost =
            gas_cost::tx_calldata(&self.current_call_frame()?.calldata, self.env.config.fork)
                .map_err(VMError::OutOfGas)?;

        intrinsic_gas = intrinsic_gas
            .checked_add(calldata_cost)
            .ok_or(OutOfGasError::ConsumedGasOverflow)?;

        // Base Cost
        intrinsic_gas = intrinsic_gas
            .checked_add(TX_BASE_COST)
            .ok_or(OutOfGasError::ConsumedGasOverflow)?;

        // Create Cost
        if self.is_create() {
            // https://eips.ethereum.org/EIPS/eip-2#specification
            if self.env.config.fork >= Fork::Homestead {
                intrinsic_gas = intrinsic_gas
                    .checked_add(CREATE_BASE_COST)
                    .ok_or(OutOfGasError::ConsumedGasOverflow)?;
            }

            // https://eips.ethereum.org/EIPS/eip-3860
            if self.env.config.fork >= Fork::Shanghai {
                let number_of_words = &self
                    .current_call_frame()?
                    .calldata
                    .len()
                    .div_ceil(WORD_SIZE);
                let double_number_of_words: u64 = number_of_words
                    .checked_mul(2)
                    .ok_or(OutOfGasError::ConsumedGasOverflow)?
                    .try_into()
                    .map_err(|_| VMError::Internal(InternalError::ConversionError))?;

                intrinsic_gas = intrinsic_gas
                    .checked_add(double_number_of_words)
                    .ok_or(OutOfGasError::ConsumedGasOverflow)?;
            }
        }

        // Access List Cost
        let mut access_lists_cost: u64 = 0;
        for (_, keys) in &self.access_list {
            access_lists_cost = access_lists_cost
                .checked_add(ACCESS_LIST_ADDRESS_COST)
                .ok_or(OutOfGasError::ConsumedGasOverflow)?;
            for _ in keys {
                access_lists_cost = access_lists_cost
                    .checked_add(ACCESS_LIST_STORAGE_KEY_COST)
                    .ok_or(OutOfGasError::ConsumedGasOverflow)?;
            }
        }

        intrinsic_gas = intrinsic_gas
            .checked_add(access_lists_cost)
            .ok_or(OutOfGasError::ConsumedGasOverflow)?;

        // Authorization List Cost
        // `unwrap_or_default` will return an empty vec when the `authorization_list` field is None.
        // If the vec is empty, the len will be 0, thus the authorization_list_cost is 0.
        let amount_of_auth_tuples: u64 = self
            .authorization_list
            .clone()
            .unwrap_or_default()
            .len()
            .try_into()
            .map_err(|_| VMError::Internal(InternalError::ConversionError))?;
        let authorization_list_cost = PER_EMPTY_ACCOUNT_COST
            .checked_mul(amount_of_auth_tuples)
            .ok_or(VMError::Internal(InternalError::GasOverflow))?;

        intrinsic_gas = intrinsic_gas
            .checked_add(authorization_list_cost)
            .ok_or(OutOfGasError::ConsumedGasOverflow)?;

        Ok(intrinsic_gas)
    }
}
