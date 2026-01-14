use crate::{
    errors::{ContextResult, VMError},
    hooks::{L2Hook, backup_hook::BackupHook, default_hook::DefaultHook},
    vm::{VM, VMType},
};
use ethrex_common::types::fee_config::FeeConfig;

pub trait Hook {
    fn prepare_execution(&mut self, vm: &mut VM<'_>) -> Result<(), VMError>;

    fn finalize_execution(
        &mut self,
        vm: &mut VM<'_>,
        report: &mut ContextResult,
    ) -> Result<(), VMError>;
}

pub fn get_hooks(vm_type: &VMType) -> Vec<Box<dyn Hook + 'static>> {
    match vm_type {
        VMType::L1 => l1_hooks(),
        VMType::L2(fee_config) => l2_hooks(*fee_config),
    }
}

pub fn l1_hooks() -> Vec<Box<dyn Hook + 'static>> {
    vec![Box::new(DefaultHook)]
}

pub fn l2_hooks(fee_config: FeeConfig) -> Vec<Box<dyn Hook + 'static>> {
    vec![
        Box::new(L2Hook { fee_config }),
        Box::new(BackupHook::default()),
    ]
}
