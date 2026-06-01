use crate::{
    TransientStorage,
    call_frame::{CallFrame, Stack},
    db::gen_db::GeneralizedDatabase,
    debug::DebugMode,
    environment::Environment,
    errors::{
        ContextResult, ExceptionalHalt, ExecutionReport, InternalError, OpcodeResult, TxResult,
        VMError,
    },
    gas_cost::{
        STATE_BYTES_PER_AUTH_BASE, STATE_BYTES_PER_AUTH_TOTAL, STATE_BYTES_PER_NEW_ACCOUNT,
        STATE_BYTES_PER_STORAGE_SET, cost_per_state_byte as compute_cost_per_state_byte,
    },
    hooks::{
        backup_hook::BackupHook,
        hook::{Hook, get_hooks},
    },
    memory::Memory,
    opcode_tracer::LevmOpcodeTracer,
    opcodes::OpCodeFn,
    precompiles::{
        self, SIZE_PRECOMPILES_CANCUN, SIZE_PRECOMPILES_PRAGUE, SIZE_PRECOMPILES_PRE_CANCUN,
    },
    tracing::LevmCallTracer,
};
use bytes::Bytes;
use ethrex_common::{
    Address, BigEndianHash, H160, H256, U256,
    tracing::CallType,
    types::{AccessListEntry, Code, Fork, Log, Transaction, fee_config::FeeConfig},
};
use ethrex_crypto::Crypto;
use rustc_hash::{FxHashMap, FxHashSet};
use std::{
    cell::{OnceCell, RefCell},
    collections::{BTreeMap, BTreeSet},
    mem,
    rc::Rc,
};

/// Storage mapping from slot key to value.
pub type Storage = FxHashMap<U256, H256>;

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
            transient_storage: TransientStorage::default(),
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
            .unwrap_or_default();

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

    /// Mark an address as accessed and return whether the slot was cold.
    pub fn add_accessed_slot(&mut self, address: Address, key: H256) -> bool {
        if self
            .accessed_storage_slots
            .get(&address)
            .is_some_and(|set| set.contains(&key))
        {
            return false;
        }

        let is_present = self
            .parent
            .as_ref()
            .map(|parent| parent.is_slot_accessed(&address, &key))
            .unwrap_or_default();

        // Note: Do not simplify this expression, it uses `||` to avoid executing the right hand
        //   expression if not necessary.
        #[expect(clippy::nonminimal_bool, reason = "order of evaluation matters")]
        !(is_present
            || !self
                .accessed_storage_slots
                .entry(address)
                .or_default()
                .insert(key))
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

    /// Mark an address as accessed and return whether the address was cold.
    pub fn add_accessed_address(&mut self, address: Address) -> bool {
        if self.accessed_addresses.contains(&address) {
            return false;
        }

        let is_present = self
            .parent
            .as_ref()
            .map(|parent| parent.is_address_accessed(&address))
            .unwrap_or_default();

        // Note: Do not simplify this expression, it uses `||` to avoid executing the right hand
        //   expression if not necessary.
        #[expect(clippy::nonminimal_bool, reason = "order of evaluation matters")]
        !(is_present || !self.accessed_addresses.insert(address))
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
            .unwrap_or_default();

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
/// let mut vm = VM::new(env, db, &tx, tracer, vm_type, &NativeCrypto);
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
    /// Opcode (EIP-3155) tracer.  Disabled by default; zero overhead when inactive.
    pub opcode_tracer: LevmOpcodeTracer,
    /// Debug mode for development diagnostics.
    pub debug_mode: DebugMode,
    /// Pool of reusable stacks to reduce allocations.
    pub stack_pool: Vec<Stack>,
    /// VM type (L1 or L2 with fee config).
    pub vm_type: VMType,
    /// EIP-8037: Accumulated state gas for this transaction (Amsterdam+).
    /// Signed: goes negative when inline refunds exceed gross charges in the local frame
    /// (e.g. SSTORE 0→x→0 restoration matching an ancestor's charge).
    pub state_gas_used: i64,
    /// EIP-8037: State gas reservoir pre-funded from excess gas_limit (Amsterdam+).
    pub state_gas_reservoir: u64,
    /// EIP-8037: Initial reservoir at tx start (before any execution). Captured in
    /// add_intrinsic_gas so block-dimensional regular gas can be computed
    /// independently of mid-tx reservoir activity (auth refunds, SSTORE credits).
    pub state_gas_reservoir_initial: u64,
    /// EIP-8037: Cumulative state gas that spilled to regular gas during execution
    /// (when reservoir was insufficient). Subtracted when computing dimensional
    /// regular gas for block accounting — EELS charge_state_gas spills don't
    /// increment regular_gas_used.
    pub state_gas_spill: u64,
    /// EIP-8037: Dynamic cost per state byte (computed from block_gas_limit, Amsterdam+).
    pub cost_per_state_byte: u64,
    /// EIP-8037: State gas for new account creation (STATE_BYTES_PER_NEW_ACCOUNT * cost_per_state_byte).
    pub state_gas_new_account: u64,
    /// EIP-8037: State gas for storage slot creation (STATE_BYTES_PER_STORAGE_SET * cost_per_state_byte).
    pub state_gas_storage_set: u64,
    /// EIP-8037: State gas for EIP-7702 auth total (STATE_BYTES_PER_AUTH_TOTAL * cost_per_state_byte).
    pub state_gas_auth_total: u64,
    /// EIP-8037: State gas for the 23-byte EIP-7702 delegation indicator
    /// (STATE_BYTES_PER_AUTH_BASE * cost_per_state_byte). Refunded by
    /// `set_delegation` when no new delegation indicator bytes are written —
    /// either the authority's code slot already holds an indicator or the
    /// auth clears against an empty authority.
    pub state_gas_auth_base: u64,
    /// EIP-8037: state-gas refund channel.
    /// Mirrors EELS `MessageCallOutput.state_refund` — a separate, monotonic accumulator
    /// for refunds that bypass per-frame `state_gas_used` accounting. Populated by
    /// `set_delegation` for existing-authority refunds, subtracted from block-level
    /// state-gas at the end of `refund_sender`. Survives revert/halt/OOG since it lives
    /// on the VM, not in any call-frame backup.
    pub state_refund: u64,
    /// EIP-8037: intrinsic state gas (`tx_env.intrinsic_state_gas` in EELS). Captured at
    /// `add_intrinsic_gas` time. ethrex lumps intrinsic + execution into `state_gas_used`,
    /// so on top-level error this field is what we leave behind when refunding the
    /// execution portion to the reservoir — block accounting then bills the intrinsic
    /// (matches EELS `tx_state_gas = intrinsic_state_gas + tx_output.state_gas_used`).
    pub intrinsic_state_gas: u64,
    /// The opcode table mapping opcodes to opcode handlers for fast lookup.
    /// Build dynamically according to the given fork config.
    pub(crate) opcode_table: [OpCodeFn; 256],
    /// Crypto provider for cryptographic operations.
    pub crypto: &'a dyn Crypto,
}

impl<'a> VM<'a> {
    pub fn new(
        env: Environment,
        db: &'a mut GeneralizedDatabase,
        tx: &Transaction,
        tracer: LevmCallTracer,
        vm_type: VMType,
        crypto: &'a dyn Crypto,
    ) -> Result<Self, VMError> {
        db.tx_backup = None; // If BackupHook is enabled, it will contain backup at the end of tx execution.

        let mut substate = Substate::initialize(&env, tx)?;

        let (callee, is_create) = Self::get_tx_callee(tx, db, &env, &mut substate)?;

        let fork = env.config.fork;

        #[expect(
            clippy::arithmetic_side_effects,
            reason = "byte-count constants are small (<200) and cpsb is bounded by block_gas_limit/year formula"
        )]
        let (
            cpsb,
            state_gas_new_account,
            state_gas_storage_set,
            state_gas_auth_total,
            state_gas_auth_base,
        ) = if fork >= Fork::Amsterdam {
            let cpsb = compute_cost_per_state_byte(env.block_gas_limit);
            (
                cpsb,
                STATE_BYTES_PER_NEW_ACCOUNT * cpsb,
                STATE_BYTES_PER_STORAGE_SET * cpsb,
                STATE_BYTES_PER_AUTH_TOTAL * cpsb,
                STATE_BYTES_PER_AUTH_BASE * cpsb,
            )
        } else {
            (0, 0, 0, 0, 0)
        };

        let mut vm = Self {
            call_frames: Vec::new(),
            substate,
            db,
            tx: tx.clone(),
            hooks: get_hooks(&vm_type),
            storage_original_values: FxHashMap::default(),
            tracer,
            opcode_tracer: LevmOpcodeTracer::disabled(),
            debug_mode: DebugMode::disabled(),
            stack_pool: Vec::new(),
            vm_type,
            state_gas_used: 0,
            state_gas_reservoir: 0,
            state_gas_reservoir_initial: 0,
            state_gas_spill: 0,
            cost_per_state_byte: cpsb,
            state_gas_new_account,
            state_gas_storage_set,
            state_gas_auth_total,
            state_gas_auth_base,
            state_refund: 0,
            intrinsic_state_gas: 0,
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
            crypto,
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

    /// EIP-8037: Charge state gas, drawing from reservoir first, spilling to gas_remaining if exhausted.
    ///
    /// Must only be called for Amsterdam+ forks. All call sites must guard with
    /// `fork >= Fork::Amsterdam` before invoking this method.
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "arithmetic proven safe by min()"
    )]
    pub fn increase_state_gas(&mut self, gas: u64) -> Result<(), VMError> {
        debug_assert!(
            self.env.config.fork >= Fork::Amsterdam,
            "increase_state_gas called pre-Amsterdam"
        );
        // Draw from reservoir first; only spill to gas_remaining if reservoir exhausted
        let from_reservoir = self.state_gas_reservoir.min(gas);
        // Safe: from_reservoir <= gas
        let spill = gas - from_reservoir;
        if spill > 0 {
            // Charge spill from gas_remaining first — if OOG, return early
            // without mutating reservoir or state_gas_used (matches EELS behavior)
            self.current_call_frame.increase_consumed_gas(spill)?;
        }
        // Safe: from_reservoir = min(reservoir, gas) so reservoir >= from_reservoir
        self.state_gas_reservoir -= from_reservoir;
        // Only increment state_gas_used AFTER the charge succeeds.
        // state_gas_used is i64; tx gas_limit caps charges well below i64::MAX.
        self.state_gas_used = self
            .state_gas_used
            .checked_add(i64::try_from(gas).map_err(|_| InternalError::Overflow)?)
            .ok_or(InternalError::Overflow)?;
        // Track the spill for block-accounting: EELS charge_state_gas spills
        // don't count toward regular_gas_used for the regular dimension.
        self.state_gas_spill = self
            .state_gas_spill
            .checked_add(spill)
            .ok_or(InternalError::Overflow)?;
        Ok(())
    }

    /// EIP-8037: credit `amount` directly to the local frame's reservoir; `state_gas_used`
    /// may go negative when the matching charge lives in an ancestor frame.
    ///
    /// Must only be called for Amsterdam+ forks.
    pub fn credit_state_gas_refund(&mut self, amount: u64) -> Result<(), VMError> {
        debug_assert!(
            self.env.config.fork >= Fork::Amsterdam,
            "credit_state_gas_refund called pre-Amsterdam"
        );
        self.state_gas_reservoir = self
            .state_gas_reservoir
            .checked_add(amount)
            .ok_or(InternalError::Overflow)?;
        self.state_gas_used = self
            .state_gas_used
            .checked_sub(i64::try_from(amount).map_err(|_| InternalError::Overflow)?)
            .ok_or(InternalError::Overflow)?;
        Ok(())
    }

    /// EIP-8037 `incorporate_child_on_error`: on child revert, restore the parent's
    /// `state_gas_used` to its pre-child value and refund the child's net
    /// `(state_gas_used + state_gas_left)` back into the parent's reservoir.
    ///
    /// In ethrex's shared-VM model the child holds the entire reservoir during its
    /// execution, so `child.state_gas_left == self.state_gas_reservoir` (absolute,
    /// not a delta against entry). `child.state_gas_used` can be negative when
    /// inline refunds inside the child exceeded its gross charges.
    pub fn incorporate_child_state_gas_on_revert(
        &mut self,
        state_gas_used_at_entry: i64,
    ) -> Result<(), VMError> {
        let child_state_gas_used = self
            .state_gas_used
            .checked_sub(state_gas_used_at_entry)
            .ok_or(InternalError::Overflow)?;
        let child_state_gas_left =
            i64::try_from(self.state_gas_reservoir).map_err(|_| InternalError::Overflow)?;
        self.state_gas_used = state_gas_used_at_entry;
        let net_return = child_state_gas_used
            .checked_add(child_state_gas_left)
            .ok_or(InternalError::Overflow)?;
        // net_return is always >= 0 by the spec invariant (reservoir conservation
        // means a child cannot refund more than its ancestors charged); clamp
        // defensively and cast — `as u64` is sound because of the `.max(0)`.
        #[expect(clippy::as_conversions, reason = ".max(0) proves non-negativity")]
        {
            self.state_gas_reservoir = net_return.max(0) as u64;
        }
        Ok(())
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

        // Empty bytecode would only execute STOP; skip the dispatch loop.
        // The BAL checkpoint below is intentionally skipped: a codeless transfer cannot
        // fail past this point and has no inner calls, so there's nothing to roll back.
        if self.is_simple_transfer_fast_path() {
            #[expect(clippy::as_conversions, reason = "gas_remaining is non-negative here")]
            let gas_used = self
                .current_call_frame
                .gas_limit
                .checked_sub(self.current_call_frame.gas_remaining as u64)
                .ok_or(InternalError::Underflow)?;
            let context_result = ContextResult {
                result: TxResult::Success,
                gas_used,
                gas_spent: gas_used,
                output: Bytes::new(),
            };
            return self.finalize_execution(context_result);
        }

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

    /// Must run after `prepare_execution` so EIP-7702 delegation is already resolved into
    /// `bytecode`.
    #[inline(always)]
    fn is_simple_transfer_fast_path(&self) -> bool {
        !self.current_call_frame.is_create
            && self.current_call_frame.bytecode.bytecode.is_empty()
            // Privileged L2 txs can leave gas negative; let the slow path surface that as OOG.
            && self.current_call_frame.gas_remaining >= 0
            && self.tx.authorization_list().is_none()
            // Precompiles dispatch via run_execution even with empty bytecode.
            && !precompiles::is_precompile(
                &self.current_call_frame.to,
                self.env.config.fork,
                self.vm_type,
            )
    }

    /// Main execution loop.
    pub fn run_execution(&mut self) -> Result<ContextResult, VMError> {
        // If gas is already exhausted (negative), fail immediately.
        // This can happen when intrinsic gas exceeds the gas limit in privileged L2 transactions.
        // Without this check, casting negative gas_remaining to u64 would wrap to a huge value.
        if self.current_call_frame.gas_remaining < 0 {
            return Ok(ContextResult {
                result: TxResult::Revert(ExceptionalHalt::OutOfGas.into()),
                gas_used: self.current_call_frame.gas_limit,
                gas_spent: self.current_call_frame.gas_limit,
                output: Bytes::new(),
            });
        }

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
                self.crypto,
            );

            // EIP-8037 Amsterdam 2D accounting recomputes `block_gas_used` from
            // `raw_consumed = gas_limit - gas_remaining` inside `refund_sender`. On a
            // top-level precompile exceptional halt, `handle_precompile_result` already
            // sets `ContextResult.gas_used = gas_limit`, but `gas_remaining` retains the
            // untouched forwarded amount — under Amsterdam that would make the block
            // report only the intrinsic portion. Zero it here so the block matches the
            // `gas_used = gas_limit` contract from `handle_precompile_result`. Pre-Amsterdam
            // reads `ctx_result.gas_used` directly and is unaffected by this path either way.
            if self.env.config.fork >= Fork::Amsterdam
                && let Ok(ctx) = &result
                && !ctx.is_success()
            {
                gas_remaining = 0;
            }

            call_frame.gas_remaining = gas_remaining as i64;

            return result;
        }

        let mut error = OnceCell::<VMError>::new();

        #[cfg(feature = "perf_opcode_timings")]
        let mut timings = crate::timings::OPCODE_TIMINGS.lock().expect("poison");

        loop {
            // Capture pc BEFORE advance_pc(1) — this is the address of the current opcode.
            let pc_of_current_op = self.current_call_frame.pc;
            let opcode = self.current_call_frame.next_opcode();
            self.advance_pc(1)?;

            // Hoist the active flag to avoid reading it twice per opcode.
            let tracer_active = self.opcode_tracer.active;

            // Struct-log pre-step capture (single branch on the fast path when disabled).
            let gas_before_op = if tracer_active {
                #[expect(
                    clippy::as_conversions,
                    reason = "gas_remaining is i64; clamp to 0 before converting to u64"
                )]
                let gas_before = self.current_call_frame.gas_remaining.max(0) as u64;
                #[expect(
                    clippy::as_conversions,
                    reason = "call depth bounded by STACK_LIMIT=1024, fits in u32"
                )]
                let depth = (self.call_frames.len() as u32).saturating_add(1);
                let refund = self.substate.refunded_gas;
                let stack_view = self.collect_stack_for_trace();
                let mem_view = self.collect_memory_for_trace();
                // mem_size always reflects actual memory size, regardless of enable_memory.
                #[expect(
                    clippy::as_conversions,
                    reason = "memory size is bounded by gas; fits in u64"
                )]
                let mem_size_for_trace = self.current_call_frame.memory.len() as u64;
                let storage_kv = self.read_storage_for_trace(opcode);
                let return_data = if self.opcode_tracer.cfg.enable_return_data {
                    self.current_call_frame.sub_return_data.clone()
                } else {
                    Bytes::new()
                };
                #[expect(
                    clippy::as_conversions,
                    reason = "pc is usize, fits in u64 on supported targets"
                )]
                let pc_u64 = pc_of_current_op as u64;
                self.opcode_tracer.pre_step_capture(
                    pc_u64,
                    opcode,
                    gas_before,
                    depth,
                    refund,
                    &stack_view,
                    &mem_view,
                    mem_size_for_trace,
                    &return_data,
                    storage_kv,
                );
                gas_before
            } else {
                0
            };

            #[cfg(feature = "perf_opcode_timings")]
            let opcode_time_start = std::time::Instant::now();

            // Fast path for common opcodes
            #[allow(clippy::indexing_slicing, clippy::as_conversions)]
            let op_result = self.opcode_table[opcode as usize].call(self, &mut error);

            #[cfg(feature = "perf_opcode_timings")]
            {
                let time = opcode_time_start.elapsed();
                timings.update(opcode, time);
            }

            // Struct-log post-step: patch gas_cost, refund-after-op, and error
            // into the buffered entry.
            if tracer_active {
                #[expect(
                    clippy::as_conversions,
                    reason = "gas_remaining is i64; clamp to 0 before converting to u64"
                )]
                let gas_after = self.current_call_frame.gas_remaining.max(0) as u64;
                // Prefer the explicit opcode-overhead cost written by CALL/CREATE handlers;
                // fall back to the gas diff for all other opcodes.
                let gas_cost = self
                    .opcode_tracer
                    .last_opcode_gas_cost
                    .take()
                    .unwrap_or_else(|| gas_before_op.saturating_sub(gas_after));
                // refund-after-op matches geth's structLogger timing: for SSTORE and
                // (pre-London) SELFDESTRUCT, the refund counter shown is the value
                // *after* the opcode's accounting applied. Other opcodes don't touch
                // refund, so the post-op value equals the captured pre-op value.
                let refund_after = self.substate.refunded_gas;
                let err_str = error.get().map(|e| e.to_string());
                self.opcode_tracer
                    .finalize_step(gas_cost, refund_after, err_str.as_deref());
            }

            let result = match op_result {
                OpcodeResult::Continue => continue,
                OpcodeResult::Halt => match error.take() {
                    None => self.handle_opcode_result()?,
                    Some(error) => self.handle_opcode_error(error)?,
                },
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
        crypto: &dyn Crypto,
    ) -> Result<ContextResult, VMError> {
        Self::handle_precompile_result(
            precompiles::execute_precompile(
                code_address,
                calldata,
                gas_remaining,
                fork,
                cache,
                crypto,
            ),
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
        // EIP-8037: On top-level tx failure (REVERT, ExceptionalHalt, or OOG),
        // refund only the EXECUTION portion of state gas to the reservoir; the intrinsic
        // stays in `state_gas_used` so block accounting bills it. EELS keeps these in
        // separate fields (`tx_output.state_gas_used` vs `tx_env.intrinsic_state_gas`);
        // ethrex lumps them so we split on the way out:
        //   tx_output.state_gas_left += tx_output.state_gas_used
        //   tx_output.state_gas_used  = 0
        // becomes in lumped form (with intrinsic preserved):
        //   reservoir   += signed(state_gas_used − intrinsic)   [clamped at 0]
        //   state_gas_used = intrinsic
        // Collision is handled separately in the hook.
        if self.env.config.fork >= Fork::Amsterdam && !ctx_result.is_success() {
            if !ctx_result.is_collision() {
                let intrinsic_signed =
                    i64::try_from(self.intrinsic_state_gas).map_err(|_| InternalError::Overflow)?;
                let execution_state_gas_used = self.state_gas_used.saturating_sub(intrinsic_signed);
                let reservoir_signed = i64::try_from(self.state_gas_reservoir)
                    .map_err(|_| InternalError::Overflow)?
                    .saturating_add(execution_state_gas_used);
                self.state_gas_reservoir =
                    u64::try_from(reservoir_signed.max(0)).map_err(|_| InternalError::Overflow)?;
                self.state_gas_used = intrinsic_signed;
            }

            // EIP-8037: on ANY top-level CREATE-tx
            // failure (revert / halt / OOG / collision), refund the intrinsic
            // `STATE_BYTES_PER_NEW_ACCOUNT * cost_per_state_byte` charge to the reservoir.
            // Also add to `state_refund` so block-level accounting subtracts it.
            // EELS reference: fork.py::process_transaction:
            //   if isinstance(tx.to, Bytes0):
            //       new_account_refund = STATE_BYTES_PER_NEW_ACCOUNT * COST_PER_STATE_BYTE
            //       tx_output.state_gas_left += new_account_refund
            //       tx_output.state_refund   += new_account_refund
            if self.is_create()? {
                let new_account_refund = self.state_gas_new_account;
                self.state_gas_reservoir = self
                    .state_gas_reservoir
                    .checked_add(new_account_refund)
                    .ok_or(InternalError::Overflow)?;
                self.state_refund = self
                    .state_refund
                    .checked_add(new_account_refund)
                    .ok_or(InternalError::Overflow)?;
            }
        }

        for hook in self.hooks.clone() {
            hook.borrow_mut()
                .finalize_execution(self, &mut ctx_result)?;
        }

        self.tracer.exit_context(&ctx_result, true)?;

        // Struct-log end-of-tx capture: record final output, gas used, and revert error.
        // gas matches geth's `executionResult.Gas` which is post-refund (`receipt.GasUsed`).
        if self.opcode_tracer.active {
            self.opcode_tracer.output = ctx_result.output.clone();
            self.opcode_tracer.gas_used = ctx_result.gas_spent;
            self.opcode_tracer.error = match ctx_result.result {
                TxResult::Revert(ref err) => Some(err.to_string()),
                _ => None,
            };
        }

        // Only include logs if transaction succeeded. When a transaction reverts,
        // no logs should be emitted (including EIP-7708 Transfer logs).
        let logs = if ctx_result.is_success() {
            self.substate.extract_logs()
        } else {
            Vec::new()
        };

        // EIP-8037: `state_gas_used` is already net (signed; credits
        // decrement it inline). Subtract `state_refund` (EIP-7702 tx-level channel) and
        // clamp at zero for block accounting — `state_gas_used` may be negative when inline
        // refunds exceed gross charges.
        let state_refund_signed =
            i64::try_from(self.state_refund).map_err(|_| InternalError::Overflow)?;
        let net_state_gas_used: u64 = u64::try_from(
            self.state_gas_used
                .saturating_sub(state_refund_signed)
                .max(0),
        )
        .map_err(|_| InternalError::Overflow)?;

        let report = ExecutionReport {
            result: ctx_result.result.clone(),
            gas_used: ctx_result.gas_used,
            gas_spent: ctx_result.gas_spent,
            gas_refunded: self.substate.refunded_gas,
            state_gas_used: net_state_gas_used,
            output: std::mem::take(&mut ctx_result.output),
            logs,
        };

        Ok(report)
    }

    // ── Struct-log helper methods ─────────────────────────────────────────────

    /// Collects the current stack in bottom-first order for struct-log emission.
    ///
    /// LEVM stack is top-first in memory (`values[offset]` = top), so we reverse
    /// the active slice to produce the bottom-first wire format geth uses.
    /// Returns an empty `Vec` when `cfg.disable_stack` is true.
    pub fn collect_stack_for_trace(&self) -> Vec<U256> {
        use crate::constants::STACK_LIMIT;
        if self.opcode_tracer.cfg.disable_stack {
            return Vec::new();
        }
        let s = &self.current_call_frame.stack;
        // offset <= STACK_LIMIT by stack invariant.
        s.values
            .get(s.offset..STACK_LIMIT)
            .map(|slice| slice.iter().rev().copied().collect())
            .unwrap_or_default()
    }

    /// Collects the live memory bytes for the current frame.
    ///
    /// Returns an empty `Vec` when `cfg.enable_memory` is false or memory is empty.
    pub fn collect_memory_for_trace(&self) -> Vec<u8> {
        if !self.opcode_tracer.cfg.enable_memory {
            return Vec::new();
        }
        self.current_call_frame.memory.live_bytes()
    }

    /// Pre-reads the storage key/value for the current SLOAD or SSTORE opcode.
    ///
    /// Returns `None` when:
    /// - `cfg.disable_storage` is set, or
    /// - `opcode` is not SLOAD (0x54) or SSTORE (0x55), or
    /// - the stack is empty (guard against underflow before the handler runs), or
    /// - the storage read fails for any reason (including `AccountNotFound` —
    ///   the trace omits the entry rather than emitting an ambiguous zero).
    ///
    /// For SLOAD: key = `stack.top`; value = the *current* stored value read from the DB.
    /// For SSTORE: key = `stack.top`, value = `stack[top-1]` (the new value being written).
    pub fn read_storage_for_trace(&mut self, opcode: u8) -> Option<(H256, H256)> {
        const SLOAD: u8 = 0x54;
        const SSTORE: u8 = 0x55;

        if self.opcode_tracer.cfg.disable_storage {
            return None;
        }
        if opcode != SLOAD && opcode != SSTORE {
            return None;
        }

        // Need at least one element on stack for SLOAD, two for SSTORE.
        use crate::constants::STACK_LIMIT;
        let offset = self.current_call_frame.stack.offset;
        if offset >= STACK_LIMIT {
            return None; // stack empty
        }

        // SLOAD/SSTORE operate on the call's storage context (`to`), not the code's
        // address. Under DELEGATECALL/CALLCODE these differ.
        let addr = self.current_call_frame.to;

        let stack_values = &self.current_call_frame.stack.values;
        let key_u256 = *stack_values.get(offset)?;
        let key = BigEndianHash::from_uint(&key_u256);

        if opcode == SLOAD {
            // Omit the entry on any read failure (incl. account not yet cached);
            // a zero value would be indistinguishable from a legitimate never-written slot.
            let v = self.get_storage_value(addr, key).ok()?;
            let value = BigEndianHash::from_uint(&v);
            Some((key, value))
        } else {
            // SSTORE: need two stack elements.
            let next_offset = offset.checked_add(1)?;
            if next_offset >= STACK_LIMIT {
                return None;
            }
            // values[offset+1] is the new value being written (second from top = stack[top-1]).
            let value_u256 = *self.current_call_frame.stack.values.get(next_offset)?;
            let value = BigEndianHash::from_uint(&value_u256);
            Some((key, value))
        }
    }
}

impl Substate {
    /// Initializes the VM substate, mainly adding addresses to the "accessed_addresses" field and the same with storage slots
    pub fn initialize(env: &Environment, tx: &Transaction) -> Result<Substate, VMError> {
        // Add sender and recipient to accessed accounts [https://www.evm.codes/about#access_list]
        // Pre-size the accessed-address set: it is never empty (sender + coinbase + precompiles,
        // min ~19) and p99 is 24, so a capacity of 32 covers >99.6% of txs without reallocating
        // while leaving headroom for the access-list tail. The precompile floor (~20) dominates
        // here; see Workstream D2, which removes those inserts and shrinks this hint.
        let mut initial_accessed_addresses =
            FxHashSet::with_capacity_and_hasher(32, Default::default());
        // Storage slots are ~98% empty (p95 0, p99 4), so `default()` (alloc-free until first
        // insert) beats pre-sizing, which would tax the common empty case.
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
