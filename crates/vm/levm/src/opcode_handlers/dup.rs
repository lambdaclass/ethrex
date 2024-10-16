// Duplication Operation (16)
// Opcodes: DUP1 ... DUP16
use super::*;

impl VM {
    // DUP operation
    pub fn op_dup(
        &mut self,
        current_call_frame: &mut CallFrame,
        op: Opcode,
    ) -> Result<OpcodeSuccess, VMError> {
        // Calculate the depth based on the opcode
        let depth = (op as u8) - (Opcode::DUP1 as u8) + 1;

        // Check for gas consumption
        if self.env.consumed_gas + gas_cost::DUPN > self.env.gas_limit {
            return Err(VMError::OutOfGas);
        }

        // Ensure the stack has enough elements to duplicate
        if current_call_frame.stack.len() < depth as usize {
            return Err(VMError::StackUnderflow);
        }

        // Get the value at the specified depth
        let value_at_depth = current_call_frame
            .stack
            .get(current_call_frame.stack.len() - depth as usize)?;

        // Push the duplicated value onto the stack
        current_call_frame.stack.push(*value_at_depth)?;

        // Update the consumed gas
        self.env.consumed_gas += gas_cost::DUPN;

        Ok(OpcodeSuccess::Continue)
    }
}
