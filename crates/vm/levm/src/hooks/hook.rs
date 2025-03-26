use crate::{
    call_frame::CallFrame,
    db::Database,
    errors::{ExecutionReport, VMError},
    vm::VM,
};

pub trait Hook {
    fn prepare_execution(
        &self,
        vm: &mut VM<impl Database>,
        initial_call_frame: &mut CallFrame,
    ) -> Result<(), VMError>;

    fn finalize_execution(
        &self,
        vm: &mut VM<impl Database>,
        initial_call_frame: &CallFrame,
        report: &mut ExecutionReport,
    ) -> Result<(), VMError>;
}
