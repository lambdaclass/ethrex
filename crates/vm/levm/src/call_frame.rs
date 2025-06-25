use crate::{
    constants::STACK_LIMIT,
    errors::{ExceptionalHalt, InternalError, VMError},
    memory::Memory,
    opcodes::Opcode,
    utils::{get_valid_jump_destinations, restore_cache_state},
    vm::VM,
};
use bytes::Bytes;
use ethrex_common::{Address, U256, types::Account};
use keccak_hash::H256;
use std::{
    collections::{HashMap, HashSet},
    fmt,
};

#[derive(Clone, PartialEq, Eq)]
/// The EVM uses a stack-based architecture and does not use registers like some other VMs.
pub struct Stack {
    pub values: Box<[U256; STACK_LIMIT]>,
    pub offset: usize,
}

impl Stack {
    pub fn pop<const N: usize>(&mut self) -> Result<&[U256; N], ExceptionalHalt> {
        // Compile-time check for stack underflow.
        if N > STACK_LIMIT {
            return Err(ExceptionalHalt::StackUnderflow);
        }

        // The following operation can never overflow as both `self.offset` and N are within
        // STACK_LIMIT (1024).
        #[expect(clippy::arithmetic_side_effects)]
        let next_offset = self.offset + N;

        // The index cannot fail because `self.offset` is known to be valid. The `first_chunk()`
        // method will ensure that `next_offset` is within `STACK_LIMIT`, so there's no need to
        // check it again.
        #[expect(clippy::indexing_slicing)]
        let values = self.values[self.offset..]
            .first_chunk::<N>()
            .ok_or(ExceptionalHalt::StackUnderflow)?;
        self.offset = next_offset;

        Ok(values)
    }

    pub fn push<const N: usize>(&mut self, values: &[U256; N]) -> Result<(), ExceptionalHalt> {
        // Since the stack grows downwards, when an offset underflow is detected the stack is
        // overflowing.
        let next_offset = self
            .offset
            .checked_sub(N)
            .ok_or(ExceptionalHalt::StackOverflow)?;

        // The following index cannot fail because `next_offset` has already been checked and
        // `self.offset` is known to be within `STACK_LIMIT`.
        #[expect(clippy::indexing_slicing)]
        self.values[next_offset..self.offset].copy_from_slice(values);
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

    pub fn get(&self, index: usize) -> Result<&U256, ExceptionalHalt> {
        // The following index cannot fail because `self.offset` is known to be within
        // `STACK_LIMIT`.
        #[expect(clippy::indexing_slicing)]
        self.values[self.offset..]
            .get(index)
            .ok_or(ExceptionalHalt::StackUnderflow)
    }

    pub fn swap(&mut self, index: usize) -> Result<(), ExceptionalHalt> {
        let index = self
            .offset
            .checked_add(index)
            .ok_or(ExceptionalHalt::StackUnderflow)?;
        if index >= self.values.len() {
            return Err(ExceptionalHalt::StackUnderflow);
        }

        self.values.swap(self.offset, index);
        Ok(())
    }
}

impl Default for Stack {
    fn default() -> Self {
        Self {
            values: Box::new([U256::zero(); 1024]),
            offset: 1024,
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

#[derive(Debug, Clone, Default, PartialEq)]
/// A call frame, or execution environment, is the context in which
/// the EVM is currently executing.
/// One context can trigger another with opcodes like CALL or CREATE.
/// Call frames relationships can be thought of as a parent-child relation.
pub struct CallFrame {
    /// Max gas a callframe can use
    pub gas_limit: u64,
    /// Keeps track of the gas that's been used in current context
    pub gas_used: u64,
    /// Program Counter
    pub pc: usize,
    /// Address of the account that sent the message
    pub msg_sender: Address,
    /// Address of the recipient of the message
    pub to: Address,
    /// Address of the code to execute. Usually the same as `to`, but can be different
    pub code_address: Address,
    /// Bytecode to execute
    pub bytecode: Bytes,
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
    /// Set of valid jump destinations (where a JUMP or JUMPI can jump to)
    pub valid_jump_destinations: HashSet<usize>,
    /// This is set to true if the function that created this callframe is CREATE or CREATE2
    pub is_create: bool,
    /// Everytime we want to write an account during execution of a callframe we store the pre-write state so that we can restore if it reverts
    pub call_frame_backup: CallFrameBackup,
    /// Return data offset
    pub ret_offset: U256,
    /// Return data size
    pub ret_size: usize,
    /// If true then transfer value from caller to callee
    pub should_transfer_value: bool,
}

#[derive(Debug, Clone, Eq, PartialEq, Default)]
pub struct CallFrameBackup {
    pub original_accounts_info: HashMap<Address, Account>,
    pub original_account_storage_slots: HashMap<Address, HashMap<H256, U256>>,
}

impl CallFrameBackup {
    pub fn clear(&mut self) {
        self.original_accounts_info.clear();
        self.original_account_storage_slots.clear();
    }

    pub fn extend(&mut self, other: CallFrameBackup) {
        self.original_account_storage_slots
            .extend(other.original_account_storage_slots);
        self.original_accounts_info
            .extend(other.original_accounts_info);
    }
}

impl CallFrame {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        msg_sender: Address,
        to: Address,
        code_address: Address,
        bytecode: Bytes,
        msg_value: U256,
        calldata: Bytes,
        is_static: bool,
        gas_limit: u64,
        depth: usize,
        should_transfer_value: bool,
        is_create: bool,
        ret_offset: U256,
        ret_size: usize,
    ) -> Self {
        let valid_jump_destinations = get_valid_jump_destinations(&bytecode).unwrap_or_default();
        Self {
            gas_limit,
            msg_sender,
            to,
            code_address,
            bytecode,
            msg_value,
            calldata,
            is_static,
            depth,
            valid_jump_destinations,
            should_transfer_value,
            is_create,
            ret_offset,
            ret_size,
            ..Default::default()
        }
    }

    pub fn next_opcode(&self) -> Opcode {
        self.bytecode
            .get(self.pc)
            .copied()
            .map(Opcode::from)
            .unwrap_or(Opcode::STOP)
    }

    pub fn increment_pc_by(&mut self, count: usize) -> Result<(), VMError> {
        self.pc = self.pc.checked_add(count).ok_or(InternalError::Overflow)?;
        Ok(())
    }

    pub fn pc(&self) -> usize {
        self.pc
    }

    /// Increases gas consumption of CallFrame and Environment, returning an error if the callframe gas limit is reached.
    pub fn increase_consumed_gas(&mut self, gas: u64) -> Result<(), ExceptionalHalt> {
        self.gas_used = self
            .gas_used
            .checked_add(gas)
            .ok_or(ExceptionalHalt::OutOfGas)?;

        if self.gas_used > self.gas_limit {
            return Err(ExceptionalHalt::OutOfGas);
        }

        Ok(())
    }

    pub fn set_code(&mut self, code: Bytes) -> Result<(), VMError> {
        self.valid_jump_destinations = get_valid_jump_destinations(&code)?;
        self.bytecode = code;
        Ok(())
    }
}

impl<'a> VM<'a> {
    pub fn current_call_frame_mut(&mut self) -> Result<&mut CallFrame, InternalError> {
        self.call_frames.last_mut().ok_or(InternalError::CallFrame)
    }

    pub fn current_call_frame(&self) -> Result<&CallFrame, InternalError> {
        self.call_frames.last().ok_or(InternalError::CallFrame)
    }

    pub fn pop_call_frame(&mut self) -> Result<CallFrame, InternalError> {
        self.call_frames.pop().ok_or(InternalError::CallFrame)
    }

    pub fn is_initial_call_frame(&self) -> bool {
        self.call_frames.len() == 1
    }

    /// Restores the cache state to the state before changes made during a callframe.
    pub fn restore_cache_state(&mut self) -> Result<(), VMError> {
        let callframe_backup = self.current_call_frame()?.call_frame_backup.clone();
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

    pub fn increment_pc_by(&mut self, count: usize) -> Result<(), VMError> {
        self.current_call_frame_mut()?.increment_pc_by(count)
    }
}
