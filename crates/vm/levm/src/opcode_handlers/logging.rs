use crate::{
    call_frame::CallFrame,
    errors::{OpcodeSuccess, VMError},
    gas_cost,
    memory::{self, calculate_memory_size},
    vm::VM,
};
use bytes::Bytes;
use ethrex_core::{types::Log, H256};

// Logging Operations (5)
// Opcodes: LOG0 ... LOG4

impl VM {
    // LOG operation
    pub fn op_log(
        &mut self,
        current_call_frame: &mut CallFrame,
        number_of_topics: u8,
    ) -> Result<OpcodeSuccess, VMError> {
        if current_call_frame.is_static {
            return Err(VMError::OpcodeNotAllowedInStaticContext);
        }

        let offset = current_call_frame.stack.pop()?;
        let size = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;
        let mut topics = Vec::new();
        for _ in 0..number_of_topics {
            let topic = current_call_frame.stack.pop()?;
            let mut topic_bytes = [0u8; 32];
            topic.to_big_endian(&mut topic_bytes);
            topics.push(H256::from_slice(&topic_bytes));
        }

        let new_memory_size = calculate_memory_size(offset, size)?;

        self.increase_consumed_gas(
            current_call_frame,
            gas_cost::log(
                new_memory_size,
                current_call_frame.memory.len(),
                size,
                number_of_topics,
            )?,
        )?;

        let log = Log {
            address: current_call_frame.to,
            topics,
            data: Bytes::from(
                memory::load_range(&mut current_call_frame.memory, offset, size)?.to_vec(),
            ),
        };
        current_call_frame.logs.push(log);

        Ok(OpcodeSuccess::Continue)
    }
}
