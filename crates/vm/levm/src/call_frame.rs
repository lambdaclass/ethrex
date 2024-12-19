use crate::{
    constants::STACK_LIMIT,
    errors::{InternalError, VMError},
    memory::Memory,
    opcodes::Opcode,
    vm::get_valid_jump_destinations,
};
use bytes::Bytes;
use ethrex_core::{types::Log, Address, U256};
use std::collections::{HashMap, HashSet};

/// [EIP-1153]: https://eips.ethereum.org/EIPS/eip-1153#reference-implementation
pub type TransientStorage = HashMap<(Address, U256), U256>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stack {
    pub stack: Vec<U256>,
}

impl Stack {
    pub fn pop(&mut self) -> Result<U256, VMError> {
        self.stack.pop().ok_or(VMError::StackUnderflow)
    }

    pub fn push(&mut self, value: U256) -> Result<(), VMError> {
        if self.stack.len() >= STACK_LIMIT {
            return Err(VMError::StackOverflow);
        }
        self.stack.push(value);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.stack.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    pub fn get(&self, index: usize) -> Result<&U256, VMError> {
        self.stack.get(index).ok_or(VMError::StackUnderflow)
    }

    pub fn swap(&mut self, a: usize, b: usize) -> Result<(), VMError> {
        if a >= self.stack.len() || b >= self.stack.len() {
            return Err(VMError::StackUnderflow);
        }
        self.stack.swap(a, b);
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
/// A call frame, or execution environment, is the context in which
/// the EVM is currently executing.
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
    /// Indicates if current context is static (if it is, it can't change state)
    pub is_static: bool,
    pub transient_storage: TransientStorage,
    pub logs: Vec<Log>,
    /// Call stack current depth
    pub depth: usize,
    /// Set of valid jump destinations (where a JUMP or JUMPI can jump to)
    pub valid_jump_destinations: HashSet<usize>,
    /// This is set to true if the function that created this callframe is CREATE or CREATE2
    pub create_op_called: bool,
}

impl CallFrame {
    pub fn new_from_bytecode(bytecode: Bytes) -> Self {
        let valid_jump_destinations = get_valid_jump_destinations(&bytecode).unwrap_or_default();
        Self {
            gas_limit: u64::MAX,
            bytecode,
            valid_jump_destinations,
            ..Default::default()
        }
    }

    pub fn assign_bytecode(&mut self, bytecode: Bytes) {
        self.bytecode = bytecode;
        self.valid_jump_destinations =
            get_valid_jump_destinations(&self.bytecode).unwrap_or_default();
    }

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
        gas_used: u64,
        depth: usize,
        create_op_called: bool,
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
            gas_used,
            valid_jump_destinations,
            create_op_called,
            ..Default::default()
        }
    }

    pub fn next_opcode(&mut self) -> Opcode {
        match self.bytecode.get(self.pc).copied().map(Opcode::from) {
            Some(opcode) => opcode,
            None => Opcode::STOP,
        }
    }

    pub fn increment_pc_by(&mut self, count: usize) -> Result<(), VMError> {
        self.pc = self
            .pc
            .checked_add(count)
            .ok_or(VMError::Internal(InternalError::PCOverflowed))?;
        Ok(())
    }

    pub fn increment_pc(&mut self) -> Result<(), VMError> {
        self.increment_pc_by(1)
    }

    pub fn pc(&self) -> usize {
        self.pc
    }
}
