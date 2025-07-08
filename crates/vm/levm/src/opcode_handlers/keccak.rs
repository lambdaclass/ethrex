use crate::{
    errors::{ExceptionalHalt, OpcodeResult, VMError},
    gas_cost,
    memory::calculate_memory_size,
    vm::VM,
};
use ethrex_common::utils::u256_from_big_endian;
use sha3::{Digest, Keccak256};

// KECCAK256 (1)
// Opcodes: KECCAK256

impl<'a> VM<'a> {
    pub fn op_keccak256(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        let [offset, size] = *current_call_frame.stack.pop()?;
        let offset: usize = offset
            .try_into()
            .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;

        let size: usize = size
            .try_into()
            .map_err(|_| ExceptionalHalt::VeryLargeNumber)?;

        let new_memory_size = calculate_memory_size(offset, size)?;

        current_call_frame.increase_consumed_gas(gas_cost::keccak256(
            new_memory_size,
            current_call_frame.memory.len(),
            size,
        )?)?;

        let mut hasher = Keccak256::new();
        hasher.update(current_call_frame.memory.load_range(offset, size)?);
        current_call_frame
            .stack
            .push(&[u256_from_big_endian(&hasher.finalize())])?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }
}
