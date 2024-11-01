use crate::{constants::STACK_LIMIT, errors::VMError, memory::Memory, opcodes::Opcode};
use bytes::Bytes;
use ethereum_rust_core::{types::Log, Address, U256};
use std::collections::HashMap;

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

    pub fn swap(&mut self, a: usize, b: usize) {
        self.stack.swap(a, b)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
/// A call frame, or execution environment, is the context in which
/// the EVM is currently executing.
pub struct CallFrame {
    pub gas_limit: U256,
    pub gas_used: U256,
    pub pc: usize,
    /// Address of the account that sent the message
    pub msg_sender: Address,
    /// Address of the recipient of the message
    pub to: Address,
    /// Address of the code to execute. Usually the same as `to`, but can be different
    pub code_address: Address,
    /// Bytecode to execute
    pub bytecode: Bytes,
    pub msg_value: U256,
    pub stack: Stack, // max 1024 in the future
    pub memory: Memory,
    pub calldata: Bytes,
    /// Return data of the CURRENT CONTEXT (see docs for more details)
    pub returndata: Bytes,
    /// Return data of the SUB-CONTEXT (see docs for more details)
    pub sub_return_data: Bytes,
    /// where to store return data of sub-context in memory
    pub sub_return_data_offset: usize,
    pub sub_return_data_size: usize,
    pub is_static: bool,
    pub transient_storage: TransientStorage,
    pub logs: Vec<Log>,
    pub depth: usize,
}

impl CallFrame {
    pub fn new_from_bytecode(bytecode: Bytes) -> Self {
        Self {
            bytecode,
            gas_limit: U256::MAX,
            ..Default::default()
        }
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
        gas_limit: U256,
        gas_used: U256,
        depth: usize,
    ) -> Self {
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
            ..Default::default()
        }
    }

    pub fn next_opcode(&mut self) -> Option<Opcode> {
        let opcode = self.opcode_at(self.pc);
        self.increment_pc();
        opcode
    }

    pub fn increment_pc_by(&mut self, count: usize) {
        self.pc += count;
    }

    pub fn increment_pc(&mut self) {
        self.increment_pc_by(1);
    }

    pub fn pc(&self) -> usize {
        self.pc
    }

    /// Jump to the given address, returns false if the jump position wasn't a JUMPDEST
    pub fn jump(&mut self, jump_address: U256) -> bool {
        if !self.valid_jump(jump_address) {
            return false;
        }
        self.pc = jump_address.as_usize();
        true
    }

    fn valid_jump(&self, jump_address: U256) -> bool {
        self.opcode_at(jump_address.as_usize())
            .map(|opcode| opcode.eq(&Opcode::JUMPDEST))
            .is_some_and(|is_jumpdest| is_jumpdest)
    }

    fn opcode_at(&self, offset: usize) -> Option<Opcode> {
        self.bytecode.get(offset).copied().map(Opcode::from)
    }
}
