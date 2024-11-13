use crate::{
    call_frame::CallFrame,
    constants::{gas_cost, WORD_SIZE},
    errors::{OpcodeSuccess, VMError},
    vm::VM,
};
use ethereum_rust_core::U256;
use sha3::{Digest, Keccak256};

// KECCAK256 (1)
// Opcodes: KECCAK256

impl VM {
    pub fn op_keccak256(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;
        let size = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;

        let minimum_word_size = (size + WORD_SIZE - 1) / WORD_SIZE;
        let memory_expansion_cost = current_call_frame.memory.expansion_cost(
            offset
                .checked_add(size)
                .ok_or(VMError::MemoryLoadOutOfBounds)?,
        )?;
        let gas_cost = gas_cost::KECCAK25_STATIC
            + gas_cost::KECCAK25_DYNAMIC_BASE * minimum_word_size
            + memory_expansion_cost;

        self.increase_consumed_gas(current_call_frame, gas_cost)?;

        let value_bytes = current_call_frame.memory.load_range(offset, size)?;

        let mut hasher = Keccak256::new();
        hasher.update(value_bytes);
        let result = hasher.finalize();
        current_call_frame
            .stack
            .push(U256::from_big_endian(&result))?;

        Ok(OpcodeSuccess::Continue)
    }
}
