use crate::{
    call_frame::CallFrameBackup,
    errors::{ContextResult, VMError},
    hooks::hook::Hook,
    vm::VM,
};

#[derive(Default)]
pub struct BackupHook {
    /// We need to store this because we clear the backup after `prepare_execution` hook is executed
    pub pre_execution_backup: CallFrameBackup,
}

impl Hook for BackupHook {
    fn prepare_execution(&mut self, vm: &mut crate::vm::VM<'_>) -> Result<(), VMError> {
        // Here we need to backup the callframe for undoing transaction changes if we want to.
        self.pre_execution_backup = vm.current_call_frame.call_frame_backup.clone();
        Ok(())
    }

    fn finalize_execution(
        &mut self,
        vm: &mut VM<'_>,
        _ctx_result: &mut ContextResult,
    ) -> Result<(), VMError> {
        // We want to restore to the initial state, this includes saving the changes made by the prepare execution
        // and the changes made by the execution itself.
        let mut execution_backup = vm.current_call_frame.call_frame_backup.clone();
        let pre_execution_backup = std::mem::take(&mut self.pre_execution_backup);
        execution_backup.extend(pre_execution_backup);
        vm.db.tx_backup = Some(execution_backup);

        Ok(())
    }
}
