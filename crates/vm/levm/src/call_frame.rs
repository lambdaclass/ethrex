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
use std::{
    collections::HashMap,
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
        #[expect(unsafe_code, reason = "next_offset == self.offset - 1 >= 0")]
        unsafe {
            std::ptr::copy_nonoverlapping(
                value.0.as_ptr(),
                self.values.get_unchecked_mut(next_offset).0.as_mut_ptr(),
                U64_PER_U256,
            );
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
}

#[derive(Debug, Clone, Eq, PartialEq, Default)]
pub struct CallFrameBackup {
    pub original_accounts_info: HashMap<Address, LevmAccount>,
    pub original_account_storage_slots: HashMap<Address, HashMap<H256, U256>>,
    /// BAL checkpoint for EIP-7928 - used to restore state changes on revert
    /// while preserving touched_addresses.
    pub bal_checkpoint: Option<BlockAccessListCheckpoint>,
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
            });

        Ok(())
    }

    pub fn clear(&mut self) {
        self.original_accounts_info.clear();
        self.original_account_storage_slots.clear();
        self.bal_checkpoint = None;
    }

    pub fn extend(&mut self, other: CallFrameBackup) {
        self.original_account_storage_slots
            .extend(other.original_account_storage_slots);
        self.original_accounts_info
            .extend(other.original_accounts_info);
        // Don't extend bal_checkpoint - it's specific to each call frame
    }
}

impl CallFrame {
    #[allow(clippy::too_many_arguments)]
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
        }
    }

    #[inline(always)]
    pub fn next_opcode(&self) -> u8 {
        if self.pc < self.bytecode.bytecode.len() {
            #[expect(unsafe_code, reason = "bounds checked above")]
            unsafe {
                *self.bytecode.bytecode.get_unchecked(self.pc)
            }
        } else {
            0
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

    pub fn set_code(&mut self, code: Code) -> Result<(), VMError> {
        self.bytecode = code;
        Ok(())
    }
}

impl<'a> VM<'a> {
    /// Adds current calframe to call_frames, sets current call frame to the passed callframe.
    #[inline(always)]
    pub fn add_callframe(&mut self, new_call_frame: CallFrame) {
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

        Ok(())
    }

    #[inline(always)]
    pub fn advance_pc(&mut self, count: usize) -> Result<(), VMError> {
        self.current_call_frame.pc = self
            .current_call_frame
            .pc
            .checked_add(count)
            .ok_or(InternalError::Overflow)?;
        Ok(())
    }
}
