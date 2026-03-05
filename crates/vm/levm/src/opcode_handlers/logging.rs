//! # Logging operations
//!
//! Includes the following opcodes:
//!   - `LOG0` to `LOG4`

use std::mem;

use crate::{
    errors::{ExceptionalHalt, OpcodeResult, VMError},
    gas_cost,
    memory::calculate_memory_size,
    opcode_handlers::OpcodeHandler,
    utils::size_offset_to_usize,
    vm::VM,
};
use ethrex_common::{H256, U256, types::Log};

/// Implementation for the `LOGn` opcodes.
pub struct OpLogHandler<const N: usize>;
impl<const N: usize> OpcodeHandler for OpLogHandler<N> {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        if vm.current_call_frame.is_static {
            return Err(ExceptionalHalt::OpcodeNotAllowedInStaticContext.into());
        }

        let [offset, len] = *vm.current_call_frame.stack.pop()?;
        let topics = vm.current_call_frame.stack.pop::<N>()?.map(|topic| {
            #[expect(unsafe_code)]
            unsafe {
                let mut hash = mem::transmute::<U256, H256>(topic);
                hash.0.reverse();
                hash
            }
        });
        let (len, offset) = size_offset_to_usize(len, offset)?;

        vm.current_call_frame.increase_consumed_gas(gas_cost::log(
            calculate_memory_size(offset, len)?,
            vm.current_call_frame.memory.len(),
            len,
            N,
        )?)?;

        let log = Log {
            address: vm.current_call_frame.to,
            topics: topics.into(),
            data: vm.current_call_frame.memory.load_range(offset, len)?,
        };
        vm.tracer.log(&log)?;
        vm.substate.add_log(log);

        Ok(OpcodeResult::Continue)
    }
}
