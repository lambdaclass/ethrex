use crate::{
    TransientStorage,
    call_frame::{CallFrame, Stack},
    db::gen_db::GeneralizedDatabase,
    debug::DebugMode,
    environment::Environment,
    errors::{ContextResult, ExecutionReport, InternalError, OpcodeResult, VMError},
    hooks::{
        backup_hook::BackupHook,
        hook::{Hook, get_hooks},
    },
    l2_precompiles, precompiles,
    tracing::LevmCallTracer,
};
use bytes::Bytes;
use ethrex_common::{
    Address, H160, H256, U256,
    tracing::CallType,
    types::{Log, Transaction},
};
use std::{
    cell::RefCell,
    collections::{BTreeSet, HashMap, HashSet},
    rc::Rc,
    sync::{Arc, RwLock},
};

pub type Storage = HashMap<U256, H256>;

#[derive(Debug, Clone, Default)]
pub enum VMType {
    #[default]
    L1,
    L2,
}

#[derive(Debug, Clone, Default)]
/// Information that changes during transaction execution
pub struct Substate {
    pub selfdestruct_set: HashSet<Address>,
    pub accessed_addresses: HashSet<Address>,
    pub accessed_storage_slots: HashMap<Address, BTreeSet<H256>>,
    pub created_accounts: HashSet<Address>,
    pub refunded_gas: u64,
    pub transient_storage: TransientStorage,
    pub logs: Vec<Log>,
}

pub struct VM<'a> {
    pub call_frames: Vec<CallFrame>,
    pub env: Environment,
    pub substate: Substate,
    pub db: &'a mut GeneralizedDatabase,
    pub tx: Transaction,
    pub hooks: Vec<Rc<RefCell<dyn Hook>>>,
    pub substate_backups: Vec<Substate>,
    /// Original storage values before the transaction. Used for gas calculations in SSTORE.
    pub storage_original_values: HashMap<(Address, H256), U256>,
    /// When enabled, it "logs" relevant information during execution
    pub tracer: LevmCallTracer,
    /// Mode for printing some useful stuff, only used in development!
    pub debug_mode: DebugMode,
    /// A pool of stacks to avoid reallocating too much when creating new call frames.
    pub stack_pool: Vec<Stack>,
    pub vm_type: VMType,
    pub cache: LevmCache,
}

/// A cache used to store data held between blocks / transactions. Such as bytecode invalid jump destinations.
#[derive(Debug, Default, Clone)]
pub struct LevmCache {
    pub invalid_jump_dests: Arc<RwLock<HashMap<H160, Box<[usize]>>>>,
}

impl<'a> VM<'a> {
    pub fn new(
        env: Environment,
        db: &'a mut GeneralizedDatabase,
        tx: &Transaction,
        tracer: LevmCallTracer,
        vm_type: VMType,
        cache: LevmCache,
    ) -> Self {
        db.tx_backup = None; // If BackupHook is enabled, it will contain backup at the end of tx execution.

        Self {
            call_frames: vec![],
            env,
            substate: Substate::default(),
            db,
            tx: tx.clone(),
            hooks: get_hooks(&vm_type),
            substate_backups: vec![],
            storage_original_values: HashMap::new(),
            tracer,
            debug_mode: DebugMode::disabled(),
            stack_pool: Vec::new(),
            vm_type,
            cache,
        }
    }

    fn add_hook(&mut self, hook: impl Hook + 'static) {
        self.hooks.push(Rc::new(RefCell::new(hook)));
    }

    /// Initializes substate and creates first execution callframe.
    pub fn setup_vm(&mut self) -> Result<(), VMError> {
        self.initialize_substate()?;

        let (callee, is_create) = self.get_tx_callee()?;

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
            is_create,
            U256::zero(),
            0,
            Stack::default(),
        );

        self.call_frames.push(initial_call_frame);

        let call_type = if is_create {
            CallType::CREATE
        } else {
            CallType::CALL
        };
        self.tracer.enter(
            call_type,
            self.env.origin,
            callee,
            self.tx.value(),
            self.env.gas_limit,
            self.tx.data(),
        );

        #[cfg(feature = "debug")]
        {
            // Enable debug mode for printing in Solidity contracts.
            self.debug_mode.enabled = true;
        }

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

        // Clear callframe backup so that changes made in prepare_execution are written in stone.
        // We want to apply these changes even if the Tx reverts. E.g. Incrementing sender nonce
        self.current_call_frame_mut()?.call_frame_backup.clear();

        if self.is_create()? {
            // Create contract, reverting the Tx if address is already occupied.
            if let Some(context_result) = self.handle_create_transaction()? {
                let report = self.finalize_execution(context_result)?;
                return Ok(report);
            }
        }

        self.backup_substate();
        let context_result = self.run_execution()?;

        let report = self.finalize_execution(context_result)?;

        Ok(report)
    }

    /// Main execution loop.
    pub fn run_execution(&mut self) -> Result<ContextResult, VMError> {
        if self.is_precompile(&self.current_call_frame()?.to) {
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
                self.handle_state_backup(&result)?;
                return Ok(result);
            }

            // Handle interaction between child and parent callframe.
            self.handle_return(&result)?;
        }
    }

    /// Executes precompile and handles the output that it returns, generating a report.
    pub fn execute_precompile(&mut self) -> Result<ContextResult, VMError> {
        let vm_type = self.vm_type.clone();

        let callframe = self.current_call_frame_mut()?;

        let precompile_result = match vm_type {
            VMType::L1 => precompiles::execute_precompile(
                callframe.code_address,
                &callframe.calldata,
                &mut callframe.gas_remaining,
            ),
            VMType::L2 => l2_precompiles::execute_precompile(
                callframe.code_address,
                &callframe.calldata,
                &mut callframe.gas_remaining,
            ),
        };

        let ctx_result = self.handle_precompile_result(precompile_result)?;

        Ok(ctx_result)
    }

    /// True if external transaction is a contract creation
    pub fn is_create(&self) -> Result<bool, InternalError> {
        Ok(self.current_call_frame()?.is_create)
    }

    /// Executes without making changes to the cache.
    pub fn stateless_execute(&mut self) -> Result<ExecutionReport, VMError> {
        // Add backup hook to restore state after execution.
        self.add_hook(BackupHook::default());
        let report = self.execute()?;
        // Restore cache to the state before execution.
        self.db.undo_last_transaction()?;
        Ok(report)
    }

    fn prepare_execution(&mut self) -> Result<(), VMError> {
        for hook in self.hooks.clone() {
            hook.borrow_mut().prepare_execution(self)?;
        }

        Ok(())
    }

    fn finalize_execution(
        &mut self,
        mut ctx_result: ContextResult,
    ) -> Result<ExecutionReport, VMError> {
        for hook in self.hooks.clone() {
            hook.borrow_mut()
                .finalize_execution(self, &mut ctx_result)?;
        }

        self.tracer.exit_context(&ctx_result, true)?;

        let report = ExecutionReport {
            result: ctx_result.result.clone(),
            gas_used: ctx_result.gas_used,
            gas_refunded: self.substate.refunded_gas,
            output: std::mem::take(&mut ctx_result.output),
            logs: self.substate.logs.clone(),
        };

        Ok(report)
    }
}
