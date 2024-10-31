use keccak_hash::H256;

use crate::{constants::WORD_SIZE, vm::StorageSlot};

use super::*;

// Stack, Memory, Storage and Flow Operations (15)
// Opcodes: POP, MLOAD, MSTORE, MSTORE8, SLOAD, SSTORE, JUMP, JUMPI, PC, MSIZE, GAS, JUMPDEST, TLOAD, TSTORE, MCOPY

impl VM {
    // POP operation
    pub fn op_pop(&mut self, current_call_frame: &mut CallFrame) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::POP)?;
        current_call_frame.stack.pop()?;
        Ok(OpcodeSuccess::Continue)
    }

    // TLOAD operation
    pub fn op_tload(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::TLOAD)?;

        let key = current_call_frame.stack.pop()?;
        let value = current_call_frame
            .transient_storage
            .get(&(current_call_frame.msg_sender, key))
            .cloned()
            .unwrap_or(U256::zero());

        current_call_frame.stack.push(value)?;
        Ok(OpcodeSuccess::Continue)
    }

    // TSTORE operation
    pub fn op_tstore(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::TSTORE)?;

        let key = current_call_frame.stack.pop()?;
        let value = current_call_frame.stack.pop()?;
        current_call_frame
            .transient_storage
            .insert((current_call_frame.msg_sender, key), value);

        Ok(OpcodeSuccess::Continue)
    }

    // MLOAD operation
    pub fn op_mload(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let offset = current_call_frame
            .stack
            .pop()?
            .try_into()
            .unwrap_or(usize::MAX);
        let memory_expansion_cost = current_call_frame
            .memory
            .expansion_cost(offset + WORD_SIZE)?;
        let gas_cost = gas_cost::MLOAD_STATIC + memory_expansion_cost;

        self.increase_consumed_gas(current_call_frame, gas_cost)?;

        let value = current_call_frame.memory.load(offset);
        current_call_frame.stack.push(value)?;

        Ok(OpcodeSuccess::Continue)
    }

    // MSTORE operation
    pub fn op_mstore(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let offset = current_call_frame.stack.pop()?.try_into().unwrap();
        let memory_expansion_cost = current_call_frame
            .memory
            .expansion_cost(offset + WORD_SIZE)?;
        let gas_cost = gas_cost::MSTORE_STATIC + memory_expansion_cost;

        self.increase_consumed_gas(current_call_frame, gas_cost)?;

        let value = current_call_frame.stack.pop()?;
        let mut value_bytes = [0u8; WORD_SIZE];
        value.to_big_endian(&mut value_bytes);

        current_call_frame.memory.store_bytes(offset, &value_bytes);

        Ok(OpcodeSuccess::Continue)
    }

    // MSTORE8 operation
    pub fn op_mstore8(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let offset = current_call_frame.stack.pop()?.try_into().unwrap();
        let memory_expansion_cost = current_call_frame.memory.expansion_cost(offset + 1)?;
        let gas_cost = gas_cost::MSTORE8_STATIC + memory_expansion_cost;

        self.increase_consumed_gas(current_call_frame, gas_cost)?;

        let value = current_call_frame.stack.pop()?;
        let mut value_bytes = [0u8; WORD_SIZE];
        value.to_big_endian(&mut value_bytes);

        current_call_frame
            .memory
            .store_bytes(offset, value_bytes[WORD_SIZE - 1..WORD_SIZE].as_ref());

        Ok(OpcodeSuccess::Continue)
    }

    // SLOAD operation
    // TODO: add gas consumption
    pub fn op_sload(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let key = current_call_frame.stack.pop()?;

        let address = current_call_frame.to;

        let mut bytes = [0u8; 32];
        key.to_big_endian(&mut bytes);
        let key = H256::from(bytes);

        let current_value = if self.cache.is_slot_cached(&address, key) {
            self.cache
                .get_storage_slot(address, key)
                .unwrap_or_default()
                .current_value
        } else {
            self.db.get_storage_slot(address, key)
        };

        current_call_frame.stack.push(current_value)?;
        Ok(OpcodeSuccess::Continue)
    }

    // SSTORE operation
    // TODO: add gas consumption
    pub fn op_sstore(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        if current_call_frame.is_static {
            return Err(VMError::OpcodeNotAllowedInStaticContext);
        }

        let key = current_call_frame.stack.pop()?;
        let value = current_call_frame.stack.pop()?;

        let mut bytes = [0u8; 32];
        key.to_big_endian(&mut bytes);
        let key = H256::from(bytes);

        let address = current_call_frame.to;

        let original_value = if self.cache.is_slot_cached(&address, key) {
            self.cache
                .get_storage_slot(address, key)
                .expect("Storage slot should have been cached")
                .original_value
        } else {
            self.cache_from_db(&address);
            self.db.get_storage_slot(address, key)
        };

        self.cache.write_account_storage(
            &address,
            key,
            StorageSlot {
                original_value,
                current_value: value,
            },
        );

        Ok(OpcodeSuccess::Continue)
    }

    // MSIZE operation
    pub fn op_msize(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::MSIZE)?;
        current_call_frame
            .stack
            .push(current_call_frame.memory.size())?;
        Ok(OpcodeSuccess::Continue)
    }

    // GAS operation
    pub fn op_gas(&mut self, current_call_frame: &mut CallFrame) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::GAS)?;

        let remaining_gas = self.env.gas_limit - self.env.consumed_gas - gas_cost::GAS;
        current_call_frame.stack.push(remaining_gas)?;

        Ok(OpcodeSuccess::Continue)
    }

    // MCOPY operation
    pub fn op_mcopy(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let dest_offset = current_call_frame
            .stack
            .pop()?
            .try_into()
            .unwrap_or(usize::MAX);
        let src_offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .unwrap_or(usize::MAX);
        let size: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .unwrap_or(usize::MAX);

        let words_copied = (size + WORD_SIZE - 1) / WORD_SIZE;

        let memory_byte_size = src_offset
            .checked_add(size)
            .and_then(|src_sum| {
                dest_offset
                    .checked_add(size)
                    .map(|dest_sum| src_sum.max(dest_sum))
            })
            .ok_or(VMError::OverflowInArithmeticOp)?;

        let memory_expansion_cost = current_call_frame.memory.expansion_cost(memory_byte_size)?;
        let gas_cost = gas_cost::MCOPY_STATIC
            + gas_cost::MCOPY_DYNAMIC_BASE * words_copied
            + memory_expansion_cost;

        self.increase_consumed_gas(current_call_frame, gas_cost)?;

        if size > 0 {
            current_call_frame
                .memory
                .copy(src_offset, dest_offset, size);
        }

        Ok(OpcodeSuccess::Continue)
    }

    // JUMP operation
    pub fn op_jump(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::JUMP)?;

        let jump_address = current_call_frame.stack.pop()?;
        if !current_call_frame.jump(jump_address) {
            return Err(VMError::InvalidJump);
        }

        Ok(OpcodeSuccess::Continue)
    }

    // JUMPI operation
    pub fn op_jumpi(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let jump_address = current_call_frame.stack.pop()?;
        let condition = current_call_frame.stack.pop()?;

        if condition != U256::zero() && !current_call_frame.jump(jump_address) {
            return Err(VMError::InvalidJump);
        }

        self.increase_consumed_gas(current_call_frame, gas_cost::JUMPI)?;
        Ok(OpcodeSuccess::Continue)
    }

    // JUMPDEST operation
    pub fn op_jumpdest(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::JUMPDEST)?;
        Ok(OpcodeSuccess::Continue)
    }

    // PC operation
    pub fn op_pc(&mut self, current_call_frame: &mut CallFrame) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::PC)?;

        current_call_frame
            .stack
            .push(U256::from(current_call_frame.pc - 1))?;

        Ok(OpcodeSuccess::Continue)
    }
}
