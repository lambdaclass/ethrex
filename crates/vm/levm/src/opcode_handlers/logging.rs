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
    pub fn op_log<const N_TOPICS: usize>(&mut self) -> Result<OpcodeResult, VMError> {
        if self.current_call_frame.is_static {
            return Err(ExceptionalHalt::OpcodeNotAllowedInStaticContext.into());
        }

        let [offset, size] = *self.current_stack().pop()?;
        let (size, offset) = size_offset_to_usize(size, offset)?;

        let topics = self
            .current_stack()
            .pop::<N_TOPICS>()?
            .map(|topic| H256(U256::to_big_endian(&topic)));

        let new_memory_size = calculate_memory_size(offset, size)?;

        self.current_call_frame
            .increase_consumed_gas(gas_cost::log(
                new_memory_size,
                self.current_call_frame.memory.len(),
                size,
                N_TOPICS,
            )?)?;

        let log = Log {
            address: self.current_call_frame.to,
            topics: topics.to_vec(),
            data: self.current_call_frame.memory.load_range(offset, size)?,
        };

        self.tracer.log(&log)?;

        self.substate.add_log(log);

        Ok(OpcodeResult::Continue)
    }
}
