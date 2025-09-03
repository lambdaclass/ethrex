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
    memory::Memory,
    precompiles::{
        self, SIZE_PRECOMPILES_CANCUN, SIZE_PRECOMPILES_PRAGUE, SIZE_PRECOMPILES_PRE_CANCUN,
    },
    tracing::LevmCallTracer,
};
use bytes::Bytes;
use ethrex_common::{
    Address, H160, H256, U256,
    tracing::CallType,
    types::{Fork, Log, Transaction},
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    rc::Rc,
};

pub type Storage = HashMap<U256, H256>;

#[derive(Debug, Clone, Copy, Default)]
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
    pub accessed_storage_slots: BTreeMap<Address, BTreeSet<H256>>,
    pub created_accounts: HashSet<Address>,
    pub refunded_gas: u64,
    pub transient_storage: TransientStorage,
    pub logs: Vec<Log>,
}

pub struct VM<'a> {
    /// Parent callframes.
    pub call_frames: Vec<CallFrame>,
    /// The current call frame.
    pub current_call_frame: CallFrame,
    pub env: Environment,
    pub substate: Substate,
    pub db: &'a mut GeneralizedDatabase,
    pub tx: Transaction,
    pub hooks: Vec<Rc<RefCell<dyn Hook>>>,
    pub substate_backups: Vec<Substate>,
    /// Original storage values before the transaction. Used for gas calculations in SSTORE.
    pub storage_original_values: BTreeMap<(Address, H256), U256>,
    /// When enabled, it "logs" relevant information during execution
    pub tracer: LevmCallTracer,
    /// Mode for printing some useful stuff, only used in development!
    pub debug_mode: DebugMode,
    /// A pool of stacks to avoid reallocating too much when creating new call frames.
    pub stack_pool: Vec<Stack>,
    pub vm_type: VMType,
}

impl<'a> VM<'a> {
    pub fn new(
        env: Environment,
        db: &'a mut GeneralizedDatabase,
        tx: &Transaction,
        tracer: LevmCallTracer,
        vm_type: VMType,
    ) -> Result<Self, VMError> {
        db.tx_backup = None; // If BackupHook is enabled, it will contain backup at the end of tx execution.

        let mut substate = Substate::initialize(&env, tx)?;

        let (callee, is_create) = Self::get_tx_callee(tx, db, &env, &mut substate)?;

        let mut vm = Self {
            call_frames: Vec::new(),
            substate,
            db,
            tx: tx.clone(),
            hooks: get_hooks(&vm_type),
            substate_backups: Vec::new(),
            storage_original_values: BTreeMap::new(),
            tracer,
            debug_mode: DebugMode::disabled(),
            stack_pool: Vec::new(),
            vm_type,
            current_call_frame: CallFrame::new(
                env.origin,
                callee,
                Address::default(), // Will be assigned at the end of prepare_execution
                Bytes::new(),       // Will be assigned at the end of prepare_execution
                tx.value(),
                tx.data().clone(),
                false,
                env.gas_limit,
                0,
                true,
                is_create,
                0,
                0,
                Stack::default(),
                Memory::default(),
            ),
            env,
        };

        let call_type = if is_create {
            CallType::CREATE
        } else {
            CallType::CALL
        };
        vm.tracer.enter(
            call_type,
            vm.env.origin,
            callee,
            vm.tx.value(),
            vm.env.gas_limit,
            vm.tx.data(),
        );

        #[cfg(feature = "debug")]
        {
            // Enable debug mode for printing in Solidity contracts.
            vm.debug_mode.enabled = true;
        }

        Ok(vm)
    }

    fn add_hook(&mut self, hook: impl Hook + 'static) {
        self.hooks.push(Rc::new(RefCell::new(hook)));
    }

    /// Executes a whole external transaction. Performing validations at the beginning.
    pub fn execute(&mut self) -> Result<ExecutionReport, VMError> {
        if let Err(e) = self.prepare_execution() {
            // Restore cache to state previous to this Tx execution because this Tx is invalid.
            self.restore_cache_state()?;
            return Err(e);
        }

        // Clear callframe backup so that changes made in prepare_execution are written in stone.
        // We want to apply these changes even if the Tx reverts. E.g. Incrementing sender nonce
        self.current_call_frame.call_frame_backup.clear();

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
        if precompiles::is_precompile(&self.current_call_frame.to, self.env.config.fork) {
            let call_frame = &mut self.current_call_frame;

            return Self::execute_precompile(
                call_frame.code_address,
                &call_frame.calldata,
                call_frame.gas_limit,
                &mut call_frame.gas_remaining,
                self.env.config.fork,
            );
        }

        loop {
            let opcode = self.current_call_frame.next_opcode();

            // Call the opcode, using the opcode function lookup table.
            // Indexing will not panic as all the opcode values fit within the table.
            #[allow(clippy::indexing_slicing, clippy::as_conversions)]
            let op_result = VM::OPCODE_TABLE[opcode as usize].call(self);

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
    pub fn execute_precompile(
        code_address: H160,
        calldata: &Bytes,
        gas_limit: u64,
        gas_remaining: &mut u64,
        fork: Fork,
    ) -> Result<ContextResult, VMError> {
        let execute_precompile = precompiles::execute_precompile;

        Self::handle_precompile_result(
            execute_precompile(code_address, calldata, gas_remaining, fork),
            gas_limit,
            *gas_remaining,
        )
    }

    /// True if external transaction is a contract creation
    pub fn is_create(&self) -> Result<bool, InternalError> {
        Ok(self.current_call_frame.is_create)
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

impl Substate {
    /// Initializes the VM substate, mainly adding addresses to the "accessed_addresses" field and the same with storage slots
    pub fn initialize(env: &Environment, tx: &Transaction) -> Result<Substate, VMError> {
        // Add sender and recipient to accessed accounts [https://www.evm.codes/about#access_list]
        let mut initial_accessed_addresses = HashSet::new();
        let mut initial_accessed_storage_slots: BTreeMap<Address, BTreeSet<H256>> = BTreeMap::new();

        // Add Tx sender to accessed accounts
        initial_accessed_addresses.insert(env.origin);

        // [EIP-3651] - Add coinbase to accessed accounts after Shanghai
        if env.config.fork >= Fork::Shanghai {
            initial_accessed_addresses.insert(env.coinbase);
        }

        // Add precompiled contracts addresses to accessed accounts.
        let max_precompile_address = match env.config.fork {
            spec if spec >= Fork::Prague => SIZE_PRECOMPILES_PRAGUE,
            spec if spec >= Fork::Cancun => SIZE_PRECOMPILES_CANCUN,
            spec if spec < Fork::Cancun => SIZE_PRECOMPILES_PRE_CANCUN,
            _ => return Err(InternalError::InvalidFork.into()),
        };
        for i in 1..=max_precompile_address {
            initial_accessed_addresses.insert(Address::from_low_u64_be(i));
        }

        // Add access lists contents to accessed accounts and accessed storage slots.
        for (address, keys) in tx.access_list().clone() {
            initial_accessed_addresses.insert(address);
            let mut warm_slots = BTreeSet::new();
            for slot in keys {
                warm_slots.insert(slot);
            }
            initial_accessed_storage_slots.insert(address, warm_slots);
        }

        let substate = Substate {
            selfdestruct_set: HashSet::new(),
            accessed_addresses: initial_accessed_addresses,
            accessed_storage_slots: initial_accessed_storage_slots,
            created_accounts: HashSet::new(),
            refunded_gas: 0,
            transient_storage: HashMap::new(),
            logs: Vec::new(),
        };

        Ok(substate)
    }
}
