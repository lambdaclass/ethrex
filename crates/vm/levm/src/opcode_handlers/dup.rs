//! # Stack duplication operations
//!
//! Includes the following opcodes:
//!   - `DUP1` to `DUP16`

use crate::{
    errors::{OpcodeResult, VMError},
    gas_cost,
    opcode_handlers::OpcodeHandler,
    vm::VM,
};

/// Implementation for the `DUPn` opcodes.
pub struct OpDupHandler<const N: usize>;
impl<const N: usize> OpcodeHandler for OpDupHandler<N> {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::DUPN)?;

        vm.current_call_frame.stack.dup::<N>()?;

        Ok(OpcodeResult::Continue)
    }
}
