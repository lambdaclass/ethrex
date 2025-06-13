use std::{cell::RefCell, rc::Rc};

use ethrex_common::types::Transaction;

use crate::{
    errors::{ContextResult, VMError},
    vm::VM,
};

pub trait Hook {
    fn prepare_execution(&mut self, vm: &mut VM<'_>) -> Result<(), VMError>;

    fn finalize_execution(
        &mut self,
        vm: &mut VM<'_>,
        report: &mut ContextResult,
    ) -> Result<(), VMError>;
}

pub fn get_hooks(_tx: &Transaction) -> Vec<Rc<RefCell<dyn Hook + 'static>>> {
    #[cfg(not(feature = "l2"))]
    {
        use crate::hooks::default_hook::DefaultHook;
        vec![Rc::new(RefCell::new(DefaultHook))]
    }

    #[cfg(feature = "l2")]
    {
        use crate::hooks::{L2Hook, backup_hook::BackupHook};
        use ethrex_common::types::PrivilegedL2Transaction;

        let recipient = match _tx {
            Transaction::PrivilegedL2Transaction(PrivilegedL2Transaction { recipient, .. }) => {
                Some(*recipient)
            }
            _ => None,
        };

        vec![
            Rc::new(RefCell::new(L2Hook { recipient })),
            Rc::new(RefCell::new(BackupHook::default())),
        ]
    }
}
