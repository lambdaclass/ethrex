use crate::{
    call_frame::CallFrame,
    errors::{OpcodeSuccess, VMError},
    gas_cost,
    opcodes::Opcode,
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
        op: Opcode,
    ) -> Result<OpcodeSuccess, VMError> {
        if current_call_frame.is_static {
            return Err(VMError::OpcodeNotAllowedInStaticContext);
        }

        let number_of_topics = (u8::from(op))
            .checked_sub(u8::from(Opcode::LOG0))
            .ok_or(VMError::InvalidOpcode)?;

        let offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;
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

        let gas_cost = gas_cost::log(current_call_frame, size, offset, number_of_topics)
            .map_err(VMError::OutOfGas)?;

        self.increase_consumed_gas(current_call_frame, gas_cost)?;

        let data = current_call_frame.memory.load_range(offset, size)?;
        let log = Log {
            address: current_call_frame.msg_sender, // Should change the addr if we are on a Call/Create transaction (Call should be the contract we are calling, Create should be the original caller)
            topics,
            data: Bytes::from(data),
        };
        current_call_frame.logs.push(log);

        Ok(OpcodeSuccess::Continue)
    }
}
