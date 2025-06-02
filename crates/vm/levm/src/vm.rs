#[cfg(feature = "l2")]
use crate::call_frame::CallFrameBackup;
use crate::{
    call_frame::CallFrame,
    db::gen_db::GeneralizedDatabase,
    environment::Environment,
    errors::{ExecutionReport, InternalError, OpcodeResult, TxResult, VMError},
    hooks::hook::Hook,
    opcodes::Opcode,
    precompiles::execute_precompile,
    TransientStorage,
};
use bytes::Bytes;
use derive_more::derive::Debug;
use ethrex_common::{
    types::{Transaction, TxKind},
    Address, H256, U256,
};
use serde::Serialize;
use std::{
    collections::{BTreeSet, HashMap, HashSet},
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

fn u64_to_hex<S>(x: &u64, s: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    s.serialize_str(&format!("0x{:x}", x))
}

fn u256_to_hex<S>(x: &U256, s: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    s.serialize_str(&format!("0x{:x}", x))
}

fn bytes_to_hex<S>(x: &Bytes, s: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    s.serialize_str(&format!("0x{:x}", x))
}

fn option_string_empty_as_str<S>(x: &Option<String>, s: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    s.serialize_str(x.as_deref().unwrap_or(""))
}

#[derive(Debug, Clone, Serialize)]
pub struct TracerCallFrame {
    #[serde(rename = "type")]
    pub call_type: Opcode,
    pub from: Address,
    pub to: Address,
    #[serde(serialize_with = "u256_to_hex")]
    pub value: U256,
    #[serde(serialize_with = "u64_to_hex")]
    pub gas: u64,
    #[serde(rename = "gasUsed", serialize_with = "u64_to_hex")]
    pub gas_used: u64,
    #[serde(serialize_with = "bytes_to_hex")]
    pub input: Bytes,
    #[serde(serialize_with = "bytes_to_hex")]
    pub output: Bytes,
    #[serde(serialize_with = "option_string_empty_as_str")]
    pub error: Option<String>,
    #[serde(rename = "revertReason", serialize_with = "option_string_empty_as_str")]
    pub revert_reason: Option<String>,
    pub calls: Vec<TracerCallFrame>,
}

impl TracerCallFrame {
    pub fn new(
        call_type: Opcode,
        from: Address,
        to: Address,
        value: U256,
        gas: u64,
        input: Bytes,
    ) -> Self {
        Self {
            call_type,
            from,
            to,
            value,
            gas,
            gas_used: 0,
            input,
            output: Bytes::new(),
            error: None,
            revert_reason: None,
            calls: Vec::new(),
        }
    }

    pub fn process_output(
        &mut self,
        gas_used: u64,
        output: Bytes,
        error: Option<String>,
        revert_reason: Option<String>,
    ) {
        self.gas_used = gas_used;
        self.output = output;
        self.error = error;
        self.revert_reason = revert_reason;
    }
}

#[derive(Debug, Default)]
/// Geth's callTracer (https://geth.ethereum.org/docs/developers/evm-tracing/built-in-tracers)
/// Use `LevmCallTracer::disabled()` when tracing is not wanted.
pub struct LevmCallTracer {
    /// Stack for tracer callframes, at the end of execution there will be only one element.
    pub callframes: Vec<TracerCallFrame>,
    /// Trace only the top call (a.k.a. the external transaction)
    pub only_top_call: bool,
    /// Trace logs
    pub with_log: bool,
    /// If active is set to false it won't trace.
    pub active: bool,
}

impl LevmCallTracer {
    pub fn new(only_top_call: bool, with_log: bool) -> Self {
        LevmCallTracer {
            callframes: vec![],
            only_top_call,
            with_log,
            active: true,
        }
    }

    /// This is to keep LEVM's code clean, like `self.tracer.enter(...)`,
    /// instead of something more complex or uglier when we don't want to trace.
    /// (For now that we only implement one tracer is the most convenient solution)
    pub fn disabled() -> Self {
        LevmCallTracer {
            active: false,
            ..Default::default()
        }
    }

    /// Starts trace call.
    pub fn enter(
        &mut self,
        call_type: Opcode,
        from: Address,
        to: Address,
        value: U256,
        gas: u64,
        input: Bytes,
    ) {
        if !self.active {
            return;
        }
        if self.only_top_call && !self.callframes.is_empty() {
            // Only create callframe if it's the first one to be created.
            return;
        }
        let callframe = TracerCallFrame::new(call_type, from, to, value, gas, input);
        self.callframes.push(callframe);
    }

    /// Exits trace call.
    fn exit(
        &mut self,
        gas_used: u64,
        output: Bytes,
        error: Option<String>,
        revert_reason: Option<String>,
    ) -> Result<(), InternalError> {
        let mut executed_callframe = self
            .callframes
            .pop()
            .ok_or(InternalError::CouldNotPopCallframe)?;

        executed_callframe.process_output(gas_used, output, error, revert_reason);

        // Append executed callframe to parent callframe if appropriate.
        if let Some(parent_callframe) = self.callframes.last_mut() {
            parent_callframe.calls.push(executed_callframe);
        } else {
            self.callframes.push(executed_callframe);
        };
        Ok(())
    }

    /// Exits trace call using the ExecutionReport.
    pub fn exit_report(
        &mut self,
        report: &ExecutionReport,
        is_top_call: bool,
    ) -> Result<(), InternalError> {
        if !self.active {
            return Ok(());
        }
        if self.only_top_call && !is_top_call {
            // We just want to register top call
            return Ok(());
        }
        let (gas_used, output) = (report.gas_used, report.output.clone());

        let (error, revert_reason) = if let TxResult::Revert(ref err) = report.result {
            let reason = String::from_utf8(report.output.to_vec()).ok();
            (Some(err.to_string()), reason)
        } else {
            (None, None)
        };

        self.exit(gas_used, output, error, revert_reason)
    }

    /// Exits trace call when CALL or CREATE opcodes return early or in case SELFDESTRUCT is called.
    pub fn exit_early(
        &mut self,
        gas_used: u64,
        error: Option<String>,
    ) -> Result<(), InternalError> {
        if !self.active || self.only_top_call {
            return Ok(());
        }
        self.exit(gas_used, Bytes::new(), error, None)
    }
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
    pub tracer: LevmCallTracer,
}

impl<'a> VM<'a> {
    pub fn new(
        env: Environment,
        db: &'a mut GeneralizedDatabase,
        tx: &Transaction,
        tracer: LevmCallTracer,
    ) -> Self {
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
            tracer,
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

        let call_type = if self.is_create() {
            Opcode::CREATE
        } else {
            Opcode::CALL
        };
        self.tracer.enter(
            call_type,
            self.env.origin,
            callee,
            self.tx.value(),
            self.env.gas_limit,
            self.tx.data().clone(),
        );

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
    pub fn execute_precompile(&mut self) -> Result<ExecutionReport, VMError> {
        let callframe = self.current_call_frame_mut()?;

        let precompile_result = {
            execute_precompile(
                callframe.code_address,
                &callframe.calldata,
                &mut callframe.gas_used,
                callframe.gas_limit,
            )
        };

        let report = self.handle_precompile_result(precompile_result)?;

        Ok(report)
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

        self.tracer.exit_report(report, true)?;

        //TODO: Remove this
        // let a = serde_json::to_string_pretty(&self.tracer.callframes.pop().unwrap()).unwrap();
        // println!("{a}");

        Ok(())
    }
}
