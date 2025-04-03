use crate::{
    call_frame::CallFrame,
    errors::{ExecutionReport, VMError},
    vm::VM,
};

pub trait Hook {
    fn prepare_execution(
        &self,
        vm: &mut VM<'_>,
        initial_call_frame: &mut CallFrame,
    ) -> Result<(), VMError>;

    fn finalize_execution(
        &self,
        vm: &mut VM<'_>,
        initial_call_frame: &CallFrame,
        report: &mut ExecutionReport,
    ) -> Result<(), VMError>;
}
