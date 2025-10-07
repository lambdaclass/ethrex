//! # Keccak256 operations
//!
//! Includes the following opcodes:
//!   - `KECCAK256`

use crate::{
    errors::{OpcodeResult, VMError},
    gas_cost,
    memory::calculate_memory_size,
    opcode_handlers::OpcodeHandler,
    utils::size_offset_to_usize,
    vm::VM,
};
use ethrex_common::utils::u256_from_big_endian;
use sha3::{Digest, Keccak256};

pub struct OpKeccak256Handler;
impl OpcodeHandler for OpKeccak256Handler {
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [offset, len] = *vm.current_call_frame.stack.pop()?;
        let (len, offset) = size_offset_to_usize(len, offset)?;

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::keccak256(
                calculate_memory_size(offset, len)?,
                vm.current_call_frame.memory.len(),
                len,
            )?)?;

        let mut hasher = Keccak256::new();
        hasher.update(vm.current_call_frame.memory.load_range(offset, len)?);
        vm.current_call_frame
            .stack
            .push1(u256_from_big_endian(&hasher.finalize()))?;

        Ok(OpcodeResult::Continue)
    }
}
