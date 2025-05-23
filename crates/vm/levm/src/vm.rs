#[cfg(feature = "l2")]
use crate::call_frame::CallFrameBackup;
use crate::{
    call_frame::CallFrame,
    db::gen_db::GeneralizedDatabase,
    environment::Environment,
    errors::{ExecutionReport, OpcodeResult, PrecompileError, VMError},
    hooks::hook::Hook,
    precompiles::execute_precompile,
    TransientStorage,
};
use bytes::Bytes;
use ethrex_common::{
    types::{Transaction, TxKind},
    Address, H256, U256,
};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fmt::Debug,
    sync::Arc,
};

pub type Storage = HashMap<U256, H256>;

#[derive(Debug, Clone, Default)]
/// Information that changes during transaction execution
pub struct Substate {
    pub selfdestruct_set: HashSet<Address>,
    pub touched_accounts: HashSet<Address>,
    pub touched_storage_slots: HashMap<Address, BTreeSet<H256>>,
    pub created_accounts: HashSet<Address>,
    pub refunded_gas: u64,
    pub transient_storage: TransientStorage,
}

pub struct VM<'a> {
    pub call_frames: Vec<CallFrame>,
    pub env: Environment,
    pub substate: Substate,
    pub db: &'a mut GeneralizedDatabase,
    pub tx: Transaction,
    pub hooks: Vec<Arc<dyn Hook>>,
    pub substate_backups: Vec<Substate>,
    /// Original storage values before the transaction. Used for gas calculations in SSTORE.
    pub storage_original_values: HashMap<Address, HashMap<H256, U256>>,
}

impl<'a> VM<'a> {
    pub fn new(env: Environment, db: &'a mut GeneralizedDatabase, tx: &Transaction) -> Self {
        let hooks = Self::get_hooks(tx);

        Self {
            call_frames: vec![],
            env,
            substate: Substate::default(),
            db,
            tx: tx.clone(),
            hooks,
            substate_backups: vec![],
            storage_original_values: HashMap::new(),
        }
    }

    /// Initializes substate and creates first execution callframe.
    pub fn setup_vm(&mut self) -> Result<(), VMError> {
        self.initialize_substate()?;

        let callee = self.get_tx_callee()?;

        let initial_call_frame = CallFrame::new(
            self.env.origin,
            callee,
            Address::default(), // Will be assigned at the end of prepare_execution
            Bytes::new(),       // Will be assigned at the end of prepare_execution
            self.tx.value(),
            self.tx.data().clone(),
            false,
            self.env.gas_limit,
            0,
            true,
            false,
            U256::zero(),
            0,
        );

        self.call_frames.push(initial_call_frame);

        Ok(())
    }

    /// Executes a whole external transaction. Performing validations at the beginning.
    pub fn execute(&mut self) -> Result<ExecutionReport, VMError> {
        self.setup_vm()?;

        if let Err(e) = self.prepare_execution() {
            // Restore cache to state previous to this Tx execution because this Tx is invalid.
            self.restore_cache_state()?;
            return Err(e);
        }

        // Here we need to backup the callframe because in the L2 we want to undo a transaction if it exceeds blob size
        // even if the transaction succeeds.
        #[cfg(feature = "l2")]
        let callframe_backup = self.current_call_frame()?.call_frame_backup.clone();

        // Clear callframe backup so that changes made in prepare_execution are written in stone.
        // We want to apply these changes even if the Tx reverts. E.g. Incrementing sender nonce
        self.current_call_frame_mut()?.call_frame_backup.clear();

        if self.is_create() {
            // Create contract, reverting the Tx if address is already occupied.
            if let Some(mut report) = self.handle_create_transaction()? {
                self.finalize_execution(&mut report)?;
                return Ok(report);
            }
        }

        self.backup_substate();
        let mut report = self.run_execution()?;

        self.finalize_execution(&mut report)?;

        // We want to restore to the initial state, this includes reverting the changes made by the prepare execution
        // and the changes made by the execution itself.
        #[cfg(feature = "l2")]
        {
            let current_backup: &mut CallFrameBackup =
                &mut self.current_call_frame_mut()?.call_frame_backup;
            current_backup
                .original_accounts_info
                .extend(callframe_backup.original_accounts_info);
            current_backup
                .original_account_storage_slots
                .extend(callframe_backup.original_account_storage_slots);
        }
        Ok(report)
    }

    /// Main execution loop.
    pub fn run_execution(&mut self) -> Result<ExecutionReport, VMError> {
        if self.is_precompile()? {
            return self.execute_precompile();
        }

        loop {
            let opcode = self.current_call_frame()?.next_opcode();

            let op_result = self.execute_opcode(opcode);

            let result = match op_result {
                Ok(OpcodeResult::Continue { pc_increment }) => {
                    self.increment_pc_by(pc_increment)?;
                    continue;
                }
                Ok(OpcodeResult::Halt) => self.handle_opcode_result()?,
                Err(error) => self.handle_opcode_error(error)?,
            };

            // Return the ExecutionReport if the executed callframe was the first one.
            if self.is_initial_call_frame() {
                return Ok(result);
            }

            // Handle interaction between child and parent callframe.
            self.handle_return(&result)?;
        }
    }

    pub fn execute_precompile(&mut self) -> Result<ExecutionReport, VMError> {
        let precompile_address = self.current_call_frame()?.code_address;

        let precompile_result = match self.is_delegation_target(precompile_address) {
            // Avoid executing precompile if it is target of a delegation in EIP-7702 transaction.
            true => {
                let gas_limit = self.current_call_frame()?.gas_limit;
                if gas_limit == 0 {
                    // `pointer_to_precompile.json` tests that it should fail in a call with zero gas limit.
                    Err(VMError::PrecompileError(PrecompileError::NotEnoughGas))
                } else {
                    Ok(Bytes::new())
                }
            }
            // Otherwise, execute precompile
            false => {
                let callframe = self.current_call_frame_mut()?;
                execute_precompile(
                    precompile_address,
                    &callframe.calldata,
                    &mut callframe.gas_used,
                    callframe.gas_limit,
                )
            }
        };

        let report = self.handle_precompile_result(precompile_result)?;

        Ok(report)
    }

    pub fn restore_state(&mut self, backup: Substate) -> Result<(), VMError> {
        self.restore_cache_state()?;
        self.substate = backup;
        Ok(())
    }

    /// True if external transaction is a contract creation
    pub fn is_create(&self) -> bool {
        matches!(self.tx.to(), TxKind::Create)
    }

    /// Executes without making changes to the cache.
    pub fn stateless_execute(&mut self) -> Result<ExecutionReport, VMError> {
        let cache_backup = self.db.cache.clone();
        let report = self.execute()?;
        // Restore the cache to its original state
        self.db.cache = cache_backup;
        Ok(report)
    }

    fn prepare_execution(&mut self) -> Result<(), VMError> {
        // NOTE: ATTOW the default hook is created in VM::new(), so
        // (in theory) _at least_ the default prepare execution should
        // run
        for hook in self.hooks.clone() {
            hook.prepare_execution(self)?;
        }
        Ok(())
    }

    fn finalize_execution(&mut self, report: &mut ExecutionReport) -> Result<(), VMError> {
        // NOTE: ATTOW the default hook is created in VM::new(), so
        // (in theory) _at least_ the default finalize execution should
        // run
        for hook in self.hooks.clone() {
            hook.finalize_execution(self, report)?;
        }

        Ok(())
    }
}
