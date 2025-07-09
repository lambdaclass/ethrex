use crate::{
    errors::{ExceptionalHalt, OpcodeResult, VMError},
    gas_cost,
    memory::{self, calculate_memory_size},
    vm::VM,
};
use bytes::Bytes;
use ethrex_common::{H256, U256, types::Log};

// Logging Operations (5)
// Opcodes: LOG0 ... LOG4

impl<'a> VM<'a> {
    // LOG operation
    pub fn op_log<const N_TOPICS: usize>(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        if current_call_frame.is_static {
            return Err(ExceptionalHalt::OpcodeNotAllowedInStaticContext.into());
        }

        let [offset, size] = *current_call_frame.stack.pop()?;
        let size = size
            .try_into()
            .map_err(|_| ExceptionalHalt::VeryLargeNumber)?;
        let offset = match offset.try_into() {
            Ok(x) => x,
            Err(_) => usize::MAX,
        };
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
            data: Bytes::from(
                memory::load_range(&mut current_call_frame.memory, offset, size)?.to_vec(),
            ),
        };

        self.tracer.log(&log)?;

        self.substate.logs.push(log);

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }
}
