use crate::{
    account::LevmAccount,
    constants::STACK_LIMIT,
    errors::{ExceptionalHalt, InternalError, VMError},
    memory::Memory,
    utils::restore_cache_state,
    vm::VM,
};
use bytes::Bytes;
use ethrex_common::types::block_access_list::BlockAccessListCheckpoint;
use ethrex_common::{Address, H256, U256, types::Code};
use rustc_hash::FxHashMap;
use std::{
    fmt,
    hash::{Hash, Hasher},
    hint::assert_unchecked,
};

/// [`u64`]s that make up a [`U256`]
const U64_PER_U256: usize = U256::MAX.0.len();

#[derive(Clone, PartialEq, Eq)]
/// The EVM uses a stack-based architecture and does not use registers like some other VMs.
///
/// The specification says the stack is limited to 1024 items, aka. 32KiB, which is reasonable
/// enough for allocating it all at once to make sense. Every time an item is pushed into the stack,
/// its bounds have to be checked; by making the stack grow downwards, the underflow detection of
/// the offset update operation can also be reused to check for stack overflow.
///
/// A few opcodes require pushing and/or popping multiple elements. The [`push`](Self::push) and
/// [`pop`](Self::pop) methods support working with multiple elements instead of a single one,
/// reducing the number of checks performed on the stack.
pub struct Stack {
    pub values: Box<[U256; STACK_LIMIT]>,
    pub offset: usize,
}

impl Stack {
    #[inline]
    pub fn pop<const N: usize>(&mut self) -> Result<&[U256; N], ExceptionalHalt> {
        // Compile-time check for stack underflow.
        const {
            assert!(N <= STACK_LIMIT);
        }

        // The following operation can never overflow as both `self.offset` and N are within
        // STACK_LIMIT (1024).
        let next_offset = self.offset.wrapping_add(N);

        // The index cannot fail because `self.offset` is known to be valid. The `first_chunk()`
        // method will ensure that `next_offset` is within `STACK_LIMIT`, so there's no need to
        // check it again.
        #[expect(unsafe_code)]
        let values = unsafe {
            self.values
                .get_unchecked(self.offset..)
                .first_chunk::<N>()
                .ok_or(ExceptionalHalt::StackUnderflow)?
        };
        // Due to previous error check in first_chunk, next_offset is guaranteed to be < STACK_LIMIT
        self.offset = next_offset;

        Ok(values)
    }

    #[inline]
    pub fn pop1(&mut self) -> Result<U256, ExceptionalHalt> {
        let value = *self
            .values
            .get(self.offset)
            .ok_or(ExceptionalHalt::StackUnderflow)?;
        // The following operation can never overflow as both `self.offset` and N are within
        // STACK_LIMIT (1024).
        self.offset = self.offset.wrapping_add(1);

        Ok(value)
    }

    /// Mutable reference to the top item without changing depth (one underflow check,
    /// no `offset` write). For stack-neutral unary ops (ISZERO, NOT, CLZ), replacing
    /// `pop1` + `push` with `*top_mut() = f(*top)` avoids the read-modify-write of the
    /// shared `offset`, which is the per-opcode serial dependency that pins dispatch IPC.
    #[inline]
    pub fn top_mut(&mut self) -> Result<&mut U256, ExceptionalHalt> {
        self.values
            .get_mut(self.offset)
            .ok_or(ExceptionalHalt::StackUnderflow)
    }

    /// Pop the top value and return it together with a mutable reference to the new top.
    /// For binary ops: `let (a, b) = pop1_and_top_mut()?; *b = f(a, *b)` writes the result
    /// in place (one `offset` write instead of `pop::<2>` + `push`'s two), where `a` is the
    /// original top and `*b` the original second operand.
    #[inline]
    pub fn pop1_and_top_mut(&mut self) -> Result<(U256, &mut U256), ExceptionalHalt> {
        let a = self.pop1()?;
        Ok((a, self.top_mut()?))
    }

    /// Push a single U256 value to the stack, faster than the generic push.
    #[inline]
    pub fn push(&mut self, value: U256) -> Result<(), ExceptionalHalt> {
        // Since the stack grows downwards, when an offset underflow is detected the stack is
        // overflowing.
        let next_offset = self
            .offset
            .checked_sub(1)
            .ok_or(ExceptionalHalt::StackOverflow)?;

        // The following index cannot fail because `next_offset` has already been checked and
        // `self.offset` is known to be within `STACK_LIMIT`.
        // Store each limb individually so LLVM treats them as 4 independent i64 scalars.
        // This prevents LLVM from grouping limbs[1..3] into a [24 x i8] alloca that would
        // then need a memset + memcpy round-trip for values with known-zero upper limbs
        // (e.g. PUSH1-PUSH31), allowing it to emit direct zero stores instead.
        #[expect(unsafe_code, reason = "next_offset == self.offset - 1 >= 0")]
        unsafe {
            let slot = self.values.get_unchecked_mut(next_offset);
            slot.0[0] = value.0[0];
            slot.0[1] = value.0[1];
            slot.0[2] = value.0[2];
            slot.0[3] = value.0[3];
        }
        self.offset = next_offset;

        Ok(())
    }

    #[inline]
    pub fn push_zero(&mut self) -> Result<(), ExceptionalHalt> {
        // Since the stack grows downwards, when an offset underflow is detected the stack is
        // overflowing.
        let next_offset = self
            .offset
            .checked_sub(1)
            .ok_or(ExceptionalHalt::StackOverflow)?;

        // The following index cannot fail because `next_offset` has already been checked and
        // `self.offset` is known to be within `STACK_LIMIT`.
        #[expect(unsafe_code, reason = "next_offset == self.offset - 1 >= 0")]
        unsafe {
            *self
                .values
                .get_unchecked_mut(next_offset)
                .0
                .as_mut_ptr()
                .cast() = [0u64; U64_PER_U256];
        }
        self.offset = next_offset;

        Ok(())
    }

    pub fn len(&self) -> usize {
        // The following operation cannot underflow because `self.offset` is known to be less than
        // or equal to `self.values.len()` (aka. `STACK_LIMIT`).
        #[expect(clippy::arithmetic_side_effects)]
        {
            self.values.len() - self.offset
        }
    }

    pub fn is_empty(&self) -> bool {
        self.offset == self.values.len()
    }

    #[inline(always)]
    pub fn swap<const N: usize>(&mut self) -> Result<(), ExceptionalHalt> {
        // Compile-time check that ensures `self.offset + N` is safe,
        // since self.offset is bounded by STACK_LIMIT
        const {
            assert!(STACK_LIMIT.checked_add(N).is_some());
        }
        #[expect(clippy::arithmetic_side_effects)]
        let index = self.offset + N;

        if index >= self.values.len() {
            return Err(ExceptionalHalt::StackUnderflow);
        }

        #[expect(unsafe_code, reason = "self.offset always < STACK_LIMIT")]
        unsafe {
            assert_unchecked(self.offset < STACK_LIMIT)
        };

        self.values.swap(self.offset, index);
        Ok(())
    }

    pub fn clear(&mut self) {
        self.offset = STACK_LIMIT;
    }

    /// Pushes a copy of the value at depth N
    #[inline]
    pub fn dup<const N: usize>(&mut self) -> Result<(), ExceptionalHalt> {
        // Compile-time check that ensures `self.offset + N` is safe,
        // since self.offset is bounded by STACK_LIMIT
        const {
            assert!(STACK_LIMIT.checked_add(N).is_some());
        }
        #[expect(clippy::arithmetic_side_effects)]
        let index = self.offset + N;
        if index >= self.values.len() {
            return Err(ExceptionalHalt::StackUnderflow);
        }

        self.offset = self
            .offset
            .checked_sub(1)
            .ok_or(ExceptionalHalt::StackOverflow)?;

        #[expect(unsafe_code, reason = "index < size, offset-1 >= 0")]
        unsafe {
            std::ptr::copy_nonoverlapping(
                self.values.get_unchecked_mut(index).0.as_mut_ptr(),
                self.values.get_unchecked_mut(self.offset).0.as_mut_ptr(),
                U64_PER_U256,
            );
        }
        Ok(())
    }
}

impl Default for Stack {
    fn default() -> Self {
        Self {
            values: Box::new([U256::zero(); STACK_LIMIT]),
            offset: STACK_LIMIT,
        }
    }
}

impl fmt::Debug for Stack {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        struct StackValues<'a>(&'a [U256]);

        impl fmt::Debug for StackValues<'_> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_list().entries(self.0.iter().rev()).finish()
            }
        }

        #[expect(clippy::indexing_slicing)]
        f.debug_tuple("Stack")
            .field(&StackValues(&self.values[self.offset..]))
            .finish()
    }
}

impl Hash for Stack {
    #[expect(
        clippy::indexing_slicing,
        reason = "offset is always within bounds of values"
    )]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.values[self.offset..].hash(state);
    }
}

#[derive(Debug)]
/// A call frame, or execution environment, is the context in which
/// the EVM is currently executing.
/// One context can trigger another with opcodes like CALL or CREATE.
/// Call frames relationships can be thought of as a parent-child relation.
pub struct CallFrame {
    /// Max gas a callframe can use
    pub gas_limit: u64,
    /// Keeps track of the remaining gas in the current context.
    ///
    /// This is a i64 for performance reasons, to allow faster gas cost substraction and checks.
    ///
    /// Additionally, gas limit won't be a problem since https://eips.ethereum.org/EIPS/eip-7825 limits it to 2^24, which is lower than i64::MAX.
    pub gas_remaining: i64,
    /// Program Counter
    pub pc: usize,
    /// Address of the account that sent the message
    pub msg_sender: Address,
    /// Address of the recipient of the message
    pub to: Address,
    /// Address of the code to execute. Usually the same as `to`, but can be different
    pub code_address: Address,
    /// Bytecode to execute.
    /// Its hash field will be bogus for initcodes, as it is inaccessible to the VM
    /// unless associated to an account, which doesn't happen for its initcode.
    pub bytecode: Code,
    /// Value sent along the transaction
    pub msg_value: U256,
    pub stack: Stack,
    pub memory: Memory,
    /// Data sent along the transaction. Empty in CREATE transactions.
    pub calldata: Bytes,
    /// Return data of the CURRENT CONTEXT (see docs for more details)
    pub output: Bytes,
    /// Return data of the SUB-CONTEXT (see docs for more details)
    pub sub_return_data: Bytes,
    /// Indicates if current context is static (if it is, it can't alter state)
    pub is_static: bool,
    /// Call stack current depth
    pub depth: usize,
    /// This is set to true if the function that created this callframe is CREATE or CREATE2
    pub is_create: bool,
    /// Everytime we want to write an account during execution of a callframe we store the pre-write state so that we can restore if it reverts
    pub call_frame_backup: CallFrameBackup,
    /// Return data offset
    pub ret_offset: usize,
    /// Return data size
    pub ret_size: usize,
    /// If true then transfer value from caller to callee
    pub should_transfer_value: bool,
    /// EIP-8037: snapshot of VM.state_gas_used (signed) at child-frame entry.
    /// Used to restore parent's state_gas_used on child revert.
    pub state_gas_used_at_entry: i64,
}

#[derive(Debug, Clone, Eq, PartialEq, Default)]
pub struct CallFrameBackup {
    pub original_accounts_info: FxHashMap<Address, LevmAccount>,
    pub original_account_storage_slots: FxHashMap<Address, FxHashMap<H256, U256>>,
    /// BAL checkpoint for EIP-7928 - used to restore state changes on revert
    /// while preserving touched_addresses.
    pub bal_checkpoint: Option<BlockAccessListCheckpoint>,
    /// Code hashes this frame inserted into the by-hash code cache
    /// (`GeneralizedDatabase::codes`) for codes it deployed. Removed from the
    /// cache on revert: a stale entry would make a later read of the same
    /// hash (from a pre-existing account) hit the cache instead of the store,
    /// hiding the read from execution-witness recording (EIP-8025).
    pub inserted_code_hashes: Vec<H256>,
}

impl CallFrameBackup {
    pub fn backup_account_info(
        &mut self,
        address: Address,
        account: &LevmAccount,
    ) -> Result<(), InternalError> {
        self.original_accounts_info
            .entry(address)
            .or_insert_with(|| LevmAccount {
                info: account.info.clone(),
                storage: Default::default(),
                status: account.status.clone(),
                has_storage: account.has_storage,
                exists: account.exists,
            });

        Ok(())
    }

    pub fn clear(&mut self) {
        self.original_accounts_info.clear();
        self.original_account_storage_slots.clear();
        self.bal_checkpoint = None;
        self.inserted_code_hashes.clear();
    }

    /// Merges `other` into `self`, per-address. For slots present in both,
    /// `other`'s values win. Callers MUST pass the older/more-original backup
    /// as `other` so the truly-original value is preserved (matches the
    /// `or_insert` semantic in `backup_storage_slot`).
    pub fn extend(&mut self, other: CallFrameBackup) {
        // Per-slot merge: plain HashMap::extend would let `other`'s inner slot map
        // replace `self`'s, dropping any slots `self` had for the same address.
        for (address, other_storage) in other.original_account_storage_slots {
            self.original_account_storage_slots
                .entry(address)
                .or_default()
                .extend(other_storage);
        }
        self.original_accounts_info
            .extend(other.original_accounts_info);
        self.inserted_code_hashes.extend(other.inserted_code_hashes);
        // Don't extend bal_checkpoint - it's specific to each call frame
    }
}

impl CallFrame {
    #[expect(
        clippy::too_many_arguments,
        reason = "inlined constructor, many args needed for performance"
    )]
    // Force inline, due to lot of arguments, inlining must be forced, and it is actually beneficial
    // because passing so much data is costly. Verified with samply.
    #[inline(always)]
    pub fn new(
        msg_sender: Address,
        to: Address,
        code_address: Address,
        bytecode: Code,
        msg_value: U256,
        calldata: Bytes,
        is_static: bool,
        gas_limit: u64,
        depth: usize,
        should_transfer_value: bool,
        is_create: bool,
        ret_offset: usize,
        ret_size: usize,
        stack: Stack,
        memory: Memory,
    ) -> Self {
        // Note: Do not use ..Default::default() because it has runtime cost.

        #[expect(clippy::as_conversions, reason = "remaining gas conversion")]
        Self {
            gas_limit,
            gas_remaining: gas_limit as i64,
            msg_sender,
            to,
            code_address,
            bytecode,
            msg_value,
            calldata,
            is_static,
            depth,
            should_transfer_value,
            is_create,
            ret_offset,
            ret_size,
            stack,
            memory,
            call_frame_backup: CallFrameBackup::default(),
            output: Bytes::default(),
            pc: 0,
            sub_return_data: Bytes::default(),
            state_gas_used_at_entry: 0,
        }
    }

    #[inline(always)]
    pub fn next_opcode(&self) -> u8 {
        // SAFETY: pc reaches at most bytecode_len + 32 (a PUSH32 at the last real
        // byte advances 33 total: +1 in the dispatch loop, +32 in the handler).
        // dispatch_buf() is bytecode_len + BYTECODE_PADDING (33) long, so the read
        // is always in bounds.
        #[expect(unsafe_code, reason = "pc bounded by padded bytecode len")]
        unsafe {
            *self.bytecode.dispatch_buf().get_unchecked(self.pc)
        }
    }

    pub fn pc(&self) -> usize {
        self.pc
    }

    /// Increases gas consumption of CallFrame and Environment, returning an error if the callframe gas limit is reached.
    #[inline(always)]
    #[expect(clippy::as_conversions, reason = "remaining gas conversion")]
    #[expect(clippy::arithmetic_side_effects, reason = "arithmethic checked")]
    pub fn increase_consumed_gas(&mut self, gas: u64) -> Result<(), ExceptionalHalt> {
        self.gas_remaining -= gas as i64;

        if self.gas_remaining < 0 {
            return Err(ExceptionalHalt::OutOfGas);
        }

        Ok(())
    }

    /// EELS' `check_gas`: assert gas is available without consuming it.
    #[inline(always)]
    #[expect(clippy::as_conversions, reason = "remaining gas conversion")]
    pub fn check_gas(&self, gas: u64) -> Result<(), ExceptionalHalt> {
        if self.gas_remaining < 0 || (self.gas_remaining as u64) < gas {
            return Err(ExceptionalHalt::OutOfGas);
        }
        Ok(())
    }

    pub fn set_code(&mut self, code: Code) -> Result<(), VMError> {
        self.bytecode = code;
        Ok(())
    }
}

impl<'a> VM<'a> {
    /// Adds current calframe to call_frames, sets current call frame to the passed callframe.
    #[inline(always)]
    pub fn add_callframe(&mut self, new_call_frame: CallFrame) {
        // Reserve once on the first sub-call (p99 depth ~10, max 27): keeps the ~43% of txs that
        // never make a call alloc-free (`call_frames` starts as `Vec::new()`), while avoiding the
        // repeated reallocs a call-heavy tx would otherwise incur as depth grows.
        if self.call_frames.is_empty() {
            self.call_frames.reserve(8);
        }
        self.call_frames.push(new_call_frame);
        #[allow(unsafe_code, reason = "just pushed, so the vec is not empty")]
        unsafe {
            std::mem::swap(
                &mut self.current_call_frame,
                self.call_frames.last_mut().unwrap_unchecked(),
            );
        }
    }

    #[inline(always)]
    pub fn pop_call_frame(&mut self) -> Result<CallFrame, InternalError> {
        let mut new = self.call_frames.pop().ok_or(InternalError::CallFrame)?;

        std::mem::swap(&mut new, &mut self.current_call_frame);

        Ok(new)
    }

    pub fn is_initial_call_frame(&self) -> bool {
        self.call_frames.is_empty()
    }

    /// Restores the cache state to the state before changes made during a callframe.
    pub fn restore_cache_state(&mut self) -> Result<(), VMError> {
        let callframe_backup = self.current_call_frame.call_frame_backup.clone();
        restore_cache_state(self.db, callframe_backup)
    }

    /// Like [`Self::restore_cache_state`] but moves the current frame's backup out instead of
    /// cloning it. Only sound when nothing reads `call_frame_backup` afterward: the inner-call
    /// revert in `handle_return` (the frame is popped right after, so its backup is dead), and
    /// the top-level / invalid-tx revert when no `BackupHook` is installed (normal L1 block
    /// execution, gated on `VM::preserve_top_level_backup`).
    ///
    /// When a `BackupHook` IS present (L2 / stateless) the top-level paths must keep cloning,
    /// because `BackupHook::finalize` reads the backup to build the tx-level undo snapshot.
    pub fn restore_cache_state_consuming(&mut self) -> Result<(), VMError> {
        let callframe_backup = std::mem::take(&mut self.current_call_frame.call_frame_backup);
        restore_cache_state(self.db, callframe_backup)
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
            .current_call_frame
            .call_frame_backup
            .original_accounts_info;
        for (address, account) in child_call_frame_backup.original_accounts_info.iter() {
            if parent_backup_accounts.get(address).is_none() {
                parent_backup_accounts.insert(*address, account.clone());
            }
        }

        let parent_backup_storage = &mut self
            .current_call_frame
            .call_frame_backup
            .original_account_storage_slots;
        for (address, storage) in child_call_frame_backup
            .original_account_storage_slots
            .iter()
        {
            let parent_storage = parent_backup_storage.entry(*address).or_default();
            for (key, value) in storage {
                if parent_storage.get(key).is_none() {
                    parent_storage.insert(*key, *value);
                }
            }
        }

        // Propagate code-cache insertions so a revert of the parent also
        // evicts codes deployed by the (committed) child frame.
        self.current_call_frame
            .call_frame_backup
            .inserted_code_hashes
            .extend(child_call_frame_backup.inserted_code_hashes.iter().copied());

        Ok(())
    }

    #[inline(always)]
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "pc bounded by padded bytecode len"
    )]
    pub fn advance_pc(&mut self) {
        self.current_call_frame.pc += 1;
    }
}
