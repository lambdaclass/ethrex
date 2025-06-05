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

/// Returns the appropriate VM hooks based on whether the `l2` feature is enabled.
pub fn get_hooks(_tx: &Transaction) -> Vec<Rc<RefCell<dyn Hook + 'static>>> {
    #[cfg(not(feature = "l2"))]
    let hooks: Vec<Rc<RefCell<dyn Hook>>> = vec![Rc::new(RefCell::new(DefaultHook))];

    #[cfg(feature = "l2")]
    let hooks: Vec<Rc<RefCell<dyn Hook>>> = {
        use crate::{call_frame::CallFrameBackup, hooks::L2Hook};
        use ethrex_common::types::PrivilegedL2Transaction;

        let recipient = if let Transaction::PrivilegedL2Transaction(PrivilegedL2Transaction {
            recipient,
            ..
        }) = _tx
        {
            Some(*recipient)
        } else {
            None
        };

        vec![Rc::new(RefCell::new(L2Hook {
            recipient,
            callframe_backup: CallFrameBackup::default(), // It will be modified afterwards.
        }))]
    };

    hooks
}
