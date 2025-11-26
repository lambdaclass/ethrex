//! # Stack exchange operations
//!
//! Includes the following opcodes:
//!   - `SWAP1` to `SWAP16`

use crate::{
    errors::{OpcodeResult, VMError},
    gas_cost,
    opcode_handlers::OpcodeHandler,
    vm::VM,
};

/// Implementation for the `SWAPn` opcodes.
pub struct OpSwapHandler<const N: usize>;
impl<const N: usize> OpcodeHandler for OpSwapHandler<N> {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::SWAPN)?;

        vm.current_call_frame.stack.swap::<N>()?;

        Ok(OpcodeResult::Continue)
    }
}
