use std::cell::OnceCell;

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
    pub fn op_keccak256(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        let [offset, size] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        let (size, offset) = match size_offset_to_usize(size, offset) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        let new_memory_size = match calculate_memory_size(offset, size) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        if let Err(err) =
            gas_cost::keccak256(new_memory_size, self.current_call_frame.memory.len(), size)
                .and_then(|x| Ok(self.current_call_frame.increase_consumed_gas(x)?))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let mut hasher = Keccak256::new();
        hasher.update(
            match self.current_call_frame.memory.load_range(offset, size) {
                Ok(x) => x,
                Err(err) => {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            },
        );
        if let Err(err) = self
            .current_call_frame
            .stack
            .push1(u256_from_big_endian(&hasher.finalize()))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }
}
