use crate::{
    account::{Account, StorageSlot},
    call_frame::CallFrame,
    constants::*,
    db::{
        cache::{self, get_account_mut, remove_account},
        CacheDB, Database,
    },
    environment::Environment,
    errors::{ExecutionReport, InternalError, OpcodeResult, TxResult, TxValidationError, VMError},
    gas_cost::{self, STANDARD_TOKEN_COST, TOTAL_COST_FLOOR_PER_TOKEN},
    precompiles::{
        execute_precompile, is_precompile, SIZE_PRECOMPILES_CANCUN, SIZE_PRECOMPILES_PRAGUE,
        SIZE_PRECOMPILES_PRE_CANCUN,
    },
    utils::*,
    AccountInfo, TransientStorage,
};

pub trait Hook {
    fn prepare_execution() -> Result<(), VMError>;

    fn finalize_execution() -> Result<(), VMError>;
}

struct DefaultHook {}

impl DefaultHook {
    pub fn new() -> DefaultHook {
        DefaultHook {}
    }
}

impl Hook for DefaultHook {
    fn prepare_execution() -> Result<(), VMError> {
        todo!();
    }

    fn finalize_execution() -> Result<(), VMError> {
        todo!();
    }
}
