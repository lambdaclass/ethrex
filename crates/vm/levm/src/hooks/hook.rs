use std::{cell::RefCell, rc::Rc};

use ethrex_common::types::Transaction;

use crate::{
    errors::{ExecutionReport, VMError},
    vm::VM,
};

pub trait Hook {
    fn prepare_execution(&mut self, vm: &mut VM<'_>) -> Result<(), VMError>;

    fn finalize_execution(
        &mut self,
        vm: &mut VM<'_>,
        report: &mut ExecutionReport,
    ) -> Result<(), VMError>;
}

pub fn get_hooks(_tx: &Transaction) -> Vec<Rc<RefCell<dyn Hook + 'static>>> {
    #[cfg(not(feature = "l2"))]
    {
        use crate::hooks::default_hook::DefaultHook;
        return vec![Rc::new(RefCell::new(DefaultHook))];
    }

    #[cfg(feature = "l2")]
    {
        use crate::{
            call_frame::CallFrameBackup,
            hooks::{backup_hook::BackupHook, L2Hook},
        };
        use ethrex_common::types::PrivilegedL2Transaction;

        let recipient = match _tx {
            Transaction::PrivilegedL2Transaction(PrivilegedL2Transaction { recipient, .. }) => {
                Some(*recipient)
            }
            _ => None,
        };

        return vec![
            Rc::new(RefCell::new(L2Hook { recipient })),
            Rc::new(RefCell::new(BackupHook {
                callframe_backup: CallFrameBackup::default(), // It will be modified in prepare_execution and finalize_execution
            })),
        ];
    }
}
