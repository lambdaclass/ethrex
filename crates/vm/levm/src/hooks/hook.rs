use std::{cell::RefCell, rc::Rc};

use ethrex_common::types::{PrivilegedL2Transaction, Transaction};

use crate::{
    errors::{ContextResult, VMError},
    hooks::{L2Hook, backup_hook::BackupHook, default_hook::DefaultHook},
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

pub fn l1_hooks() -> Vec<Rc<RefCell<dyn Hook + 'static>>> {
    vec![Rc::new(RefCell::new(DefaultHook))]
}

pub fn l2_hooks(tx: &Transaction) -> Vec<Rc<RefCell<dyn Hook + 'static>>> {
    let recipient =
        if let Transaction::PrivilegedL2Transaction(PrivilegedL2Transaction { recipient, .. }) = tx
        {
            Some(*recipient)
        } else {
            None
        };

    vec![
        Rc::new(RefCell::new(L2Hook { recipient })),
        Rc::new(RefCell::new(BackupHook::default())),
    ]
}
