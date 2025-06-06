use std::fmt::Debug;

use crate::{
    call_frame::CallFrameBackup,
    errors::{ExecutionReport, VMError},
    hooks::hook::Hook,
    vm::VM,
};

#[derive(Default)]
pub struct BackupHook {
    pub callframe_backup: CallFrameBackup,
}

impl Debug for BackupHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackupHook")
            .field("callframe_backup", &self.callframe_backup)
            .finish()
    }
}

impl Hook for BackupHook {
    fn prepare_execution(&mut self, vm: &mut crate::vm::VM<'_>) -> Result<(), VMError> {
        // Here we need to backup the callframe for undoing transaction changes if we want to.
        self.callframe_backup = vm.current_call_frame()?.call_frame_backup.clone();
        Ok(())
    }

    fn finalize_execution(
        &mut self,
        vm: &mut VM<'_>,
        _report: &mut ExecutionReport,
    ) -> Result<(), VMError> {
        // We want to restore to the initial state, this includes saving the changes made by the prepare execution
        // and the changes made by the execution itself.
        let execution_backup = &mut vm.current_call_frame_mut()?.call_frame_backup;
        let pre_execution_backup = std::mem::take(&mut self.callframe_backup);
        execution_backup.extend(pre_execution_backup);
        Ok(())
    }
}
