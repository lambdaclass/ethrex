use crate::{
    errors::{ExecutionReport, VMError},
    vm::VM,
};

pub trait Hook {
    fn prepare_execution(&self, vm: &mut VM<'_>) -> Result<(), VMError>;

    fn finalize_execution(
        &self,
        vm: &mut VM<'_>,
        report: &mut ExecutionReport,
    ) -> Result<(), VMError>;
}
