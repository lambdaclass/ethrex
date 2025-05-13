use crate::{
    call_frame::{CallFrame, CallFrameBackup},
    db::gen_db::GeneralizedDatabase,
    environment::Environment,
    errors::{ExecutionReport, InternalError, OpcodeResult, TxResult, VMError},
    hooks::hook::Hook,
    precompiles::{
        execute_precompile, is_precompile, SIZE_PRECOMPILES_CANCUN, SIZE_PRECOMPILES_PRAGUE,
        SIZE_PRECOMPILES_PRE_CANCUN,
    },
    utils::*,
    TransientStorage,
};
use bytes::Bytes;
use ethrex_common::{
    types::{
        tx_fields::{AccessList, AuthorizationList},
        Fork, Transaction, TxKind,
    },
    Address, H256, U256,
};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fmt::Debug,
    sync::Arc,
};

#[cfg(not(feature = "l2"))]
use crate::hooks::DefaultHook;
#[cfg(feature = "l2")]
use {crate::hooks::L2Hook, ethrex_common::types::PrivilegedL2Transaction};

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
    pub accrued_substate: Substate,
    pub db: &'a mut GeneralizedDatabase,
    pub tx_kind: TxKind,
    pub access_list: AccessList,
    pub authorization_list: Option<AuthorizationList>,
    pub hooks: Vec<Arc<dyn Hook>>,
    pub substate_backups: Vec<Substate>,
    /// Original storage values before the transaction. Used for gas calculations in SSTORE.
    pub storage_original_values: HashMap<Address, HashMap<H256, U256>>,
}

impl<'a> VM<'a> {
    pub fn new(
        env: Environment,
        db: &'a mut GeneralizedDatabase,
        tx: &Transaction,
    ) -> Result<Self, VMError> {
        // Add sender and recipient (in the case of a Call) to cache [https://www.evm.codes/about#access_list]
        let mut default_touched_accounts = HashSet::from_iter([env.origin].iter().cloned());

        // [EIP-3651] - Add coinbase to cache if the spec is SHANGHAI or higher
        if env.config.fork >= Fork::Shanghai {
            default_touched_accounts.insert(env.coinbase);
        }

        let mut default_touched_storage_slots: HashMap<Address, BTreeSet<H256>> = HashMap::new();

        // Add access lists contents to cache
        for (address, keys) in tx.access_list() {
            default_touched_accounts.insert(address);
            let mut warm_slots = BTreeSet::new();
            for slot in keys {
                warm_slots.insert(slot);
            }
            default_touched_storage_slots.insert(address, warm_slots);
        }

        // Add precompiled contracts addresses to cache.
        let max_precompile_address = match env.config.fork {
            spec if spec >= Fork::Prague => SIZE_PRECOMPILES_PRAGUE,
            spec if spec >= Fork::Cancun => SIZE_PRECOMPILES_CANCUN,
            spec if spec < Fork::Cancun => SIZE_PRECOMPILES_PRE_CANCUN,
            _ => return Err(VMError::Internal(InternalError::InvalidSpecId)),
        };
        for i in 1..=max_precompile_address {
            default_touched_accounts.insert(Address::from_low_u64_be(i));
        }

        #[cfg(not(feature = "l2"))]
        let hooks: Vec<Arc<dyn Hook>> = vec![Arc::new(DefaultHook)];
        #[cfg(feature = "l2")]
        let hooks: Vec<Arc<dyn Hook>> = {
            let recipient = if let Transaction::PrivilegedL2Transaction(PrivilegedL2Transaction {
                recipient,
                ..
            }) = tx
            {
                Some(*recipient)
            } else {
                None
            };
            vec![Arc::new(L2Hook { recipient })]
        };

        let mut substate = Substate {
            selfdestruct_set: HashSet::new(),
            touched_accounts: default_touched_accounts,
            touched_storage_slots: default_touched_storage_slots,
            created_accounts: HashSet::new(),
            refunded_gas: 0,
            transient_storage: HashMap::new(),
        };

        let bytecode;
        let destination_and_code_address;

        match tx.to() {
            TxKind::Call(address_to) => {
                substate.touched_accounts.insert(address_to);

                let (_is_delegation, _eip7702_gas_consumed, _code_address, bytes) =
                    eip7702_get_code(db, &mut substate, address_to)?;
                destination_and_code_address = address_to;
                bytecode = bytes;
            }

            TxKind::Create => {
                let sender_nonce = db.get_account(env.origin)?.info.nonce;

                // In this case, the destination address which also holds the code would be the address of the newly created contract
                destination_and_code_address = calculate_create_address(env.origin, sender_nonce)
                    .map_err(|_| {
                    VMError::Internal(InternalError::CouldNotComputeCreateAddress)
                })?;

                substate
                    .touched_accounts
                    .insert(destination_and_code_address);

                substate
                    .created_accounts
                    .insert(destination_and_code_address);

                bytecode = Bytes::new() //Bytecode will be later assigned from the calldata after passing validations;
            }
        }

        let initial_call_frame = CallFrame::new(
            env.origin,
            destination_and_code_address,
            destination_and_code_address,
            bytecode,
            tx.value(),
            tx.data().clone(),
            false,
            env.gas_limit,
            0,
            true,
            false,
            U256::zero(),
            0,
        );

        Ok(Self {
            call_frames: vec![initial_call_frame],
            env,
            accrued_substate: substate,
            db,
            tx_kind: tx.to(),
            access_list: tx.access_list(),
            authorization_list: tx.authorization_list(),
            hooks,
            substate_backups: vec![],
            storage_original_values: HashMap::new(),
        })
    }

    pub fn run_execution(&mut self) -> Result<ExecutionReport, VMError> {
        let fork = self.env.config.fork;

        if is_precompile(&self.current_call_frame()?.code_address, fork) {
            let mut current_call_frame = self.pop_callframe()?;
            let precompile_result = execute_precompile(&mut current_call_frame);
            let backup = self
                .substate_backups
                .pop()
                .ok_or(VMError::Internal(InternalError::CouldNotPopCallframe))?;
            let report =
                self.handle_precompile_result(precompile_result, backup, &mut current_call_frame)?;
            self.handle_return(&current_call_frame, &report)?;
            self.current_call_frame_mut()?.increment_pc_by(1)?;
            return Ok(report);
        }

        loop {
            let opcode = self.current_call_frame()?.next_opcode();

            let op_result = self.handle_current_opcode(opcode);

            match op_result {
                Ok(OpcodeResult::Continue { pc_increment }) => self
                    .current_call_frame_mut()?
                    .increment_pc_by(pc_increment)?,
                Ok(OpcodeResult::Halt) | Err(_) => {
                    let mut executed_call_frame = self.pop_callframe()?;

                    let report = match op_result {
                        Ok(OpcodeResult::Halt) => {
                            self.handle_opcode_result(&mut executed_call_frame)?
                        }
                        Err(error) => self.handle_opcode_error(error, &mut executed_call_frame)?,
                        _ => unreachable!(),
                    };

                    if self.handle_return(&executed_call_frame, &report)? {
                        self.current_call_frame_mut()?.increment_pc_by(1)?;
                    } else {
                        return Ok(report);
                    }
                }
            }
        }
    }

    pub fn restore_state(
        &mut self,
        backup: Substate,
        call_frame_backup: CallFrameBackup,
    ) -> Result<(), VMError> {
        self.restore_cache_state(call_frame_backup)?;
        self.accrued_substate = backup;
        Ok(())
    }

    pub fn is_create(&self) -> bool {
        matches!(self.tx_kind, TxKind::Create)
    }

    /// Executes without making changes to the cache.
    pub fn stateless_execute(&mut self) -> Result<ExecutionReport, VMError> {
        let cache_backup = self.db.cache.clone();
        let report = self.execute()?;
        // Restore the cache to its original state
        self.db.cache = cache_backup;
        Ok(report)
    }

    /// Main function for executing an external transaction
    pub fn execute(&mut self) -> Result<ExecutionReport, VMError> {
        if let Err(e) = self.prepare_execution() {
            // We need to do a cleanup of the cache so that it doesn't interfere with next transaction's execution
            self.restore_cache_state(self.current_call_frame()?.call_frame_backup.clone())?;
            return Err(e);
        }

        // Here we clear the cache backup because if prepare_execution succeeded we don't want to
        // revert the changes it made.
        // Even if the transaction reverts we want to apply these kind of changes!
        // These are: Incrementing sender nonce, transferring value to a delegate account, decreasing sender account balance
        self.current_call_frame_mut()?.call_frame_backup = CallFrameBackup {
            original_accounts_info: HashMap::new(),
            original_account_storage_slots: HashMap::new(),
        };

        // In CREATE type transactions:
        //  Add created contract to cache, reverting transaction if the address is already occupied
        if self.is_create() {
            let new_contract_address = self.current_call_frame()?.to;
            let new_account = self.get_account_mut(new_contract_address)?;

            if new_account.has_code_or_nonce() {
                return self.handle_create_non_empty_account();
            }

            self.increase_account_balance(
                new_contract_address,
                self.current_call_frame()?.msg_value,
            )?;

            // https://eips.ethereum.org/EIPS/eip-161
            self.increment_account_nonce(new_contract_address)?;
        }

        // Backup of Substate,a copy of the current substate to restore if sub-context is reverted
        let backup = self.accrued_substate.clone();

        self.substate_backups.push(backup);

        let mut report = self.run_execution()?;

        self.finalize_execution(&mut report)?;
        Ok(report)
    }

    fn handle_create_non_empty_account(&mut self) -> Result<ExecutionReport, VMError> {
        let mut report = ExecutionReport {
            result: TxResult::Revert(VMError::AddressAlreadyOccupied),
            gas_used: self.env.gas_limit,
            gas_refunded: 0,
            logs: vec![],
            output: Bytes::new(),
        };

        self.finalize_execution(&mut report)?;

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
