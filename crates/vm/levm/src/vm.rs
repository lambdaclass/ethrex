use crate::{
    TransientStorage,
    call_frame::{CallFrame, Stack},
    db::gen_db::GeneralizedDatabase,
    debug::DebugMode,
    environment::Environment,
    errors::{ContextResult, ExecutionReport, InternalError, OpcodeResult, VMError},
    hooks::{
        backup_hook::BackupHook,
        hook::{Hook, get_hooks},
    },
    memory::Memory,
    opcodes::OpCodeFn,
    precompiles::{
        self, SIZE_PRECOMPILES_CANCUN, SIZE_PRECOMPILES_PRAGUE, SIZE_PRECOMPILES_PRE_CANCUN,
    },
    tracing::LevmCallTracer,
};
use bytes::Bytes;
use ethrex_common::{
    Address, H160, H256, U256,
    tracing::CallType,
    types::{AccessListEntry, Code, Fork, Log, Transaction, fee_config::FeeConfig},
};
use rustc_hash::{FxHashMap, FxHashSet};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, HashMap},
    mem,
    rc::Rc,
};

/// Storage mapping from slot key to value.
pub type Storage = HashMap<U256, H256>;

/// Specifies whether the VM operates in L1 or L2 mode.
#[derive(Debug, Clone, Copy, Default)]
pub enum VMType {
    /// Standard Ethereum L1 execution.
    #[default]
    L1,
    /// L2 rollup execution with additional fee handling.
    L2(FeeConfig),
}

/// Execution substate that tracks changes during transaction execution.
///
/// The substate maintains all information that may need to be reverted if a
/// call fails, including:
/// - Self-destructed accounts
/// - Accessed addresses and storage slots (for EIP-2929 gas accounting)
/// - Created accounts
/// - Gas refunds
/// - Transient storage (EIP-1153)
/// - Event logs
///
/// # Backup Mechanism
///
/// The substate supports checkpointing via [`push_backup`] and restoration via
/// [`revert_backup`] or commitment via [`commit_backup`]. This is used to handle
/// nested calls where inner calls may fail and need to be reverted.
///
/// Most fields are private by design. The backup mechanism only works correctly
/// if data modifications are append-only.
#[derive(Debug, Default)]
pub struct Substate {
    /// Parent checkpoint for reverting on failure.
    parent: Option<Box<Self>>,
    /// Accounts marked for self-destruction (deleted at end of transaction).
    selfdestruct_set: FxHashSet<Address>,
    /// Addresses accessed during execution (for EIP-2929 warm/cold gas costs).
    accessed_addresses: FxHashSet<Address>,
    /// Storage slots accessed per address (for EIP-2929 warm/cold gas costs).
    accessed_storage_slots: FxHashMap<Address, FxHashSet<H256>>,
    /// Accounts created during this transaction.
    created_accounts: FxHashSet<Address>,
    /// Accumulated gas refund (e.g., from storage clears).
    pub refunded_gas: u64,
    /// Transient storage (EIP-1153), cleared at end of transaction.
    transient_storage: TransientStorage,
    /// Event logs emitted during execution.
    logs: Vec<Log>,
}

impl Substate {
    pub fn from_accesses(
        accessed_addresses: FxHashSet<Address>,
        accessed_storage_slots: FxHashMap<Address, FxHashSet<H256>>,
    ) -> Self {
        Self {
            parent: None,

            selfdestruct_set: FxHashSet::default(),
            accessed_addresses,
            accessed_storage_slots,
            created_accounts: FxHashSet::default(),
            refunded_gas: 0,
            transient_storage: TransientStorage::new(),
            logs: Vec::new(),
        }
    }

    /// Push a checkpoint that can be either reverted or committed. All data up to this point is
    /// still accessible.
    pub fn push_backup(&mut self) {
        let parent = mem::take(self);
        self.refunded_gas = parent.refunded_gas;
        self.parent = Some(Box::new(parent));
    }

    /// Pop and merge with the last backup.
    ///
    /// Does nothing if the substate has no backup.
    pub fn commit_backup(&mut self) {
        if let Some(parent) = self.parent.as_mut() {
            let mut delta = mem::take(parent);
            mem::swap(self, &mut delta);

            self.selfdestruct_set.extend(delta.selfdestruct_set);
            self.accessed_addresses.extend(delta.accessed_addresses);
            for (address, slot_set) in delta.accessed_storage_slots {
                self.accessed_storage_slots
                    .entry(address)
                    .or_default()
                    .extend(slot_set);
            }
            self.created_accounts.extend(delta.created_accounts);
            self.refunded_gas = delta.refunded_gas;
            self.transient_storage.extend(delta.transient_storage);
            self.logs.extend(delta.logs);
        }
    }

    /// Discard current changes and revert to last backup.
    ///
    /// Does nothing if the substate has no backup.
    pub fn revert_backup(&mut self) {
        if let Some(parent) = self.parent.as_mut() {
            *self = mem::take(parent);
        }
    }

    /// Return an iterator over all selfdestruct addresses.
    pub fn iter_selfdestruct(&self) -> impl Iterator<Item = &Address> {
        struct Iter<'a> {
            parent: Option<&'a Substate>,
            iter: std::collections::hash_set::Iter<'a, Address>,
        }

        impl<'a> Iterator for Iter<'a> {
            type Item = &'a Address;

            fn next(&mut self) -> Option<Self::Item> {
                let next_item = self.iter.next();
                if next_item.is_none()
                    && let Some(parent) = self.parent
                {
                    self.parent = parent.parent.as_deref();
                    self.iter = parent.selfdestruct_set.iter();

                    return self.next();
                }

                next_item
            }
        }

        Iter {
            parent: self.parent.as_deref(),
            iter: self.selfdestruct_set.iter(),
        }
    }

    /// Mark an address as selfdestructed and return whether is was already marked.
    pub fn add_selfdestruct(&mut self, address: Address) -> bool {
        if self.selfdestruct_set.contains(&address) {
            return true;
        }

        let is_present = self
            .parent
            .as_ref()
            .map(|parent| parent.is_selfdestruct(&address))
            .unwrap_or(false);

        is_present || !self.selfdestruct_set.insert(address)
    }

    /// Return whether an address is already marked as selfdestructed.
    pub fn is_selfdestruct(&self, address: &Address) -> bool {
        self.selfdestruct_set.contains(address)
            || self
                .parent
                .as_ref()
                .map(|parent| parent.is_selfdestruct(address))
                .unwrap_or_default()
    }

    /// Build an access list from all accessed storage slots.
    pub fn make_access_list(&self) -> Vec<AccessListEntry> {
        let mut entries = BTreeMap::<Address, BTreeSet<H256>>::new();

        let mut current = self;
        loop {
            for (address, slot_set) in &current.accessed_storage_slots {
                entries
                    .entry(*address)
                    .or_default()
                    .extend(slot_set.iter().copied());
            }

            current = match current.parent.as_deref() {
                Some(x) => x,
                None => break,
            };
        }

        entries
            .into_iter()
            .map(|(address, storage_keys)| AccessListEntry {
                address,
                storage_keys: storage_keys.into_iter().collect(),
            })
            .collect()
    }

    /// Mark an address as accessed and return whether is was already marked.
    pub fn add_accessed_slot(&mut self, address: Address, key: H256) -> bool {
        // Check self first — short-circuits for re-accessed (warm) slots
        if self
            .accessed_storage_slots
            .get(&address)
            .map(|set| set.contains(&key))
            .unwrap_or(false)
        {
            return true;
        }

        let is_present = self
            .parent
            .as_ref()
            .map(|parent| parent.is_slot_accessed(&address, &key))
            .unwrap_or(false);

        is_present
            || !self
                .accessed_storage_slots
                .entry(address)
                .or_default()
                .insert(key)
    }

    /// Return whether an address has already been accessed.
    pub fn is_slot_accessed(&self, address: &Address, key: &H256) -> bool {
        self.accessed_storage_slots
            .get(address)
            .map(|slot_set| slot_set.contains(key))
            .unwrap_or_default()
            || self
                .parent
                .as_ref()
                .map(|parent| parent.is_slot_accessed(address, key))
                .unwrap_or_default()
    }

    /// Returns all accessed storage slots for a given address.
    /// Used by SELFDESTRUCT to record storage reads in BAL per EIP-7928:
    /// "SELFDESTRUCT: Include modified/read storage keys as storage_read"
    pub fn get_accessed_storage_slots(&self, address: &Address) -> BTreeSet<H256> {
        let mut slots = BTreeSet::new();

        // Collect from current substate
        if let Some(slot_set) = self.accessed_storage_slots.get(address) {
            slots.extend(slot_set.iter().copied());
        }

        // Collect from parent substates recursively
        if let Some(parent) = self.parent.as_ref() {
            slots.extend(parent.get_accessed_storage_slots(address));
        }

        slots
    }

    /// Mark an address as accessed and return whether is was already marked.
    pub fn add_accessed_address(&mut self, address: Address) -> bool {
        // Check self first — short-circuits for re-accessed (warm) addresses
        if self.accessed_addresses.contains(&address) {
            return true;
        }

        let is_present = self
            .parent
            .as_ref()
            .map(|parent| parent.is_address_accessed(&address))
            .unwrap_or(false);

        is_present || !self.accessed_addresses.insert(address)
    }

    /// Return whether an address has already been accessed.
    pub fn is_address_accessed(&self, address: &Address) -> bool {
        self.accessed_addresses.contains(address)
            || self
                .parent
                .as_ref()
                .map(|parent| parent.is_address_accessed(address))
                .unwrap_or_default()
    }

    /// Mark an address as a new account and return whether is was already marked.
    pub fn add_created_account(&mut self, address: Address) -> bool {
        if self.created_accounts.contains(&address) {
            return true;
        }

        let is_present = self
            .parent
            .as_ref()
            .map(|parent| parent.is_account_created(&address))
            .unwrap_or(false);

        is_present || !self.created_accounts.insert(address)
    }

    /// Return whether an address has already been marked as a new account.
    pub fn is_account_created(&self, address: &Address) -> bool {
        self.created_accounts.contains(address)
            || self
                .parent
                .as_ref()
                .map(|parent| parent.is_account_created(address))
                .unwrap_or_default()
    }

    /// Return the data associated with a transient storage entry, or zero if not present.
    pub fn get_transient(&self, to: &Address, key: &U256) -> U256 {
        self.transient_storage
            .get(&(*to, *key))
            .copied()
            .unwrap_or_else(|| {
                self.parent
                    .as_ref()
                    .map(|parent| parent.get_transient(to, key))
                    .unwrap_or_default()
            })
    }

    /// Return the data associated with a transient storage entry, or zero if not present.
    pub fn set_transient(&mut self, to: &Address, key: &U256, value: U256) {
        self.transient_storage.insert((*to, *key), value);
    }

    /// Extract all logs in order.
    pub fn extract_logs(&self) -> Vec<Log> {
        fn inner(substrate: &Substate, target: &mut Vec<Log>) {
            if let Some(parent) = substrate.parent.as_deref() {
                inner(parent, target);
            }

            target.extend_from_slice(&substrate.logs);
        }

        let mut logs = Vec::new();
        inner(self, &mut logs);

        logs
    }

    /// Push a log record.
    pub fn add_log(&mut self, log: Log) {
        self.logs.push(log);
    }
}

/// The LEVM (Lambda EVM) execution engine.
///
/// The VM executes Ethereum transactions by processing EVM bytecode. It maintains
/// a call stack, memory, and tracks all state changes during execution.
///
/// # Execution Model
///
/// 1. Transaction is validated (nonce, balance, gas limit)
/// 2. Initial call frame is created with transaction data
/// 3. Opcodes are executed sequentially until completion or error
/// 4. State changes are committed or reverted based on success
///
/// # Call Stack
///
/// Nested calls (CALL, DELEGATECALL, etc.) push new frames onto `call_frames`.
/// Each frame has its own memory, stack, and execution context. The `current_call_frame`
/// is always the active frame being executed.
///
/// # Hooks
///
/// The VM supports hooks for extending functionality (e.g., tracing, debugging).
/// Hooks are called at various points during execution and implement pre/post-execution
/// logic. L2-specific behavior (such as fee handling) is implemented via hooks.
///
/// # Example
///
/// ```ignore
/// let mut vm = VM::new(env, db, &tx, tracer, debug_mode, vm_type);
/// let report = vm.execute()?;
/// if report.is_success() {
///     println!("Gas used: {}, Output: {:?}", report.gas_used, report.output);
/// } else {
///     println!("Transaction reverted");
/// }
/// ```
pub struct VM<'a> {
    /// Stack of parent call frames (for nested calls).
    pub call_frames: Vec<CallFrame>,
    /// The currently executing call frame.
    pub current_call_frame: CallFrame,
    /// Block and transaction environment.
    pub env: Environment,
    /// Execution substate (accessed addresses, logs, refunds, etc.).
    pub substate: Substate,
    /// Database for reading/writing account state.
    pub db: &'a mut GeneralizedDatabase,
    /// The transaction being executed.
    pub tx: Transaction,
    /// Execution hooks for tracing and debugging.
    pub hooks: Vec<Rc<RefCell<dyn Hook>>>,
    /// Original storage values before transaction (for SSTORE gas calculation),
    /// keyed first by account to avoid hashing the full tuple on each access.
    pub storage_original_values: FxHashMap<Address, FxHashMap<H256, U256>>,
    /// Call tracer for execution tracing.
    pub tracer: LevmCallTracer,
    /// Debug mode for development diagnostics.
    pub debug_mode: DebugMode,
    /// Pool of reusable stacks to reduce allocations.
    pub stack_pool: Vec<Stack>,
    /// VM type (L1 or L2 with fee config).
    pub vm_type: VMType,
    /// Opcode dispatch table, built dynamically per fork.
    pub(crate) opcode_table: [OpCodeFn<'a>; 256],
}

impl<'a> VM<'a> {
    pub fn new(
        env: Environment,
        db: &'a mut GeneralizedDatabase,
        tx: &Transaction,
        tracer: LevmCallTracer,
        vm_type: VMType,
    ) -> Result<Self, VMError> {
        db.tx_backup = None; // If BackupHook is enabled, it will contain backup at the end of tx execution.

        let mut substate = Substate::initialize(&env, tx)?;

        let (callee, is_create) = Self::get_tx_callee(tx, db, &env, &mut substate)?;

        let fork = env.config.fork;

        let mut vm = Self {
            call_frames: Vec::new(),
            substate,
            db,
            tx: tx.clone(),
            hooks: get_hooks(&vm_type),
            storage_original_values: FxHashMap::default(),
            tracer,
            debug_mode: DebugMode::disabled(),
            stack_pool: Vec::new(),
            vm_type,
            current_call_frame: CallFrame::new(
                env.origin,
                callee,
                Address::default(), // Will be assigned at the end of prepare_execution
                Code::default(),    // Will be assigned at the end of prepare_execution
                tx.value(),
                tx.data().clone(),
                false,
                env.gas_limit,
                0,
                true,
                is_create,
                0,
                0,
                Stack::default(),
                Memory::default(),
            ),
            env,
            opcode_table: VM::build_opcode_table(fork),
        };

        let call_type = if is_create {
            CallType::CREATE
        } else {
            CallType::CALL
        };
        vm.tracer.enter(
            call_type,
            vm.env.origin,
            callee,
            vm.tx.value(),
            vm.env.gas_limit,
            vm.tx.data(),
        );

        #[cfg(feature = "debug")]
        {
            // Enable debug mode for printing in Solidity contracts.
            vm.debug_mode.enabled = true;
        }

        Ok(vm)
    }

    fn add_hook(&mut self, hook: impl Hook + 'static) {
        self.hooks.push(Rc::new(RefCell::new(hook)));
    }

    /// Executes a whole external transaction. Performing validations at the beginning.
    pub fn execute(&mut self) -> Result<ExecutionReport, VMError> {
        if let Err(e) = self.prepare_execution() {
            // Restore cache to state previous to this Tx execution because this Tx is invalid.
            self.restore_cache_state()?;
            return Err(e);
        }

        // Clear callframe backup so that changes made in prepare_execution are written in stone.
        // We want to apply these changes even if the Tx reverts. E.g. Incrementing sender nonce
        self.current_call_frame.call_frame_backup.clear();

        // EIP-7928: Take a BAL checkpoint AFTER clearing the backup. This captures the state
        // after prepare_execution (nonce increment, etc.) but before actual execution.
        // When the top-level call fails, we restore to this checkpoint so that inner call
        // state changes (like value transfers) are reverted from the BAL.
        self.current_call_frame.call_frame_backup.bal_checkpoint =
            self.db.bal_recorder.as_ref().map(|r| r.checkpoint());

        if self.is_create()? {
            // Create contract, reverting the Tx if address is already occupied.
            if let Some(context_result) = self.handle_create_transaction()? {
                let report = self.finalize_execution(context_result)?;
                return Ok(report);
            }
        }

        self.substate.push_backup();
        let context_result = self.run_execution()?;

        let report = self.finalize_execution(context_result)?;

        Ok(report)
    }

    /// Main execution loop.
    pub fn run_execution(&mut self) -> Result<ContextResult, VMError> {
        #[expect(clippy::as_conversions, reason = "remaining gas conversion")]
        if precompiles::is_precompile(
            &self.current_call_frame.to,
            self.env.config.fork,
            self.vm_type,
        ) {
            let call_frame = &mut self.current_call_frame;

            let mut gas_remaining = call_frame.gas_remaining as u64;
            let result = Self::execute_precompile(
                call_frame.code_address,
                &call_frame.calldata,
                call_frame.gas_limit,
                &mut gas_remaining,
                self.env.config.fork,
                self.db.store.precompile_cache(),
            );

            call_frame.gas_remaining = gas_remaining as i64;

            return result;
        }

        #[cfg(feature = "perf_opcode_timings")]
        let mut timings = crate::timings::OPCODE_TIMINGS.lock().expect("poison");

        loop {
            let opcode = self.current_call_frame.next_opcode();
            self.advance_pc(1)?;

            #[cfg(feature = "perf_opcode_timings")]
            let opcode_time_start = std::time::Instant::now();

            // Fast path for common opcodes
            #[allow(clippy::indexing_slicing, clippy::as_conversions)]
            let op_result = match opcode {
                0x5d if self.env.config.fork >= Fork::Cancun => self.op_tstore(),
                0x60 => self.op_push::<1>(),
                0x61 => self.op_push::<2>(),
                0x62 => self.op_push::<3>(),
                0x63 => self.op_push::<4>(),
                0x64 => self.op_push::<5>(),
                0x65 => self.op_push::<6>(),
                0x66 => self.op_push::<7>(),
                0x67 => self.op_push::<8>(),
                0x68 => self.op_push::<9>(),
                0x69 => self.op_push::<10>(),
                0x6a => self.op_push::<11>(),
                0x6b => self.op_push::<12>(),
                0x6c => self.op_push::<13>(),
                0x6d => self.op_push::<14>(),
                0x6e => self.op_push::<15>(),
                0x6f => self.op_push::<16>(),
                0x70 => self.op_push::<17>(),
                0x71 => self.op_push::<18>(),
                0x72 => self.op_push::<19>(),
                0x73 => self.op_push::<20>(),
                0x74 => self.op_push::<21>(),
                0x75 => self.op_push::<22>(),
                0x76 => self.op_push::<23>(),
                0x77 => self.op_push::<24>(),
                0x78 => self.op_push::<25>(),
                0x79 => self.op_push::<26>(),
                0x7a => self.op_push::<27>(),
                0x7b => self.op_push::<28>(),
                0x7c => self.op_push::<29>(),
                0x7d => self.op_push::<30>(),
                0x7e => self.op_push::<31>(),
                0x7f => self.op_push::<32>(),
                0x80 => self.op_dup::<0>(),
                0x81 => self.op_dup::<1>(),
                0x82 => self.op_dup::<2>(),
                0x83 => self.op_dup::<3>(),
                0x84 => self.op_dup::<4>(),
                0x85 => self.op_dup::<5>(),
                0x86 => self.op_dup::<6>(),
                0x87 => self.op_dup::<7>(),
                0x88 => self.op_dup::<8>(),
                0x89 => self.op_dup::<9>(),
                0x8a => self.op_dup::<10>(),
                0x8b => self.op_dup::<11>(),
                0x8c => self.op_dup::<12>(),
                0x8d => self.op_dup::<13>(),
                0x8e => self.op_dup::<14>(),
                0x8f => self.op_dup::<15>(),
                0x90 => self.op_swap::<1>(),
                0x91 => self.op_swap::<2>(),
                0x92 => self.op_swap::<3>(),
                0x93 => self.op_swap::<4>(),
                0x94 => self.op_swap::<5>(),
                0x95 => self.op_swap::<6>(),
                0x96 => self.op_swap::<7>(),
                0x97 => self.op_swap::<8>(),
                0x98 => self.op_swap::<9>(),
                0x99 => self.op_swap::<10>(),
                0x9a => self.op_swap::<11>(),
                0x9b => self.op_swap::<12>(),
                0x9c => self.op_swap::<13>(),
                0x9d => self.op_swap::<14>(),
                0x9e => self.op_swap::<15>(),
                0x9f => self.op_swap::<16>(),
                0x00 => self.op_stop(),
                0x01 => self.op_add(),
                0x02 => self.op_mul(),
                0x03 => self.op_sub(),
                0x10 => self.op_lt(),
                0x11 => self.op_gt(),
                0x14 => self.op_eq(),
                0x15 => self.op_iszero(),
                0x16 => self.op_and(),
                0x17 => self.op_or(),
                0x1b if self.env.config.fork >= Fork::Constantinople => self.op_shl(),
                0x1c if self.env.config.fork >= Fork::Constantinople => self.op_shr(),
                0x35 => self.op_calldataload(),
                0x39 => self.op_codecopy(),
                0x50 => self.op_pop(),
                0x51 => self.op_mload(),
                0x52 => self.op_mstore(),
                0x54 => self.op_sload(),
                0x56 => self.op_jump(),
                0x57 => self.op_jumpi(),
                0x5b => self.op_jumpdest(),
                0x5f if self.env.config.fork >= Fork::Shanghai => self.op_push0(),
                0xf3 => self.op_return(),
                _ => {
                    // Call the opcode, using the opcode function lookup table.
                    // Indexing will not panic as all the opcode values fit within the table.
                    self.opcode_table[opcode as usize].call(self)
                }
            };

            #[cfg(feature = "perf_opcode_timings")]
            {
                let time = opcode_time_start.elapsed();
                timings.update(opcode, time);
            }

            let result = match op_result {
                Ok(OpcodeResult::Continue) => continue,
                Ok(OpcodeResult::Halt) => self.handle_opcode_result()?,
                Err(error) => self.handle_opcode_error(error)?,
            };

            // Return the ExecutionReport if the executed callframe was the first one.
            if self.is_initial_call_frame() {
                self.handle_state_backup(&result)?;
                return Ok(result);
            }

            // Handle interaction between child and parent callframe.
            self.handle_return(&result)?;
        }
    }

    /// Executes precompile and handles the output that it returns, generating a report.
    pub fn execute_precompile(
        code_address: H160,
        calldata: &Bytes,
        gas_limit: u64,
        gas_remaining: &mut u64,
        fork: Fork,
        cache: Option<&precompiles::PrecompileCache>,
    ) -> Result<ContextResult, VMError> {
        Self::handle_precompile_result(
            precompiles::execute_precompile(code_address, calldata, gas_remaining, fork, cache),
            gas_limit,
            *gas_remaining,
        )
    }

    /// True if external transaction is a contract creation
    pub fn is_create(&self) -> Result<bool, InternalError> {
        Ok(self.current_call_frame.is_create)
    }

    /// Executes without making changes to the cache.
    pub fn stateless_execute(&mut self) -> Result<ExecutionReport, VMError> {
        // Add backup hook to restore state after execution.
        self.add_hook(BackupHook::default());
        let report = self.execute()?;
        // Restore cache to the state before execution.
        self.db.undo_last_transaction()?;
        Ok(report)
    }

    fn prepare_execution(&mut self) -> Result<(), VMError> {
        for hook in self.hooks.clone() {
            hook.borrow_mut().prepare_execution(self)?;
        }

        Ok(())
    }

    fn finalize_execution(
        &mut self,
        mut ctx_result: ContextResult,
    ) -> Result<ExecutionReport, VMError> {
        for hook in self.hooks.clone() {
            hook.borrow_mut()
                .finalize_execution(self, &mut ctx_result)?;
        }

        self.tracer.exit_context(&ctx_result, true)?;

        // Only include logs if transaction succeeded. When a transaction reverts,
        // no logs should be emitted (including EIP-7708 Transfer logs).
        let logs = if ctx_result.is_success() {
            self.substate.extract_logs()
        } else {
            Vec::new()
        };

        let report = ExecutionReport {
            result: ctx_result.result.clone(),
            gas_used: ctx_result.gas_used,
            gas_spent: ctx_result.gas_spent,
            gas_refunded: self.substate.refunded_gas,
            output: std::mem::take(&mut ctx_result.output),
            logs,
        };

        Ok(report)
    }
}

impl Substate {
    /// Initializes the VM substate, mainly adding addresses to the "accessed_addresses" field and the same with storage slots
    pub fn initialize(env: &Environment, tx: &Transaction) -> Result<Substate, VMError> {
        // Add sender and recipient to accessed accounts [https://www.evm.codes/about#access_list]
        let mut initial_accessed_addresses = FxHashSet::default();
        let mut initial_accessed_storage_slots: FxHashMap<Address, FxHashSet<H256>> =
            FxHashMap::default();

        // Add Tx sender to accessed accounts
        initial_accessed_addresses.insert(env.origin);

        // [EIP-3651] - Add coinbase to accessed accounts after Shanghai
        if env.config.fork >= Fork::Shanghai {
            initial_accessed_addresses.insert(env.coinbase);
        }

        // Add precompiled contracts addresses to accessed accounts.
        let max_precompile_address = match env.config.fork {
            spec if spec >= Fork::Prague => SIZE_PRECOMPILES_PRAGUE,
            spec if spec >= Fork::Cancun => SIZE_PRECOMPILES_CANCUN,
            spec if spec < Fork::Cancun => SIZE_PRECOMPILES_PRE_CANCUN,
            _ => return Err(InternalError::InvalidFork.into()),
        };

        for i in 1..=max_precompile_address {
            initial_accessed_addresses.insert(Address::from_low_u64_be(i));
        }

        // Add the address for the P256 verify precompile post-Osaka
        if env.config.fork >= Fork::Osaka {
            initial_accessed_addresses.insert(Address::from_low_u64_be(0x100));
        }

        // Add access lists contents to accessed accounts and accessed storage slots.
        for (address, keys) in tx.access_list().clone() {
            initial_accessed_addresses.insert(address);
            // Access lists can have different entries even for the same address, that's why we check if there's an existing set instead of considering it empty
            let warm_slots = initial_accessed_storage_slots.entry(address).or_default();
            for slot in keys {
                warm_slots.insert(slot);
            }
        }

        let substate =
            Substate::from_accesses(initial_accessed_addresses, initial_accessed_storage_slots);

        Ok(substate)
    }
}
