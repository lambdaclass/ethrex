use crate::hooks::hook::Hook;

pub struct EmptyHook;

/// This hook does nothing, it is used for simulating transactions.
impl Hook for EmptyHook {
    fn prepare_execution(
        &mut self,
        _vm: &mut crate::vm::VM<'_>,
    ) -> Result<(), crate::errors::VMError> {
        Ok(())
    }

    fn finalize_execution(
        &mut self,
        _vm: &mut crate::vm::VM<'_>,
        _report: &mut crate::errors::ContextResult,
    ) -> Result<(), crate::errors::VMError> {
        Ok(())
    }
}
