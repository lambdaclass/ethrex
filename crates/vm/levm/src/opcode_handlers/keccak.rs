use crate::{
    errors::{OpcodeResult, VMError},
    gas_cost,
    memory::calculate_memory_size,
    utils::size_offset_to_usize,
    vm::VM,
};
use ethrex_common::utils::u256_from_big_endian;
use sha3::{Digest, Keccak256};

// KECCAK256 (1)
// Opcodes: KECCAK256

impl<'a> VM<'a> {
    pub fn op_keccak256(&mut self) -> Result<OpcodeResult, VMError> {
        let [offset, size] = *self.current_stack().pop()?;
        let (size, offset) = size_offset_to_usize(size, offset)?;

        let new_memory_size = calculate_memory_size(offset, size)?;

        self.current_call_frame
            .increase_consumed_gas(gas_cost::keccak256(
                new_memory_size,
                self.current_call_frame.memory.len(),
                size,
            )?)?;

        let mut hasher = Keccak256::new();
        hasher.update(self.current_call_frame.memory.load_range(offset, size)?);
        self.current_stack()
            .push1(u256_from_big_endian(&hasher.finalize()))?;

        Ok(OpcodeResult::Continue)
    }
}
