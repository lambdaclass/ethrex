// Logging Operations (5)
// Opcodes: LOG0 ... LOG4

use crate::{
    call_frame::CallFrame,
    constants::gas_cost,
    errors::{OpcodeSuccess, VMError},
    opcodes::Opcode,
    vm::VM,
};
use bytes::Bytes;
use ethereum_rust_core::{types::Log, H256};

impl VM {
    // LOG operation
    pub fn op_log(
        &mut self,
        current_call_frame: &mut CallFrame,
        op: Opcode,
    ) -> Result<OpcodeSuccess, VMError> {
        if current_call_frame.is_static {
            return Err(VMError::OpcodeNotAllowedInStaticContext);
        }

        let number_of_topics = (op as u8) - (Opcode::LOG0 as u8);
        let offset = current_call_frame
            .stack
            .pop()?
            .try_into()
            .unwrap_or(usize::MAX);
        let size = current_call_frame
            .stack
            .pop()?
            .try_into()
            .unwrap_or(usize::MAX);
        let mut topics = Vec::new();
        for _ in 0..number_of_topics {
            let topic = current_call_frame.stack.pop()?;
            let mut topic_bytes = [0u8; 32];
            topic.to_big_endian(&mut topic_bytes);
            topics.push(H256::from_slice(&topic_bytes));
        }

        let memory_expansion_cost = current_call_frame.memory.expansion_cost(offset + size)?;
        let gas_cost = gas_cost::LOGN_STATIC
            + gas_cost::LOGN_DYNAMIC_BASE * number_of_topics
            + gas_cost::LOGN_DYNAMIC_BYTE_BASE * size
            + memory_expansion_cost;

        self.increase_consumed_gas(current_call_frame, gas_cost)?;

        let data = current_call_frame.memory.load_range(offset, size);
        let log = Log {
            address: current_call_frame.msg_sender, // Should change the addr if we are on a Call/Create transaction (Call should be the contract we are calling, Create should be the original caller)
            topics,
            data: Bytes::from(data),
        };
        current_call_frame.logs.push(log);

        Ok(OpcodeSuccess::Continue)
    }
}
