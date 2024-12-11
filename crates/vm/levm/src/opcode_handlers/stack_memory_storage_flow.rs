use crate::{
    call_frame::CallFrame,
    constants::{WORD_SIZE, WORD_SIZE_IN_BYTES_USIZE},
    errors::{OpcodeSuccess, OutOfGasError, VMError},
    gas_cost,
    memory::{self, calculate_memory_size},
    vm::VM,
};
use ethrex_core::{H256, U256};

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
        let offset = current_call_frame.stack.pop()?;

        let new_memory_size = calculate_memory_size(offset, WORD_SIZE_IN_BYTES_USIZE)?;

        self.increase_consumed_gas(
            current_call_frame,
            gas_cost::mload(new_memory_size, current_call_frame.memory.len())?,
        )?;

        current_call_frame
            .stack
            .push(memory::load_word(&mut current_call_frame.memory, offset)?)?;

        Ok(OpcodeSuccess::Continue)
    }

    // MSTORE operation
    pub fn op_mstore(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let offset = current_call_frame.stack.pop()?;

        let new_memory_size = calculate_memory_size(offset, WORD_SIZE_IN_BYTES_USIZE)?;

        self.increase_consumed_gas(
            current_call_frame,
            gas_cost::mstore(new_memory_size, current_call_frame.memory.len())?,
        )?;

        let value = current_call_frame.stack.pop()?;
        let mut value_bytes = [0u8; WORD_SIZE];
        value.to_big_endian(&mut value_bytes);

        memory::try_store_data(&mut current_call_frame.memory, offset, &value_bytes)?;

        Ok(OpcodeSuccess::Continue)
    }

    // MSTORE8 operation
    pub fn op_mstore8(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        // TODO: modify expansion cost to accept U256
        let offset = current_call_frame.stack.pop()?;

        let new_memory_size = calculate_memory_size(offset, 1)?;

        self.increase_consumed_gas(
            current_call_frame,
            gas_cost::mstore8(new_memory_size, current_call_frame.memory.len())?,
        )?;

        let value = current_call_frame.stack.pop()?;
        let mut value_bytes = [0u8; WORD_SIZE];
        value.to_big_endian(&mut value_bytes);

        memory::try_store_data(
            &mut current_call_frame.memory,
            offset,
            &value_bytes[WORD_SIZE - 1..WORD_SIZE],
        )?;

        Ok(OpcodeSuccess::Continue)
    }

    // SLOAD operation
    pub fn op_sload(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let storage_slot_key = current_call_frame.stack.pop()?;
        let address = current_call_frame.to;

        let mut bytes = [0u8; 32];
        storage_slot_key.to_big_endian(&mut bytes);
        let storage_slot_key = H256::from(bytes);

        let (storage_slot, storage_slot_was_cold) =
            self.access_storage_slot(address, storage_slot_key)?;

        self.increase_consumed_gas(current_call_frame, gas_cost::sload(storage_slot_was_cold)?)?;

        current_call_frame.stack.push(storage_slot.current_value)?;
        Ok(OpcodeSuccess::Continue)
    }

    // SSTORE operation
    // TODO: https://github.com/lambdaclass/ethrex/issues/1087
    pub fn op_sstore(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        if current_call_frame.is_static {
            return Err(VMError::OpcodeNotAllowedInStaticContext);
        }

        let storage_slot_key = current_call_frame.stack.pop()?;
        let new_storage_slot_value = current_call_frame.stack.pop()?;

        // Convert key from U256 to H256
        let mut bytes = [0u8; 32];
        storage_slot_key.to_big_endian(&mut bytes);
        let key = H256::from(bytes);

        let (storage_slot, storage_slot_was_cold) =
            self.access_storage_slot(current_call_frame.to, key)?;

        self.increase_consumed_gas(
            current_call_frame,
            gas_cost::sstore(
                &storage_slot,
                new_storage_slot_value,
                storage_slot_was_cold,
                current_call_frame,
            )?,
        )?;

        // Gas Refunds
        // TODO: Think about what to do in case of underflow of gas refunds (when we try to substract from it if the value is low)
        let mut gas_refunds = U256::zero();
        if new_storage_slot_value != storage_slot.current_value {
            if storage_slot.current_value == storage_slot.original_value {
                if !storage_slot.original_value.is_zero() && new_storage_slot_value.is_zero() {
                    gas_refunds = gas_refunds
                        .checked_add(U256::from(4800))
                        .ok_or(VMError::GasRefundsOverflow)?;
                }
            } else if !storage_slot.original_value.is_zero() {
                if storage_slot.current_value.is_zero() {
                    gas_refunds = gas_refunds
                        .checked_sub(U256::from(4800))
                        .ok_or(VMError::GasRefundsUnderflow)?;
                } else if new_storage_slot_value.is_zero() {
                    gas_refunds = gas_refunds
                        .checked_add(U256::from(4800))
                        .ok_or(VMError::GasRefundsOverflow)?;
                }
            } else if new_storage_slot_value == storage_slot.original_value {
                if storage_slot.original_value.is_zero() {
                    gas_refunds = gas_refunds
                        .checked_add(U256::from(19900))
                        .ok_or(VMError::GasRefundsOverflow)?;
                } else {
                    gas_refunds = gas_refunds
                        .checked_add(U256::from(2800))
                        .ok_or(VMError::GasRefundsOverflow)?;
                }
            }
        };

        self.env.refunded_gas = self
            .env
            .refunded_gas
            .checked_add(gas_refunds)
            .ok_or(VMError::GasLimitPriceProductOverflow)?;

        self.update_account_storage(current_call_frame.to, key, new_storage_slot_value)?;

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
            .push(current_call_frame.memory.len().into())?;
        Ok(OpcodeSuccess::Continue)
    }

    // GAS operation
    pub fn op_gas(&mut self, current_call_frame: &mut CallFrame) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::GAS)?;

        let remaining_gas = current_call_frame
            .gas_limit
            .checked_sub(current_call_frame.gas_used)
            .ok_or(OutOfGasError::ConsumedGasOverflow)?;
        // Note: These are not consumed gas calculations, but are related, so I used this wrapping here
        current_call_frame.stack.push(remaining_gas)?;

        Ok(OpcodeSuccess::Continue)
    }

    // MCOPY operation
    pub fn op_mcopy(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let dest_offset = current_call_frame.stack.pop()?;
        let src_offset = current_call_frame.stack.pop()?;
        let size: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;

        let new_memory_size_for_dest = calculate_memory_size(dest_offset, size)?;

        let new_memory_size_for_src = calculate_memory_size(src_offset, size)?;

        self.increase_consumed_gas(
            current_call_frame,
            gas_cost::mcopy(
                new_memory_size_for_dest.max(new_memory_size_for_src),
                current_call_frame.memory.len(),
                size,
            )?,
        )?;

        memory::try_copy_within(
            &mut current_call_frame.memory,
            src_offset,
            dest_offset,
            size,
        )?;

        Ok(OpcodeSuccess::Continue)
    }

    // JUMP operation
    pub fn op_jump(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::JUMP)?;

        let jump_address = current_call_frame.stack.pop()?;
        Self::jump(current_call_frame, jump_address)?;

        Ok(OpcodeSuccess::Continue)
    }

    /// JUMP* family (`JUMP` and `JUMP` ATTOW [DEC 2024]) helper
    /// function.
    /// This function returns whether the `jump_address` is a valid JUMPDEST
    /// for the specified `call_frame` or not.
    fn is_valid_jump_addr(call_frame: &CallFrame, jump_address: usize) -> bool {
        call_frame.valid_jump_destinations.contains(&jump_address)
    }

    /// JUMP* family (`JUMP` and `JUMP` ATTOW [DEC 2024]) helper
    /// function.
    /// This function will change the PC for the specified call frame
    /// to be equal to the specified address. If the address is not a
    /// valid JUMPDEST, it will return an error
    pub fn jump(call_frame: &mut CallFrame, jump_address: U256) -> Result<(), VMError> {
        let jump_address_usize = jump_address
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;

        match Self::is_valid_jump_addr(call_frame, jump_address_usize) {
            true => {
                call_frame.pc = jump_address_usize;
                Ok(())
            }
            false => Err(VMError::InvalidJump),
        }
    }

    // JUMPI operation
    pub fn op_jumpi(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let jump_address = current_call_frame.stack.pop()?;
        let condition = current_call_frame.stack.pop()?;

        self.increase_consumed_gas(current_call_frame, gas_cost::JUMPI)?;

        if !condition.is_zero() {
            Self::jump(current_call_frame, jump_address)?
        } else {
            current_call_frame.increment_pc()?;
        }
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
            .push(U256::from(current_call_frame.pc))?;

        Ok(OpcodeSuccess::Continue)
    }
}
