use crate::{
    errors::{ExceptionalHalt, VMError},
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
    pub fn op_log<const N_TOPICS: usize>(&mut self) -> Result<bool, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        if current_call_frame.is_static {
            return Err(ExceptionalHalt::OpcodeNotAllowedInStaticContext.into());
        }

        let [offset, size] = *current_call_frame.stack.pop()?;
        let (size, offset) = size_offset_to_usize(size, offset)?;

        let topics = current_call_frame
            .stack
            .pop::<N_TOPICS>()?
            .map(|topic| H256(U256::to_big_endian(&topic)));

        let new_memory_size = calculate_memory_size(offset, size)?;

        current_call_frame.increase_consumed_gas(gas_cost::log(
            new_memory_size,
            current_call_frame.memory.len(),
            size,
            N_TOPICS,
        )?)?;

        let log = Log {
            address: current_call_frame.to,
            topics: topics.to_vec(),
            data: current_call_frame.memory.load_range(offset, size)?,
        };

        self.tracer.log(&log)?;

        self.substate.add_log(log);

        Ok(false)
    }
}
