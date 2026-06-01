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

pub struct OpKeccak256Handler;
impl OpcodeHandler for OpKeccak256Handler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [offset, len] = *vm.current_call_frame.stack.pop()?;
        let (len, offset) = size_offset_to_usize(len, offset)?;

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::keccak256(
                calculate_memory_size(offset, len)?,
                vm.current_call_frame.memory.len(),
                len,
            )?)?;

        // Hash the memory range in place — `with_range` lends a borrow to keccak256
        // instead of allocating a throwaway `Bytes` copy (KECCAK256 fires ~15x/tx).
        // Bind `crypto` first so the closure doesn't capture `vm` while `memory` is
        // borrowed mutably.
        let crypto = vm.crypto;
        let hash = vm
            .current_call_frame
            .memory
            .with_range(offset, len, |bytes| crypto.keccak256(bytes))?;

        vm.current_call_frame
            .stack
            .push(u256_from_big_endian(&hash))?;

        Ok(OpcodeResult::Continue)
    }
}
