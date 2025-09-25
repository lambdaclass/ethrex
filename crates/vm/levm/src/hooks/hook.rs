use std::{cell::RefCell, rc::Rc};

use ethrex_common::Address;

use crate::{
    errors::{ContextResult, VMError},
    hooks::{L2Hook, backup_hook::BackupHook, default_hook::DefaultHook},
    vm::{VM, VMType},
};

pub trait Hook {
    fn prepare_execution(&mut self, vm: &mut VM<'_>) -> Result<(), VMError>;

    fn finalize_execution(
        &mut self,
        vm: &mut VM<'_>,
        report: &mut ContextResult,
    ) -> Result<(), VMError>;
}

pub fn get_hooks(
    vm_type: &VMType,
    fee_vault: Option<Address>,
) -> Vec<Rc<RefCell<dyn Hook + 'static>>> {
    match vm_type {
        VMType::L1 => l1_hooks(),
        VMType::L2 => l2_hooks(fee_vault),
    }
}

pub fn l1_hooks() -> Vec<Rc<RefCell<dyn Hook + 'static>>> {
    vec![Rc::new(RefCell::new(DefaultHook))]
}

pub fn l2_hooks(fee_vault: Option<Address>) -> Vec<Rc<RefCell<dyn Hook + 'static>>> {
    vec![
        Rc::new(RefCell::new(L2Hook { fee_vault })),
        Rc::new(RefCell::new(BackupHook::default())),
    ]
}
