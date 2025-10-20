use ethrex_common::Address;

use crate::hooks::hook::Hook;

pub struct FeeTokenHook {
    pub fee_token_address: Address,
}

impl Hook for FeeTokenHook {
    fn prepare_execution(
        &mut self,
        _vm: &mut crate::vm::VM<'_>,
    ) -> Result<(), crate::errors::VMError> {
        dbg!("In FeeTokenHook prepare_execution");
        Ok(())
    }

    fn finalize_execution(
        &mut self,
        _vm: &mut crate::vm::VM<'_>,
        _report: &mut crate::errors::ContextResult,
    ) -> Result<(), crate::errors::VMError> {
        dbg!("In FeeTokenHook finalize_execution");
        Ok(())
    }
}
