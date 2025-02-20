use crate::hooks::hook::Hook;
use std::cell::RefCell;
pub struct GasCounter {
    counter: Cell<u64>,
}

impl Hook for GasCounter {
    fn prepare_execution(
        &self,
        vm: &mut VM,
        initial_call_frame: &mut CallFrame,
    ) -> Result<(), VMError> {
    }
    fn finalize_execution(
        &self,
        vm: &mut VM,
        _: &CallFrame,
        report: &mut ExecutionReport,
    ) -> Result<(), VMError> {
        self.counter.update(|gas_used| gas_used + report.gas_used)
    }
}
