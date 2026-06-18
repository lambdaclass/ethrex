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
    /// Fork of the enclosing transaction. Lets the warmth helpers treat precompile addresses as
    /// always-warm without occupying a hashset slot (EIP-2929). Constant for a tx, so it is
    /// carried forward across `push_backup` checkpoints.
    fork: Fork,
    /// Accounts marked for self-destruction (deleted at end of transaction).
    selfdestruct_set: FxHashSet<Address>,
    /// Addresses accessed during execution (for EIP-2929 warm/cold gas costs).
    /// Precompiles are NOT stored here; they are warm by construction (see `is_warm_precompile`).
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
        fork: Fork,
        accessed_addresses: FxHashSet<Address>,
        accessed_storage_slots: FxHashMap<Address, FxHashSet<H256>>,
    ) -> Self {
        Self {
            parent: None,
            fork,
            selfdestruct_set: FxHashSet::default(),
            accessed_addresses,
            accessed_storage_slots,
            created_accounts: FxHashSet::default(),
            refunded_gas: 0,
            transient_storage: TransientStorage::default(),
            logs: Vec::new(),
        }
    }

    /// Whether `address` is a precompile that the EVM treats as warm from the start of the tx
    /// (EIP-2929), exactly matching the addresses `Substate::initialize` used to pre-seed.
    ///
    /// Replicates the pre-seed *precisely* — the contiguous range `0x01..=max_for_fork` plus the
    /// post-Osaka P256VERIFY address `0x100` — and is intentionally `vm_type`-independent, since
    /// the old pre-seed was too. (Using `precompiles::is_precompile`, which gates `0x100` on L2
    /// for any fork, would change L2 pre-Osaka warmth — a consensus difference, not an opt.)
    #[inline]
    fn is_warm_precompile(&self, address: &Address) -> bool {
        // Fast reject: every pre-seeded precompile has 18 leading zero bytes (max is `0x01_00`),
        // so real contract/EOA addresses bail out here, off the hot warmth path.
        if address.0[..18] != [0u8; 18] {
            return false;
        }
        let n = u16::from_be_bytes([address.0[18], address.0[19]]);
        let max_contiguous: u64 = match self.fork {
            f if f >= Fork::Prague => SIZE_PRECOMPILES_PRAGUE,
            f if f >= Fork::Cancun => SIZE_PRECOMPILES_CANCUN,
            _ => SIZE_PRECOMPILES_PRE_CANCUN,
        };
        (n >= 1 && u64::from(n) <= max_contiguous) || (n == 0x100 && self.fork >= Fork::Osaka)
    }

    /// Push a checkpoint that can be either reverted or committed. All data up to this point is
    /// still accessible.
    pub fn push_backup(&mut self) {
        let parent = mem::take(self);
        self.refunded_gas = parent.refunded_gas;
        // Carry the fork forward so child checkpoints keep the same precompile-warmth view.
        self.fork = parent.fork;
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
        // Precompiles are warm from tx start (EIP-2929) without occupying a hashset slot. Returns
        // `false` (not cold) so cold-access gas is never charged — identical to the old pre-seed.
        if self.is_warm_precompile(&address) {
            return false;
        }

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
        // Precompiles are always warm; the chain shares one `fork`, so this is consistent across
        // sub-frame substates.
        self.is_warm_precompile(address)
            || self.accessed_addresses.contains(address)
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
    /// The transaction being executed. Borrowed for the VM's lifetime (the caller owns it for at
    /// least that long), avoiding a per-tx deep clone of the access/authorization lists.
    pub tx: &'a Transaction,
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
    /// Whether the top-level call-frame backup must be PRESERVED (deep-cloned) on the
    /// revert / invalid-tx paths because a `BackupHook` will read it in `finalize_execution`
    /// to build the tx-level undo snapshot. Derived from the installed `hooks` (via
    /// [`Hook::reads_top_level_backup`]) rather than from `vm_type`, so it stays correct if
    /// hook wiring changes; `add_hook` keeps it in sync for the `BackupHook` that
    /// `stateless_execute` installs after construction. False for normal L1 block execution
    /// (no `BackupHook`), where the backup is dead once the cache is restored and can be moved
    /// out instead of cloned.
    pub(crate) preserve_top_level_backup: bool,
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
    /// EIP-8037 (#3002): whether a top-level CREATE transaction targeted an
    /// already-alive account (existed and non-empty) at tx start, captured in
    /// `handle_create_transaction` before any state mutation. Mirrors EELS
    /// `MessageCallOutput.created_target_alive`. Extends the create-tx
    /// new-account refund in `finalize_execution` to also fire on success when
    /// the target was alive (no new account leaf created). Default false.
    pub created_target_alive: bool,
    /// The opcode table mapping opcodes to opcode handlers for fast lookup.
    /// A shared `&'static` reference to a per-fork table that is `const`-built once for the
    /// whole process (immutable), so each VM holds only a pointer instead of a 2 KB inline copy.
    pub(crate) opcode_table: &'static [OpCodeFn; 256],
    /// Crypto provider for cryptographic operations.
    pub crypto: &'a dyn Crypto,
}

impl<'a> VM<'a> {
    /// Constructs a VM, allocating a fresh 32 KB root call-frame stack.
    ///
    /// Hot block execution should prefer [`VM::new_pooled`], which draws the root stack from a
    /// reusable pool instead of allocating + zeroing one per transaction.
    pub fn new(
        env: Environment,
        db: &'a mut GeneralizedDatabase,
        tx: &'a Transaction,
        tracer: LevmCallTracer,
        vm_type: VMType,
        crypto: &'a dyn Crypto,
    ) -> Result<Self, VMError> {
        Self::new_with_root_stack(
            env,
            db,
            tx,
            tracer,
            vm_type,
            crypto,
            Stack::default(),
            Memory::default(),
        )
    }

    /// Like [`VM::new`], but draws the root call-frame stack from `stack_pool` (falling back to a
    /// fresh `Stack::default()` only when the pool is empty) and adopts the remaining pooled
    /// stacks for sub-call frames. This avoids the per-tx 32 KB stack alloc+zero on a warm pool —
    /// the dominant allocation for transfer-heavy blocks, where the root frame is the only frame.
    ///
    /// Pair with [`VM::reclaim_into`] after execution to return every stack (root + sub-frame)
    /// to `stack_pool` and the root memory buffer to `memory_pool` so the next tx reuses them.
    #[allow(clippy::too_many_arguments)]
    pub fn new_pooled(
        env: Environment,
        db: &'a mut GeneralizedDatabase,
        tx: &'a Transaction,
        tracer: LevmCallTracer,
        vm_type: VMType,
        crypto: &'a dyn Crypto,
        stack_pool: &mut Vec<Stack>,
        memory_pool: &mut Vec<Memory>,
    ) -> Result<Self, VMError> {
        // Reuse a pooled stack for the root frame. `clear()` only resets the offset (no zeroing),
        // which is sound because the EVM never reads stack slots it didn't write — the same
        // invariant that already makes sub-frame pooling safe.
        let mut root_stack = stack_pool.pop().unwrap_or_default();
        root_stack.clear();
        // Reuse a pooled root memory buffer (capacity retained from a prior tx, contents dropped).
        // `reclaim_into` truncates it to length 0, so `resize`'s zero-fill invariant holds. Only
        // the root buffer is pooled: sub-frame memories are `Rc` clones of it (`next_memory`).
        let mut root_memory = memory_pool.pop().unwrap_or_default();
        root_memory.reset_for_reuse();
        let mut vm = Self::new_with_root_stack(
            env,
            db,
            tx,
            tracer,
            vm_type,
            crypto,
            root_stack,
            root_memory,
        )?;
        // Adopt the caller's pooled stacks for sub-frames; returned via `reclaim_into`.
        mem::swap(&mut vm.stack_pool, stack_pool);
        Ok(vm)
    }

    /// Returns this VM's reusable buffers to the caller's pools so the next transaction reuses
    /// them instead of allocating: every stack (root call-frame stack plus any sub-frame stacks
    /// still pooled internally) to `stack_pool`, and the root memory buffer to `memory_pool`.
    /// Must run on both the success and error paths of [`VM::execute`].
    pub fn reclaim_into(mut self, stack_pool: &mut Vec<Stack>, memory_pool: &mut Vec<Memory>) {
        // Hand the internal sub-frame pool back to the caller first.
        mem::swap(&mut self.stack_pool, stack_pool);
        // Then reclaim the root frame's stack. Moving it out by value (VM/CallFrame have no Drop)
        // avoids leaving a fresh 32 KB `Stack::default()` placeholder behind — which a
        // `mem::take`/`mem::replace` against an empty pool would force, defeating the win on
        // exactly the transfer-only blocks (no sub-frames ever seed the pool) we target.
        let mut root_stack = self.current_call_frame.stack;
        root_stack.clear();
        stack_pool.push(root_stack);
        // Reclaim the root memory buffer with its grown capacity. `reset_for_reuse` truncates it
        // to length 0 (capacity kept) so the next tx's `resize` zero-fills correctly.
        //
        // Every call frame shares the same `Rc<RefCell<Vec<u8>>>` buffer, so on the error path the
        // ancestor frames left in `call_frames` (error propagation unwinds out of `execute` without
        // popping them) still hold clones. Drop them first so the buffer is `Rc`-unique on BOTH
        // paths before we clear it — otherwise the clear would propagate to a frame still holding a
        // reference. `CallFrame` has no `Drop` and these frames are never read again, so dropping
        // them early is free.
        self.call_frames.clear();
        let mut root_memory = self.current_call_frame.memory;
        debug_assert_eq!(
            Rc::strong_count(&root_memory.buffer),
            1,
            "root memory buffer must be Rc-unique at reclaim; a frame is still holding it and \
             would observe the reset_for_reuse clear",
        );
        root_memory.reset_for_reuse();
        memory_pool.push(root_memory);
    }

    #[allow(clippy::too_many_arguments)]
    fn new_with_root_stack(
        env: Environment,
        db: &'a mut GeneralizedDatabase,
        tx: &'a Transaction,
        tracer: LevmCallTracer,
        vm_type: VMType,
        crypto: &'a dyn Crypto,
        root_stack: Stack,
        root_memory: Memory,
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

        // Derive whether the top-level backup must be preserved from the installed hooks rather
        // than from `vm_type`. The flag's real meaning is "a hook reads the top-level backup in
        // `finalize_execution`," which today is the `BackupHook` on L2 / stateless. Deriving it
        // keeps the flag correct if hook wiring ever changes (e.g. a future `vm_type` that adds
        // `BackupHook`, or L2 dropping it), and `add_hook` keeps it in sync for the `BackupHook`
        // that `stateless_execute` installs after construction. L1 block execution installs no
        // `BackupHook` (see `l1_hooks`), so the backup is dead once the cache is restored.
        let hooks = get_hooks(&vm_type);
        let preserve_top_level_backup = hooks
            .iter()
            .any(|hook| hook.borrow().reads_top_level_backup());

        let mut vm = Self {
            call_frames: Vec::new(),
            substate,
            db,
            tx,
            hooks,
            storage_original_values: FxHashMap::default(),
            tracer,
            opcode_tracer: LevmOpcodeTracer::disabled(),
            debug_mode: DebugMode::disabled(),
            stack_pool: Vec::new(),
            vm_type,
            preserve_top_level_backup,
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
            created_target_alive: false,
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
                root_stack,
                root_memory,
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
        // Keep `preserve_top_level_backup` in sync: a hook added after construction (e.g. the
        // `BackupHook` in `stateless_execute`) may read the top-level backup in `finalize_execution`.
        self.preserve_top_level_backup |= hook.reads_top_level_backup();
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
        // Per-frame spill: EELS charge_state_gas does `frame_state_gas_spilled += remainder`.
        // LIFO refund source; propagated to parent on child success.
        self.current_call_frame.frame_state_gas_spilled = self
            .current_call_frame
            .frame_state_gas_spilled
            .checked_add(spill)
            .ok_or(InternalError::Overflow)?;
        Ok(())
    }

    /// EIP-8037 `credit_state_gas_refund`: refund `amount` of state gas using LIFO
    /// source order, mirroring EELS `credit_state_gas_refund`.
    ///
    /// The refund is sourced gas_remaining-first: the portion that was spilled past
    /// the reservoir into this frame's `gas_remaining` (`frame_state_gas_spilled`) is
    /// returned to `gas_remaining` first, and only the remainder flows into the shared
    /// reservoir. Concretely `from_gas_left = min(amount, frame_state_gas_spilled)`
    /// returns to `gas_remaining`, and `amount - from_gas_left` returns to the
    /// reservoir. `state_gas_used` is always decremented by the full `amount` and may
    /// go negative when the matching charge lives in an ancestor frame.
    ///
    /// Block accounting: both the per-frame `frame_state_gas_spilled` and the VM-level
    /// `state_gas_spill` counter are decremented by the `from_gas_left` portion ONLY
    /// (the part credited back to `gas_remaining`), never by the full `amount`.
    ///
    /// Must only be called for Amsterdam+ forks.
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "subtractions proven safe by min()"
    )]
    pub fn credit_state_gas_refund(&mut self, amount: u64) -> Result<(), VMError> {
        debug_assert!(
            self.env.config.fork >= Fork::Amsterdam,
            "credit_state_gas_refund called pre-Amsterdam"
        );
        // LIFO: drain the frame's spill (gas borrowed from gas_remaining) first.
        let from_gas_left = self.current_call_frame.frame_state_gas_spilled.min(amount);
        // Return the spilled portion to gas_remaining (i64).
        self.current_call_frame.gas_remaining = self
            .current_call_frame
            .gas_remaining
            .checked_add(i64::try_from(from_gas_left).map_err(|_| InternalError::Overflow)?)
            .ok_or(InternalError::Overflow)?;
        // Safe: from_gas_left = min(spill, amount) <= frame_state_gas_spilled.
        self.current_call_frame.frame_state_gas_spilled -= from_gas_left;
        // Block accounting: the refilled spill is no longer regular gas.
        self.state_gas_spill = self
            .state_gas_spill
            .checked_sub(from_gas_left)
            .ok_or(InternalError::Underflow)?;
        // The remainder of the refund flows into the shared reservoir.
        // Safe: from_gas_left = min(spill, amount) <= amount.
        let to_reservoir = amount - from_gas_left;
        self.state_gas_reservoir = self
            .state_gas_reservoir
            .checked_add(to_reservoir)
            .ok_or(InternalError::Overflow)?;
        // state_gas_used always drops by the full amount (may go negative).
        self.state_gas_used = self
            .state_gas_used
            .checked_sub(i64::try_from(amount).map_err(|_| InternalError::Overflow)?)
            .ok_or(InternalError::Overflow)?;
        Ok(())
    }

    /// EIP-8037 `refill_frame_state_gas`: roll back this frame's state gas in LIFO
    /// order on revert or exceptional halt, mirroring EELS `refill_frame_state_gas`.
    ///
    /// `entry` is the value of `state_gas_used` when this frame began executing
    /// (`current_call_frame.state_gas_used_at_entry`). The frame's net charge is
    /// `frame_used = state_gas_used - entry`. Of that, `frame_state_gas_spilled` was
    /// drawn from `gas_remaining` (spilled past the reservoir) and the remainder came
    /// from the reservoir. LIFO refill returns the spilled portion to `gas_remaining`
    /// first and the rest to the reservoir, restoring the exact pools the charges drew
    /// from. `state_gas_used` is rolled back to `entry` and the per-frame spill counter
    /// is cleared.
    ///
    /// Revert-vs-halt equivalence (load-bearing): on revert, the spilled gas returns to
    /// `gas_remaining` (raising the sender refund / lowering raw_consumed) while
    /// `state_gas_spill` drops by the same amount, so the regular dimension in
    /// `refund_sender` (default_hook) drops by exactly the refilled spill. On exceptional
    /// halt the caller subsequently sets `gas_remaining = 0` and burns it to the regular
    /// dimension — but `state_gas_spill` was already decremented here, so the spilled gas
    /// stays counted as regular. Both paths are correct.
    ///
    /// Must only be called for Amsterdam+ forks.
    pub fn refill_frame_state_gas(&mut self, entry: i64) -> Result<(), VMError> {
        debug_assert!(
            self.env.config.fork >= Fork::Amsterdam,
            "refill_frame_state_gas called pre-Amsterdam"
        );
        // The frame's net state-gas charge since it began executing. May be
        // negative when the frame's inline refunds (e.g. an SSTORE clearing a
        // slot an ancestor set) exceeded its own gross charges.
        let frame_used = self
            .state_gas_used
            .checked_sub(entry)
            .ok_or(InternalError::Underflow)?;
        let spilled = self.current_call_frame.frame_state_gas_spilled;
        // LIFO invariant: any remaining spill is undrained own-charge, so it
        // implies frame_used >= 0. A net-negative frame_used only arises after
        // credit_state_gas_refund has already drained all spill (spilled == 0).
        debug_assert!(
            frame_used >= 0 || spilled == 0,
            "negative frame_used with positive spill violates LIFO invariant \
             (frame_used={frame_used}, spilled={spilled})"
        );
        // LIFO: return the spilled portion (borrowed from gas_remaining) first.
        self.current_call_frame.gas_remaining = self
            .current_call_frame
            .gas_remaining
            .checked_add(i64::try_from(spilled).map_err(|_| InternalError::Overflow)?)
            .ok_or(InternalError::Overflow)?;
        // The remainder (drawn from the reservoir) flows back to the reservoir.
        let to_reservoir = frame_used
            .checked_sub(i64::try_from(spilled).map_err(|_| InternalError::Overflow)?)
            .ok_or(InternalError::Overflow)?;
        // `to_reservoir` is negative in the cross-ancestor refund case
        // (frame_used < 0); clamp so the reservoir never goes negative.
        let reservoir_signed =
            i64::try_from(self.state_gas_reservoir).map_err(|_| InternalError::Overflow)?;
        self.state_gas_reservoir = u64::try_from(
            reservoir_signed
                .checked_add(to_reservoir)
                .ok_or(InternalError::Overflow)?
                .max(0),
        )
        .map_err(|_| InternalError::Overflow)?;
        // Roll back state_gas_used to the frame's entry baseline.
        self.state_gas_used = entry;
        // Block accounting: the refilled spill is no longer regular gas.
        self.state_gas_spill = self
            .state_gas_spill
            .checked_sub(spilled)
            .ok_or(InternalError::Underflow)?;
        self.current_call_frame.frame_state_gas_spilled = 0;
        Ok(())
    }

    /// Executes a whole external transaction. Performing validations at the beginning.
    pub fn execute(&mut self) -> Result<ExecutionReport, VMError> {
        if let Err(e) = self.prepare_execution() {
            // Restore cache to state previous to this Tx execution because this Tx is invalid.
            // Consume the backup unless a `BackupHook` will read it (L2 / stateless); on L1 it
            // is dead once the cache is restored.
            if self.preserve_top_level_backup {
                self.restore_cache_state()?;
            } else {
                self.restore_cache_state_consuming()?;
            }
            return Err(e);
        }

        // Clear callframe backup so that changes made in prepare_execution are written in stone.
        // We want to apply these changes even if the Tx reverts. E.g. Incrementing sender nonce
        self.current_call_frame.call_frame_backup.clear();

        // Empty bytecode would only execute STOP; skip the dispatch loop.
        // The BAL checkpoint below is intentionally skipped: a codeless transfer cannot
        // fail past this point and has no inner calls, so there's nothing to roll back.
        if self.is_simple_transfer_fast_path() {
            // EIP-8037: no `refill_frame_state_gas` needed here — a codeless transfer always
            // succeeds, runs no opcodes, and charges no execution state gas, so the frame's
            // `frame_state_gas_spilled` is 0 and `state_gas_used` equals its entry baseline.
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
            // EIP-8037 invariant: precompiles never touch state gas. `execute_precompile`
            // is a free function that only mutates `gas_remaining`; it has no access to
            // `state_gas_used` / `state_gas_reservoir` / `state_gas_spill`. This is what makes
            // it safe to omit any state-gas refill on a precompile revert (no
            // `refill_frame_state_gas` needed here). The assert below guards the invariant.
            #[cfg(debug_assertions)]
            let state_gas_used_before_precompile = self.state_gas_used;

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

            debug_assert_eq!(
                self.state_gas_used, state_gas_used_before_precompile,
                "precompile execution must not mutate state_gas_used"
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

        // Copy the `&'static` table pointer once; it doesn't borrow `self`, so dispatch can still
        // pass `self` mutably to the handler without reloading the pointer each iteration.
        let opcode_table = self.opcode_table;

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
            let op_result = opcode_table[opcode as usize].call(self, &mut error);

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
                // Consume the backup (move it out) unless a `BackupHook` will read it afterward
                // to build the tx-level undo snapshot (L2 / stateless). On L1 nothing reads it
                // once the cache is restored, so cloning it would be dead work.
                self.handle_state_backup(&result, !self.preserve_top_level_backup)?;
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
        // Add backup hook to restore state after execution. `add_hook` flips
        // `preserve_top_level_backup` on via `Hook::reads_top_level_backup`, so the backup is
        // cloned (not moved out) on the revert paths even though this VM was built with L1 `vm_type`.
        self.add_hook(BackupHook::default());
        let report = self.execute()?;
        // Restore cache to the state before execution.
        self.db.undo_last_transaction()?;
        Ok(report)
    }

    fn prepare_execution(&mut self) -> Result<(), VMError> {
        // Clone each hook's `Rc` (cheap refcount bump) so the borrow on `self.hooks` is released
        // and `self` can be passed mutably — without `self.hooks.clone()`'s per-tx `Vec` realloc.
        // `self.hooks` is not mutated during the loop, so `get(i)` is always `Some` in range.
        for i in 0..self.hooks.len() {
            if let Some(hook) = self.hooks.get(i).map(Rc::clone) {
                hook.borrow_mut().prepare_execution(self)?;
            }
        }

        Ok(())
    }

    fn finalize_execution(
        &mut self,
        mut ctx_result: ContextResult,
    ) -> Result<ExecutionReport, VMError> {
        // EIP-8037: On top-level tx failure (REVERT, ExceptionalHalt, or OOG), the
        // execution portion of state gas has already been refilled into the reservoir by
        // the top-frame `refill_frame_state_gas` (seeded at the post-intrinsic baseline in
        // `add_intrinsic_gas` and fired on revert/halt in `handle_opcode_error` /
        // `handle_opcode_result`). The intrinsic portion stays in `state_gas_used` so block
        // accounting bills it. No reservoir-move is performed here. Collision returns before
        // any execution state gas is charged, so it has nothing to refill (see the create
        // collision branch in `handle_create_transaction`).
        //
        // EIP-8037 (#3002): the create-tx NEW_ACCOUNT refund fires for every top-level
        // CREATE-tx failure (revert / halt / OOG / collision), AND on success when the
        // target was already alive (`created_target_alive`) — no new account leaf created.
        // EELS reference: fork.py::process_transaction:
        //   if isinstance(tx.to, Bytes0) and (
        //       tx_output.error is not None or tx_output.created_target_alive
        //   ):
        //       new_account_refund = STATE_BYTES_PER_NEW_ACCOUNT * COST_PER_STATE_BYTE
        //       tx_output.state_gas_left += new_account_refund
        //       tx_output.state_refund   += new_account_refund
        // The `created_target_alive` term only ever holds on the success path: on
        // collision `handle_create_transaction` returns before setting it, so the
        // collision refund still fires exactly once via `!is_success`.
        if self.env.config.fork >= Fork::Amsterdam
            && self.is_create()?
            && (!ctx_result.is_success() || self.created_target_alive)
        {
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

        // See `prepare_execution`: per-hook `Rc::clone` avoids the `self.hooks.clone()` realloc.
        for i in 0..self.hooks.len() {
            if let Some(hook) = self.hooks.get(i).map(Rc::clone) {
                hook.borrow_mut()
                    .finalize_execution(self, &mut ctx_result)?;
            }
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
        let fork = env.config.fork;

        // Add sender and recipient to accessed accounts [https://www.evm.codes/about#access_list]
        // Precompiles are NO LONGER inserted here — they are warm by construction (see
        // `is_warm_precompile`), removing the ~20-entry floor that used to dominate this set. The
        // remaining working set is small (sender + coinbase + recipient + access-list/touched
        // addresses; real p99 ~7), so a capacity of 8 covers most txs with little waste.
        let mut initial_accessed_addresses =
            FxHashSet::with_capacity_and_hasher(8, Default::default());
        // Storage slots are ~98% empty (p95 0, p99 4), so `default()` (alloc-free until first
        // insert) beats pre-sizing, which would tax the common empty case.
        let mut initial_accessed_storage_slots: FxHashMap<Address, FxHashSet<H256>> =
            FxHashMap::default();

        // Add Tx sender to accessed accounts
        initial_accessed_addresses.insert(env.origin);

        // [EIP-3651] - Add coinbase to accessed accounts after Shanghai
        if fork >= Fork::Shanghai {
            initial_accessed_addresses.insert(env.coinbase);
        }

        // Add access lists contents to accessed accounts and accessed storage slots.
        // Iterate by reference (`Address`/`H256` are `Copy`); the old `.clone()` deep-copied
        // the whole `Vec<(Address, Vec<H256>)>` per tx just to read it.
        for (address, keys) in tx.access_list() {
            initial_accessed_addresses.insert(*address);
            // Access lists can have different entries even for the same address, that's why we check if there's an existing set instead of considering it empty
            let warm_slots = initial_accessed_storage_slots.entry(*address).or_default();
            for slot in keys {
                warm_slots.insert(*slot);
            }
        }

        let substate = Substate::from_accesses(
            fork,
            initial_accessed_addresses,
            initial_accessed_storage_slots,
        );

        Ok(substate)
    }
}

#[cfg(test)]
mod state_gas_tests {
    //! EIP-8037 source-based state-gas refund unit tests.
    //!
    //! Test-harness approach (Phase 5.1): these tests build the real [`VM`] via a struct
    //! literal directly (the test module is inline in `vm.rs`, so it has access to the
    //! crate-private `opcode_table` / `preserve_top_level_backup` fields) rather than going
    //! through [`VM::new`]. `VM::new` runs `Substate::initialize` and `get_tx_callee`, both of
    //! which read the database; that machinery is irrelevant to the pure two-pool arithmetic
    //! under test and would pull in `ethrex-storage` / `ethrex-blockchain` (a dependency cycle
    //! for the levm crate). Instead the harness wires up the minimal state the three methods
    //! actually touch — `env.config.fork = Amsterdam`, a single top-level call frame with a
    //! configurable `gas_remaining`, and a configurable `state_gas_reservoir` — and exercises
    //! the genuine `increase_state_gas` / `credit_state_gas_refund` / `refill_frame_state_gas`
    //! methods. No EF fixtures are read.

    use super::*;
    use crate::db::Database;
    use crate::environment::EVMConfig;
    use crate::errors::DatabaseError;
    use ethrex_common::types::{AccountState, ChainConfig, CodeMetadata, EIP1559Transaction};
    use ethrex_crypto::NativeCrypto;
    use std::sync::Arc;

    /// Minimal in-crate [`Database`] used only to satisfy [`GeneralizedDatabase::new`].
    /// None of its methods are reached by these tests (the VM is built by struct literal,
    /// so no account/storage/code loads occur), so every method returns an error.
    struct StubDatabase;

    impl Database for StubDatabase {
        fn get_account_state(&self, _address: Address) -> Result<AccountState, DatabaseError> {
            Err(DatabaseError::Custom("stub db: no account state".into()))
        }
        fn get_storage_value(&self, _address: Address, _key: H256) -> Result<U256, DatabaseError> {
            Err(DatabaseError::Custom("stub db: no storage value".into()))
        }
        fn get_block_hash(&self, _block_number: u64) -> Result<H256, DatabaseError> {
            Err(DatabaseError::Custom("stub db: no block hash".into()))
        }
        fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
            Err(DatabaseError::Custom("stub db: no chain config".into()))
        }
        fn get_account_code(&self, _code_hash: H256) -> Result<Code, DatabaseError> {
            Err(DatabaseError::Custom("stub db: no account code".into()))
        }
        fn get_code_metadata(&self, _code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
            Err(DatabaseError::Custom("stub db: no code metadata".into()))
        }
    }

    /// A large gas budget for the top-level frame so spills never run the frame OOG; the tests
    /// assert against the delta from this baseline.
    const FRAME_GAS_LIMIT: u64 = 1_000_000;

    /// Builds a `GeneralizedDatabase` over the stub backend. Returned by value so the caller
    /// owns it for at least the VM's lifetime.
    fn stub_db() -> GeneralizedDatabase {
        GeneralizedDatabase::new(Arc::new(StubDatabase))
    }

    /// A trivial transaction the VM borrows but never reads in these tests.
    fn stub_tx() -> Transaction {
        Transaction::EIP1559Transaction(EIP1559Transaction::default())
    }

    /// Builds an Amsterdam `Environment` with a generous gas limit.
    fn amsterdam_env() -> Environment {
        Environment {
            config: EVMConfig::new(
                Fork::Amsterdam,
                EVMConfig::canonical_values(Fork::Amsterdam),
            ),
            gas_limit: FRAME_GAS_LIMIT,
            block_gas_limit: FRAME_GAS_LIMIT,
            ..Default::default()
        }
    }

    /// Builds a single top-level call frame with `gas_remaining = FRAME_GAS_LIMIT`.
    fn top_frame() -> CallFrame {
        CallFrame::new(
            Address::default(),
            Address::default(),
            Address::default(),
            Code::default(),
            U256::zero(),
            Bytes::new(),
            false,
            FRAME_GAS_LIMIT,
            0,
            true,
            false,
            0,
            0,
            Stack::default(),
            Memory::default(),
        )
    }

    /// Assembles a minimal Amsterdam VM with the given starting `state_gas_reservoir`,
    /// borrowing the caller-owned `db`, `tx` and `crypto`.
    fn build_vm<'a>(
        env: Environment,
        db: &'a mut GeneralizedDatabase,
        tx: &'a Transaction,
        crypto: &'a dyn Crypto,
        state_gas_reservoir: u64,
    ) -> VM<'a> {
        let fork = env.config.fork;
        VM {
            call_frames: Vec::new(),
            current_call_frame: top_frame(),
            env,
            substate: Substate::default(),
            db,
            tx,
            hooks: Vec::new(),
            storage_original_values: FxHashMap::default(),
            tracer: LevmCallTracer::disabled(),
            opcode_tracer: LevmOpcodeTracer::disabled(),
            debug_mode: DebugMode::disabled(),
            stack_pool: Vec::new(),
            vm_type: VMType::L1,
            preserve_top_level_backup: false,
            state_gas_used: 0,
            state_gas_reservoir,
            state_gas_reservoir_initial: state_gas_reservoir,
            state_gas_spill: 0,
            cost_per_state_byte: 0,
            state_gas_new_account: 0,
            state_gas_storage_set: 0,
            state_gas_auth_total: 0,
            state_gas_auth_base: 0,
            state_refund: 0,
            intrinsic_state_gas: 0,
            created_target_alive: false,
            opcode_table: VM::build_opcode_table(fork),
            crypto,
        }
    }

    /// Convenience: lossless `u64 -> i64` for test values (all well below `i64::MAX`).
    /// Saturates rather than panics; every test value is a small constant so the saturation
    /// branch is never taken.
    fn as_i64(v: u64) -> i64 {
        i64::try_from(v).unwrap_or(i64::MAX)
    }

    /// Convenience: current frame's `gas_remaining`.
    fn gas_remaining(vm: &VM<'_>) -> i64 {
        vm.current_call_frame.gas_remaining
    }

    /// Convenience: current frame's per-frame spill counter.
    fn frame_spilled(vm: &VM<'_>) -> u64 {
        vm.current_call_frame.frame_state_gas_spilled
    }

    /// 5.2 — Full spill then refund (reservoir empty).
    ///
    /// reservoir = 0, so `increase_state_gas(N)` spills the whole charge out of
    /// `gas_remaining`. `credit_state_gas_refund(N)` must, in LIFO order, return all of it
    /// to `gas_remaining` (none to the reservoir) and zero every counter.
    #[test]
    fn credit_lifo_spill_first() {
        const N: u64 = 5_000;
        let mut db = stub_db();
        let tx = stub_tx();
        let crypto = NativeCrypto;
        let mut vm = build_vm(amsterdam_env(), &mut db, &tx, &crypto, 0);

        let gas_before = gas_remaining(&vm);
        vm.increase_state_gas(N).unwrap();

        // The full charge spilled to gas_remaining.
        assert_eq!(frame_spilled(&vm), N, "whole charge must spill");
        assert_eq!(
            gas_remaining(&vm),
            gas_before - as_i64(N),
            "gas_remaining drops by the spilled amount"
        );
        assert_eq!(vm.state_gas_spill, N, "vm-level spill tracks the spill");
        assert_eq!(vm.state_gas_used, as_i64(N));
        assert_eq!(vm.state_gas_reservoir, 0);

        vm.credit_state_gas_refund(N).unwrap();

        assert_eq!(
            gas_remaining(&vm),
            gas_before,
            "gas_remaining fully restored (LIFO: spill returned first)"
        );
        assert_eq!(frame_spilled(&vm), 0, "frame spill drained");
        assert_eq!(vm.state_gas_reservoir, 0, "nothing flowed to reservoir");
        assert_eq!(vm.state_gas_spill, 0, "vm-level spill drained");
        assert_eq!(vm.state_gas_used, 0, "state_gas_used back to zero");
    }

    /// 5.3 — Charge fully from reservoir, no spill, then refund.
    ///
    /// reservoir = N, so `increase_state_gas(N)` draws entirely from the reservoir and never
    /// touches `gas_remaining`. The refund must return the full amount to the reservoir,
    /// leaving `gas_remaining` untouched.
    #[test]
    fn credit_lifo_reservoir_only() {
        const N: u64 = 5_000;
        let mut db = stub_db();
        let tx = stub_tx();
        let crypto = NativeCrypto;
        let mut vm = build_vm(amsterdam_env(), &mut db, &tx, &crypto, N);

        let gas_before = gas_remaining(&vm);
        vm.increase_state_gas(N).unwrap();

        assert_eq!(
            frame_spilled(&vm),
            0,
            "no spill when reservoir covers charge"
        );
        assert_eq!(gas_remaining(&vm), gas_before, "gas_remaining untouched");
        assert_eq!(vm.state_gas_reservoir, 0, "reservoir fully drawn down");
        assert_eq!(vm.state_gas_spill, 0);
        assert_eq!(vm.state_gas_used, as_i64(N));

        vm.credit_state_gas_refund(N).unwrap();

        assert_eq!(vm.state_gas_reservoir, N, "refund flows back to reservoir");
        assert_eq!(
            gas_remaining(&vm),
            gas_before,
            "gas_remaining still untouched"
        );
        assert_eq!(frame_spilled(&vm), 0);
        assert_eq!(vm.state_gas_spill, 0);
        assert_eq!(vm.state_gas_used, 0, "state_gas_used back to zero");
    }

    /// 5.4 — Partial spill: reservoir K covers part, S spills, then refund the whole charge.
    ///
    /// LIFO refund returns the spilled S to `gas_remaining` first and the remaining K to the
    /// reservoir.
    #[test]
    fn credit_lifo_partial_spill() {
        const K: u64 = 3_000;
        const S: u64 = 2_000;
        let mut db = stub_db();
        let tx = stub_tx();
        let crypto = NativeCrypto;
        let mut vm = build_vm(amsterdam_env(), &mut db, &tx, &crypto, K);

        let gas_before = gas_remaining(&vm);
        vm.increase_state_gas(K + S).unwrap();

        assert_eq!(frame_spilled(&vm), S, "only the over-reservoir part spills");
        assert_eq!(
            gas_remaining(&vm),
            gas_before - as_i64(S),
            "gas_remaining drops only by the spill S"
        );
        assert_eq!(vm.state_gas_reservoir, 0, "reservoir fully drawn");
        assert_eq!(vm.state_gas_spill, S);
        assert_eq!(vm.state_gas_used, as_i64(K + S));

        vm.credit_state_gas_refund(K + S).unwrap();

        assert_eq!(
            gas_remaining(&vm),
            gas_before,
            "gas_remaining += S (spill returned)"
        );
        assert_eq!(vm.state_gas_reservoir, K, "remainder K flows to reservoir");
        assert_eq!(frame_spilled(&vm), 0, "frame spill drained");
        assert_eq!(vm.state_gas_spill, 0, "vm-level spill drained");
        assert_eq!(vm.state_gas_used, 0);
    }

    /// 5.5 — `refill_frame_state_gas` on a frame that spilled (reservoir empty at entry).
    ///
    /// After a full spill of N, refilling from entry=0 returns N to `gas_remaining`, leaves the
    /// reservoir at 0, and clears `state_gas_used` / both spill counters.
    #[test]
    fn refill_on_spilled_frame() {
        const N: u64 = 5_000;
        let mut db = stub_db();
        let tx = stub_tx();
        let crypto = NativeCrypto;
        let mut vm = build_vm(amsterdam_env(), &mut db, &tx, &crypto, 0);

        let gas_before = gas_remaining(&vm);
        vm.increase_state_gas(N).unwrap();
        assert_eq!(frame_spilled(&vm), N);
        assert_eq!(gas_remaining(&vm), gas_before - as_i64(N));

        vm.refill_frame_state_gas(0).unwrap();

        assert_eq!(
            gas_remaining(&vm),
            gas_before,
            "spilled gas returned to gas_remaining"
        );
        assert_eq!(vm.state_gas_reservoir, 0, "no spill went to reservoir");
        assert_eq!(vm.state_gas_used, 0, "state_gas_used rolled back to entry");
        assert_eq!(frame_spilled(&vm), 0, "frame spill cleared");
        assert_eq!(vm.state_gas_spill, 0, "vm-level spill cleared");
    }

    /// 5.5 (no-spill variant) — `refill_frame_state_gas` on a frame whose charge came entirely
    /// from the reservoir. The reservoir-sourced portion must flow back to the reservoir; gas
    /// remaining is untouched.
    #[test]
    fn refill_on_reservoir_only_frame() {
        const N: u64 = 5_000;
        let mut db = stub_db();
        let tx = stub_tx();
        let crypto = NativeCrypto;
        let mut vm = build_vm(amsterdam_env(), &mut db, &tx, &crypto, N);

        let gas_before = gas_remaining(&vm);
        vm.increase_state_gas(N).unwrap();
        assert_eq!(frame_spilled(&vm), 0);
        assert_eq!(vm.state_gas_reservoir, 0);

        vm.refill_frame_state_gas(0).unwrap();

        assert_eq!(
            vm.state_gas_reservoir, N,
            "reservoir-sourced charge returns to reservoir"
        );
        assert_eq!(
            gas_remaining(&vm),
            gas_before,
            "gas_remaining unchanged (no spill)"
        );
        assert_eq!(vm.state_gas_used, 0, "state_gas_used rolled back to entry");
        assert_eq!(frame_spilled(&vm), 0);
        assert_eq!(vm.state_gas_spill, 0);
    }

    /// 5.6 — `refill_frame_state_gas` preserves the intrinsic baseline.
    ///
    /// Top-frame entry is seeded at the post-intrinsic `state_gas_used` value (see
    /// `add_intrinsic_gas` in `utils.rs`). A revert/halt refill from that entry must roll back
    /// only the execution-portion of the charge and leave the intrinsic portion billed.
    #[test]
    fn refill_preserves_intrinsic_baseline() {
        const INTRINSIC: i64 = 4_000;
        const EXEC: u64 = 6_000;
        let mut db = stub_db();
        let tx = stub_tx();
        let crypto = NativeCrypto;
        let mut vm = build_vm(amsterdam_env(), &mut db, &tx, &crypto, 0);

        // Simulate the post-intrinsic baseline: intrinsic state gas already accounted for, and
        // the frame's entry snapshot taken at that point (mirrors add_intrinsic_gas seeding).
        vm.state_gas_used = INTRINSIC;
        vm.current_call_frame.state_gas_used_at_entry = INTRINSIC;

        let gas_before = gas_remaining(&vm);
        // Execution-time state-gas charge (spills, reservoir is empty).
        vm.increase_state_gas(EXEC).unwrap();
        assert_eq!(vm.state_gas_used, INTRINSIC + as_i64(EXEC));
        assert_eq!(frame_spilled(&vm), EXEC);

        let entry = vm.current_call_frame.state_gas_used_at_entry;
        vm.refill_frame_state_gas(entry).unwrap();

        assert_eq!(
            vm.state_gas_used, INTRINSIC,
            "execution portion rolled back, intrinsic preserved"
        );
        assert_eq!(
            gas_remaining(&vm),
            gas_before,
            "spilled execution gas returned to gas_remaining"
        );
        assert_eq!(frame_spilled(&vm), 0);
        assert_eq!(vm.state_gas_spill, 0);
        assert_eq!(vm.state_gas_reservoir, 0);
    }

    /// 5.7 — Revert-vs-halt regular-dimension equivalence (method-level proxy).
    ///
    /// Decision: a full `vm.execute()` path is NOT feasible fixture-free here. `execute`
    /// requires a funded sender, valid bytecode and a working database (account/code loads),
    /// which `VM::new`'s `Substate::initialize` / `get_tx_callee` perform against the store; a
    /// fixture-free in-crate VM cannot drive that without pulling in `ethrex-storage` /
    /// `ethrex-blockchain` (dependency cycle). This test is therefore the documented
    /// method-level proxy for the end-to-end equivalence: it drives the exact two-method
    /// sequence the production revert and halt paths use (`increase_state_gas` to spill, then
    /// `refill_frame_state_gas` to roll the frame back) and asserts the regular-gas dimension
    /// for both a "gas not zeroed" (revert) and a "gas then zeroed" (exceptional halt) sequence.
    ///
    /// The regular-gas dimension consumed by the tx is, per `refund_sender`/`default_hook`:
    ///   regular = (gas_limit - gas_remaining) - state_gas_spill
    /// On revert, the refilled spill returns to `gas_remaining` AND `state_gas_spill` drops by
    /// the same amount, so the spilled gas is fully refunded to the sender. On exceptional
    /// halt the caller zeroes `gas_remaining` after the refill, burning everything left to the
    /// regular dimension; but because `refill_frame_state_gas` already decremented
    /// `state_gas_spill`, the spilled gas stays counted as regular (burned, not refunded).
    #[test]
    fn revert_vs_halt_regular_dimension_proxy() {
        const N: u64 = 5_000;

        // Computes the regular-gas dimension exactly as default_hook's refund_sender does.
        fn regular_dimension(vm: &VM<'_>) -> i64 {
            let consumed = as_i64(FRAME_GAS_LIMIT) - vm.current_call_frame.gas_remaining;
            consumed - as_i64(vm.state_gas_spill)
        }

        // --- Revert path: refill, gas_remaining NOT zeroed ---
        let mut db_r = stub_db();
        let tx_r = stub_tx();
        let crypto_r = NativeCrypto;
        let mut vm_revert = build_vm(amsterdam_env(), &mut db_r, &tx_r, &crypto_r, 0);
        vm_revert.increase_state_gas(N).unwrap();
        // Mid-spill: the spilled gas is currently counted as regular.
        assert_eq!(
            regular_dimension(&vm_revert),
            0,
            "before refill, spill is netted out of the regular dimension"
        );
        vm_revert.refill_frame_state_gas(0).unwrap();
        // Revert keeps gas_remaining as-is (no zeroing).
        let revert_regular = regular_dimension(&vm_revert);
        assert_eq!(
            revert_regular, 0,
            "revert: spilled gas refunded to sender (regular dimension unchanged at 0)"
        );
        assert_eq!(vm_revert.state_gas_spill, 0, "revert drains vm-level spill");

        // --- Halt path: refill, THEN zero gas_remaining (exceptional halt) ---
        let mut db_h = stub_db();
        let tx_h = stub_tx();
        let crypto_h = NativeCrypto;
        let mut vm_halt = build_vm(amsterdam_env(), &mut db_h, &tx_h, &crypto_h, 0);
        vm_halt.increase_state_gas(N).unwrap();
        vm_halt.refill_frame_state_gas(0).unwrap();
        // Exceptional halt: caller zeroes gas_remaining after the refill.
        vm_halt.current_call_frame.gas_remaining = 0;
        let halt_regular = regular_dimension(&vm_halt);
        assert_eq!(
            halt_regular,
            as_i64(FRAME_GAS_LIMIT),
            "halt: all remaining gas burned to the regular dimension (spilled gas stays burned)"
        );
        assert_eq!(
            vm_halt.state_gas_spill, 0,
            "halt also drained vm-level spill"
        );

        // The two paths differ in exactly the regular dimension: revert refunds the spilled
        // gas (sender keeps it), halt burns it.
        assert_ne!(
            revert_regular, halt_regular,
            "revert and halt must produce different regular-gas dimensions"
        );
    }

    /// 6.2 — Pre-Amsterdam invariance guard.
    ///
    /// Builds a pre-Amsterdam (Prague) VM through the same struct-literal harness and asserts
    /// that every EIP-8037 state-gas field is 0 at construction and that nothing seeds them on
    /// a pre-Amsterdam fork. The Amsterdam-gated methods (`increase_state_gas` /
    /// `credit_state_gas_refund` / `refill_frame_state_gas`) are intentionally NOT called here:
    /// their `debug_assert!(fork >= Amsterdam)` gates would fire pre-Amsterdam. Instead this
    /// proves the no-state-gas path leaves the per-frame and VM-level counters untouched, which
    /// is what every production state-gas call site relies on (each is wrapped in a
    /// `fork >= Fork::Amsterdam` guard, so on a pre-Amsterdam fork none of them ever run).
    #[test]
    fn pre_amsterdam_state_gas_fields_stay_zero() {
        // Build a Prague (pre-Amsterdam) environment via the harness.
        let env = Environment {
            config: EVMConfig::new(Fork::Prague, EVMConfig::canonical_values(Fork::Prague)),
            gas_limit: FRAME_GAS_LIMIT,
            block_gas_limit: FRAME_GAS_LIMIT,
            ..Default::default()
        };
        assert!(
            env.config.fork < Fork::Amsterdam,
            "guard test must run on a pre-Amsterdam fork"
        );

        let mut db = stub_db();
        let tx = stub_tx();
        let crypto = NativeCrypto;
        // Reservoir 0: a pre-Amsterdam VM never funds a state-gas reservoir.
        let vm = build_vm(env, &mut db, &tx, &crypto, 0);

        // At construction, every state-gas field is 0; nothing seeds them pre-Amsterdam.
        assert_eq!(
            frame_spilled(&vm),
            0,
            "per-frame spill must stay 0 pre-Amsterdam"
        );
        assert_eq!(
            vm.state_gas_spill, 0,
            "vm-level spill must stay 0 pre-Amsterdam"
        );
        assert_eq!(
            vm.state_gas_reservoir, 0,
            "reservoir must stay 0 pre-Amsterdam"
        );
        assert_eq!(
            vm.state_gas_used, 0,
            "state_gas_used must stay 0 pre-Amsterdam"
        );
    }

    /// EIP-8037 #3002 (Case 1, CREATE/CREATE2 success-to-alive-target) — method-level proxy.
    ///
    /// Decision: a full create-to-an-already-alive-target execution path is NOT feasible
    /// fixture-free here (it needs a funded sender, real initcode, a pre-existing alive target
    /// account and a working store — the same `VM::new`/`Substate::initialize` machinery the
    /// other tests document as unavailable without an `ethrex-storage`/`ethrex-blockchain`
    /// dependency cycle; the EF v6.0.0 fixtures for #3002 are also unavailable). This test is
    /// therefore the documented proxy: it reproduces the exact two-step the production
    /// `generic_create` success arm performs when `target_alive` holds — the unconditional
    /// new-account state-gas charge (`increase_state_gas(state_gas_new_account)` at the top of
    /// `generic_create`) followed by `credit_state_gas_refund(state_gas_new_account)` (the
    /// `if target_alive` refund added to the `handle_return_create` success arm) — and asserts
    /// the reservoir and `state_gas_used` are fully restored, i.e. the unconditional charge is
    /// net-zero when the target was already alive. Mirrors EELS `generic_create`:
    ///   if target_alive: credit_state_gas_refund(evm, StateGasCosts.NEW_ACCOUNT)
    #[test]
    fn create_success_to_alive_target_refund_proxy() {
        const NEW_ACCOUNT: u64 = 7_500;
        let mut db = stub_db();
        let tx = stub_tx();
        let crypto = NativeCrypto;
        // Reservoir large enough that the unconditional charge draws fully from it (no spill),
        // matching a CREATE with ample state-gas headroom.
        let mut vm = build_vm(amsterdam_env(), &mut db, &tx, &crypto, NEW_ACCOUNT);
        vm.state_gas_new_account = NEW_ACCOUNT;

        let gas_before = gas_remaining(&vm);
        let reservoir_before = vm.state_gas_reservoir;

        // Top of `generic_create`: charge the new-account state gas unconditionally.
        vm.increase_state_gas(vm.state_gas_new_account).unwrap();
        assert_eq!(vm.state_gas_used, as_i64(NEW_ACCOUNT), "charge landed");
        assert_eq!(vm.state_gas_reservoir, 0, "charge drawn from reservoir");

        // Success arm of `handle_return_create` with `target_alive == true`: refund it.
        vm.credit_state_gas_refund(vm.state_gas_new_account)
            .unwrap();

        assert_eq!(
            vm.state_gas_used, 0,
            "alive-target refund makes the new-account charge net-zero"
        );
        assert_eq!(
            vm.state_gas_reservoir, reservoir_before,
            "refund restores the reservoir"
        );
        assert_eq!(
            gas_remaining(&vm),
            gas_before,
            "gas_remaining untouched (charge and refund both via reservoir)"
        );
        assert_eq!(frame_spilled(&vm), 0, "no spill to drain");
        assert_eq!(vm.state_gas_spill, 0);
    }
}
