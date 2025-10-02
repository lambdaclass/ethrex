use std::cell::OnceCell;

use crate::{
    errors::{ExceptionalHalt, OpcodeResult, VMError},
    gas_cost,
    memory::calculate_memory_size,
    utils::size_offset_to_usize,
    vm::VM,
};
use ethrex_common::{H256, U256, types::Log};

// Logging Operations (5)
// Opcodes: LOG0 ... LOG4

impl<'a> VM<'a> {
    // LOG operation
    pub fn op_log<const N_TOPICS: usize>(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if self.current_call_frame.is_static {
            error.set(ExceptionalHalt::OpcodeNotAllowedInStaticContext.into());
            return OpcodeResult::Halt;
        }

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

        let topics = match self.current_call_frame.stack.pop::<N_TOPICS>() {
            Ok(x) => x.map(|topic| H256(U256::to_big_endian(&topic))),
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

        if let Err(err) = gas_cost::log(
            new_memory_size,
            self.current_call_frame.memory.len(),
            size,
            N_TOPICS,
        )
        .and_then(|x| Ok(self.current_call_frame.increase_consumed_gas(x)?))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let log = Log {
            address: self.current_call_frame.to,
            topics: topics.to_vec(),
            data: match self.current_call_frame.memory.load_range(offset, size) {
                Ok(x) => x,
                Err(err) => {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            },
        };

        if let Err(err) = self.tracer.log(&log) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        self.substate.add_log(log);

        OpcodeResult::Continue
    }
}
