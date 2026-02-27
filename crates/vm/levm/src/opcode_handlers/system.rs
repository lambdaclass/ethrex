//! # System operations
//!
//! Includes the following opcodes:
//!   - `CALL`
//!   - `CALLCODE`
//!   - `DELEGATECALL`
//!   - `STATICCALL`
//!   - `RETURN`
//!   - `CREATE`
//!   - `CREATE2`
//!   - `SELFDESTRUCT`
//!   - `REVERT`

use crate::{
    call_frame::CallFrame,
    constants::{FAIL, INIT_CODE_MAX_SIZE, SUCCESS},
    errors::{ContextResult, ExceptionalHalt, InternalError, OpcodeResult, TxResult, VMError},
    gas_cost,
    memory::{self, calculate_memory_size},
    opcode_handlers::OpcodeHandler,
    precompiles,
    utils::{
        address_to_word, create_eth_transfer_log, create_selfdestruct_log, word_to_address, *,
    },
    vm::VM,
};
use bytes::Bytes;
use ethrex_common::{Address, H256, U256, evm::calculate_create_address, types::Fork};
use ethrex_common::{tracing::CallType, types::Code};

pub struct OpCallHandler;
impl OpcodeHandler for OpCallHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [
            gas,
            callee,
            value,
            args_offset,
            args_len,
            return_offset,
            return_len,
        ] = *vm.current_call_frame.stack.pop()?;
        let callee = word_to_address(callee);
        let (args_len, args_offset) = size_offset_to_usize(args_len, args_offset)?;
        let (return_len, return_offset) = size_offset_to_usize(return_len, return_offset)?;

        // Validations.
        if vm.current_call_frame.is_static && !value.is_zero() {
            return Err(ExceptionalHalt::OpcodeNotAllowedInStaticContext.into());
        }

        // Check EIP-7702 delegation (gas is NOT charged yet, deferred to after BAL recording).
        let (is_delegation_7702, eip7702_gas_consumed, code_address, bytecode) =
            eip7702_get_code(vm.db, &mut vm.substate, callee)?;

        // Process gas usage.
        let (new_memory_size, address_is_empty, address_was_cold) =
            vm.get_call_gas_params(args_offset, args_len, return_offset, return_len, callee)?;

        // Record addresses for BAL per EIP-7928.
        // gas_remaining has NOT been reduced by eip7702_gas_consumed yet,
        // matching the EELS reference where BAL recording sees pre-eip7702 gas.
        let value_cost = if !value.is_zero() {
            gas_cost::CALL_POSITIVE_VALUE
        } else {
            0
        };
        let create_cost = if address_is_empty && !value.is_zero() {
            gas_cost::CALL_TO_EMPTY_ACCOUNT
        } else {
            0
        };
        vm.record_bal_call_touch(
            callee,
            code_address,
            is_delegation_7702,
            eip7702_gas_consumed,
            new_memory_size,
            vm.current_call_frame.memory.len(),
            address_was_cold,
            value_cost,
            create_cost,
        );

        // Compute gas_left after eip7702 consumption (without modifying gas_remaining yet).
        #[expect(clippy::as_conversions, reason = "safe")]
        let gas_left = (vm.current_call_frame.gas_remaining as u64)
            .checked_sub(eip7702_gas_consumed)
            .ok_or(ExceptionalHalt::OutOfGas)?;
        let (gas_cost, gas_limit) = gas_cost::call(
            new_memory_size,
            vm.current_call_frame.memory.len(),
            address_was_cold,
            address_is_empty,
            value,
            gas,
            gas_left,
        )?;
        vm.current_call_frame.increase_consumed_gas(
            gas_cost
                .checked_add(eip7702_gas_consumed)
                .ok_or(ExceptionalHalt::OutOfGas)?,
        )?;

        // Resize memory: this is necessary for multiple reasons:
        //   - Make sure the memory is expanded.
        //   - When there is return data, preallocate it because it won't be possible while the next
        //     call frame is active.
        vm.current_call_frame.memory.resize(new_memory_size)?;

        // Trace CALL operation.
        let data = vm.get_calldata(args_offset, args_len)?;
        vm.tracer.enter(
            CallType::CALL,
            vm.current_call_frame.to,
            callee,
            value,
            gas_limit,
            &data,
        );

        // Generic call.
        vm.generic_call(
            gas_limit,
            value,
            vm.current_call_frame.to,
            callee,
            code_address,
            true,
            vm.current_call_frame.is_static,
            data,
            return_offset,
            return_len,
            bytecode,
            is_delegation_7702,
        )
    }
}

pub struct OpCallCodeHandler;
impl OpcodeHandler for OpCallCodeHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [
            gas,
            address,
            value,
            args_offset,
            args_len,
            return_offset,
            return_len,
        ] = *vm.current_call_frame.stack.pop()?;
        let address = word_to_address(address);
        let (args_len, args_offset) = size_offset_to_usize(args_len, args_offset)?;
        let (return_len, return_offset) = size_offset_to_usize(return_len, return_offset)?;

        // Check EIP-7702 delegation (gas is NOT charged yet, deferred to after BAL recording).
        let (is_delegation_7702, eip7702_gas_consumed, code_address, bytecode) =
            eip7702_get_code(vm.db, &mut vm.substate, address)?;

        // Process gas usage.
        let (new_memory_size, _, address_was_cold) =
            vm.get_call_gas_params(args_offset, args_len, return_offset, return_len, address)?;

        // Record addresses for BAL per EIP-7928.
        let value_cost = if !value.is_zero() {
            gas_cost::CALLCODE_POSITIVE_VALUE
        } else {
            0
        };
        vm.record_bal_call_touch(
            address,
            code_address,
            is_delegation_7702,
            eip7702_gas_consumed,
            new_memory_size,
            vm.current_call_frame.memory.len(),
            address_was_cold,
            value_cost,
            0,
        );

        #[expect(clippy::as_conversions, reason = "safe")]
        let gas_left = (vm.current_call_frame.gas_remaining as u64)
            .checked_sub(eip7702_gas_consumed)
            .ok_or(ExceptionalHalt::OutOfGas)?;
        let (gas_cost, gas_limit) = gas_cost::callcode(
            new_memory_size,
            vm.current_call_frame.memory.len(),
            address_was_cold,
            value,
            gas,
            gas_left,
        )?;
        vm.current_call_frame.increase_consumed_gas(
            gas_cost
                .checked_add(eip7702_gas_consumed)
                .ok_or(ExceptionalHalt::OutOfGas)?,
        )?;

        // Resize memory: this is necessary for multiple reasons:
        //   - Make sure the memory is expanded.
        //   - When there is return data, preallocate it because it won't be possible while the next
        //     call frame is active.
        vm.current_call_frame.memory.resize(new_memory_size)?;

        // Trace CALL operation.
        let data = vm.get_calldata(args_offset, args_len)?;
        vm.tracer.enter(
            CallType::CALLCODE,
            vm.current_call_frame.to,
            vm.current_call_frame.to,
            value,
            gas_limit,
            &data,
        );

        // Generic call.
        vm.generic_call(
            gas_limit,
            value,
            vm.current_call_frame.to,
            vm.current_call_frame.to,
            code_address,
            true,
            vm.current_call_frame.is_static,
            data,
            return_offset,
            return_len,
            bytecode,
            is_delegation_7702,
        )
    }
}

pub struct OpDelegateCallHandler;
impl OpcodeHandler for OpDelegateCallHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [
            gas,
            address,
            args_offset,
            args_len,
            return_offset,
            return_len,
        ] = *vm.current_call_frame.stack.pop()?;
        let address = word_to_address(address);
        let (args_len, args_offset) = size_offset_to_usize(args_len, args_offset)?;
        let (return_len, return_offset) = size_offset_to_usize(return_len, return_offset)?;

        // Check EIP-7702 delegation (gas is NOT charged yet, deferred to after BAL recording).
        let (is_delegation_7702, eip7702_gas_consumed, code_address, bytecode) =
            eip7702_get_code(vm.db, &mut vm.substate, address)?;

        // Process gas usage.
        let (new_memory_size, _, address_was_cold) =
            vm.get_call_gas_params(args_offset, args_len, return_offset, return_len, address)?;

        // Record addresses for BAL per EIP-7928.
        vm.record_bal_call_touch(
            address,
            code_address,
            is_delegation_7702,
            eip7702_gas_consumed,
            new_memory_size,
            vm.current_call_frame.memory.len(),
            address_was_cold,
            0,
            0,
        );

        #[expect(clippy::as_conversions, reason = "safe")]
        let gas_left = (vm.current_call_frame.gas_remaining as u64)
            .checked_sub(eip7702_gas_consumed)
            .ok_or(ExceptionalHalt::OutOfGas)?;
        let (gas_cost, gas_limit) = gas_cost::delegatecall(
            new_memory_size,
            vm.current_call_frame.memory.len(),
            address_was_cold,
            gas,
            gas_left,
        )?;
        vm.current_call_frame.increase_consumed_gas(
            gas_cost
                .checked_add(eip7702_gas_consumed)
                .ok_or(ExceptionalHalt::OutOfGas)?,
        )?;

        // Resize memory: this is necessary for multiple reasons:
        //   - Make sure the memory is expanded.
        //   - When there is return data, preallocate it because it won't be possible while the next
        //     call frame is active.
        vm.current_call_frame.memory.resize(new_memory_size)?;

        // Trace CALL operation.
        let data = vm.get_calldata(args_offset, args_len)?;
        vm.tracer.enter(
            CallType::DELEGATECALL,
            vm.current_call_frame.msg_sender,
            vm.current_call_frame.to,
            vm.current_call_frame.msg_value,
            gas_limit,
            &data,
        );

        // Generic call.
        vm.generic_call(
            gas_limit,
            vm.current_call_frame.msg_value,
            vm.current_call_frame.msg_sender,
            vm.current_call_frame.to,
            code_address,
            false,
            vm.current_call_frame.is_static,
            data,
            return_offset,
            return_len,
            bytecode,
            is_delegation_7702,
        )
    }
}

pub struct OpStaticCallHandler;
impl OpcodeHandler for OpStaticCallHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [
            gas,
            address,
            args_offset,
            args_len,
            return_offset,
            return_len,
        ] = *vm.current_call_frame.stack.pop()?;
        let address = word_to_address(address);
        let (args_len, args_offset) = size_offset_to_usize(args_len, args_offset)?;
        let (return_len, return_offset) = size_offset_to_usize(return_len, return_offset)?;

        // Check EIP-7702 delegation (gas is NOT charged yet, deferred to after BAL recording).
        let (is_delegation_7702, eip7702_gas_consumed, code_address, bytecode) =
            eip7702_get_code(vm.db, &mut vm.substate, address)?;

        // Process gas usage.
        let (new_memory_size, _, address_was_cold) =
            vm.get_call_gas_params(args_offset, args_len, return_offset, return_len, address)?;

        // Record addresses for BAL per EIP-7928.
        vm.record_bal_call_touch(
            address,
            code_address,
            is_delegation_7702,
            eip7702_gas_consumed,
            new_memory_size,
            vm.current_call_frame.memory.len(),
            address_was_cold,
            0,
            0,
        );

        #[expect(clippy::as_conversions, reason = "safe")]
        let gas_left = (vm.current_call_frame.gas_remaining as u64)
            .checked_sub(eip7702_gas_consumed)
            .ok_or(ExceptionalHalt::OutOfGas)?;
        let (gas_cost, gas_limit) = gas_cost::staticcall(
            new_memory_size,
            vm.current_call_frame.memory.len(),
            address_was_cold,
            gas,
            gas_left,
        )?;
        vm.current_call_frame.increase_consumed_gas(
            gas_cost
                .checked_add(eip7702_gas_consumed)
                .ok_or(ExceptionalHalt::OutOfGas)?,
        )?;

        // Resize memory: this is necessary for multiple reasons:
        //   - Make sure the memory is expanded.
        //   - When there is return data, preallocate it because it won't be possible while the next
        //     call frame is active.
        vm.current_call_frame.memory.resize(new_memory_size)?;

        // Trace CALL operation.
        let data = vm.get_calldata(args_offset, args_len)?;
        vm.tracer.enter(
            CallType::STATICCALL,
            vm.current_call_frame.to,
            address,
            U256::zero(),
            gas_limit,
            &data,
        );

        // Generic call.
        vm.generic_call(
            gas_limit,
            U256::zero(),
            vm.current_call_frame.to,
            address,
            address,
            true,
            true,
            data,
            return_offset,
            return_len,
            bytecode,
            is_delegation_7702,
        )
    }
}

pub struct OpReturnHandler;
impl OpcodeHandler for OpReturnHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [offset, len] = *vm.current_call_frame.stack.pop()?;
        let (len, offset) = size_offset_to_usize(len, offset)?;

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::exit_opcode(
                calculate_memory_size(offset, len)?,
                vm.current_call_frame.memory.len(),
            )?)?;

        if len != 0 {
            vm.current_call_frame.output = vm.current_call_frame.memory.load_range(offset, len)?;
        }

        Ok(OpcodeResult::Halt)
    }
}

pub struct OpCreateHandler;
impl OpcodeHandler for OpCreateHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [value_in_wei, code_offset, code_len] = *vm.current_call_frame.stack.pop()?;
        let (code_len, code_offset) = size_offset_to_usize(code_len, code_offset)?;

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::create(
                calculate_memory_size(code_offset, code_len)?,
                vm.current_call_frame.memory.len(),
                code_len,
                vm.env.config.fork,
            )?)?;

        vm.generic_create(value_in_wei, code_offset, code_len, None)
    }
}

pub struct OpCreate2Handler;
impl OpcodeHandler for OpCreate2Handler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [value_in_wei, code_offset, code_len, salt] = *vm.current_call_frame.stack.pop()?;
        let (code_len, code_offset) = size_offset_to_usize(code_len, code_offset)?;

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::create_2(
                calculate_memory_size(code_offset, code_len)?,
                vm.current_call_frame.memory.len(),
                code_len,
                vm.env.config.fork,
            )?)?;

        vm.generic_create(value_in_wei, code_offset, code_len, Some(salt))
    }
}

pub struct OpSelfDestructHandler;
impl OpcodeHandler for OpSelfDestructHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        if vm.current_call_frame.is_static {
            return Err(ExceptionalHalt::OpcodeNotAllowedInStaticContext.into());
        }

        let beneficiary = word_to_address(vm.current_call_frame.stack.pop1()?);
        let to = vm.current_call_frame.to;

        let target_account_is_cold = vm.substate.add_accessed_address(beneficiary);
        let target_account_is_empty = vm.db.get_account(beneficiary)?.is_empty();
        let balance = vm.db.get_account(to)?.info.balance;

        // EIP-7928 (Amsterdam): Two-phase gas check for SELFDESTRUCT.
        // First check base cost (SELFDESTRUCT + cold access) before state access,
        // then record BAL tracking, then charge the full cost including NEW_ACCOUNT.
        // This ensures the beneficiary is recorded in BAL even when the full
        // selfdestruct cost (with NEW_ACCOUNT) would cause OOG.
        if vm.env.config.fork >= Fork::Amsterdam {
            let base_cost = gas_cost::selfdestruct_base(target_account_is_cold)?;
            // Phase 1: Check base cost is available (without charging)
            #[expect(clippy::as_conversions, reason = "base_cost fits in i64")]
            if vm.current_call_frame.gas_remaining < (base_cost as i64) {
                return Err(ExceptionalHalt::OutOfGas.into());
            }

            // State access: record BAL tracking between the two gas phases
            let accessed_slots = vm.substate.get_accessed_storage_slots(&to);
            if let Some(recorder) = vm.db.bal_recorder.as_mut() {
                recorder.record_touched_address(beneficiary);
                recorder.record_touched_address(to);
                if balance > U256::zero() {
                    recorder.set_initial_balance(to, balance);
                }
                for key in &accessed_slots {
                    let slot = U256::from_big_endian(key.as_bytes());
                    recorder.record_storage_read(to, slot);
                }
            }

            // Phase 2: Charge the full cost (base + NEW_ACCOUNT if applicable)
            vm.current_call_frame
                .increase_consumed_gas(gas_cost::selfdestruct(
                    target_account_is_cold,
                    target_account_is_empty,
                    balance,
                )?)?;
        } else {
            vm.current_call_frame
                .increase_consumed_gas(gas_cost::selfdestruct(
                    target_account_is_cold,
                    target_account_is_empty,
                    balance,
                )?)?;

            // Record beneficiary and destroyed account for BAL per EIP-7928
            let accessed_slots = vm.substate.get_accessed_storage_slots(&to);
            if let Some(recorder) = vm.db.bal_recorder.as_mut() {
                recorder.record_touched_address(beneficiary);
                recorder.record_touched_address(to);
                if balance > U256::zero() {
                    recorder.set_initial_balance(to, balance);
                }
                for key in &accessed_slots {
                    let slot = U256::from_big_endian(key.as_bytes());
                    recorder.record_storage_read(to, slot);
                }
            }
        }

        // [EIP-6780] - SELFDESTRUCT only in same transaction from CANCUN
        if vm.env.config.fork >= Fork::Cancun {
            vm.transfer(to, beneficiary, balance)?;

            // Selfdestruct is executed in the same transaction as the contract was created
            if vm.substate.is_account_created(&to) {
                // If target is the same as the contract calling, Ether will be burnt.
                vm.get_account_mut(to)?.info.balance = U256::zero();

                // Record balance change to zero for destroyed account in BAL
                if let Some(recorder) = vm.db.bal_recorder.as_mut() {
                    recorder.record_balance_change(to, U256::zero());
                }

                vm.substate.add_selfdestruct(to);
            }

            // EIP-7708: Emit appropriate log for ETH movement
            if vm.env.config.fork >= Fork::Amsterdam && !balance.is_zero() {
                if to != beneficiary {
                    let log = create_eth_transfer_log(to, beneficiary, balance);
                    vm.substate.add_log(log);
                } else if vm.substate.is_account_created(&to) {
                    // Selfdestruct-to-self: only emit log when created in same tx (burns ETH)
                    // Pre-existing contracts selfdestructing to self emit NO log
                    let log = create_selfdestruct_log(to, balance);
                    vm.substate.add_log(log);
                }
            }
        } else {
            vm.increase_account_balance(beneficiary, balance)?;
            vm.get_account_mut(to)?.info.balance = U256::zero();

            // Record balance change to zero for destroyed account in BAL
            if let Some(recorder) = vm.db.bal_recorder.as_mut() {
                recorder.record_balance_change(to, U256::zero());
            }

            vm.substate.add_selfdestruct(to);

            // EIP-7708: Emit appropriate log for ETH movement
            if vm.env.config.fork >= Fork::Amsterdam && !balance.is_zero() {
                let log = if to != beneficiary {
                    create_eth_transfer_log(to, beneficiary, balance)
                } else {
                    create_selfdestruct_log(to, balance)
                };
                vm.substate.add_log(log);
            }
        }

        vm.tracer.enter(
            CallType::SELFDESTRUCT,
            vm.current_call_frame.to,
            beneficiary,
            balance,
            0,
            &Default::default(),
        );
        vm.tracer.exit_early(0, None)?;

        Ok(OpcodeResult::Halt)
    }
}

pub struct OpRevertHandler;
impl OpcodeHandler for OpRevertHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [offset, len] = *vm.current_call_frame.stack.pop()?;
        let (len, offset) = size_offset_to_usize(len, offset)?;

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::exit_opcode(
                calculate_memory_size(offset, len)?,
                vm.current_call_frame.memory.len(),
            )?)?;

        if len != 0 {
            vm.current_call_frame.output = vm.current_call_frame.memory.load_range(offset, len)?;
        }

        Err(VMError::RevertOpcode)
    }
}

impl<'a> VM<'a> {
    /// Common behavior for CREATE and CREATE2 opcodes
    pub fn generic_create(
        &mut self,
        value: U256,
        code_offset_in_memory: usize,
        code_size_in_memory: usize,
        salt: Option<U256>,
    ) -> Result<OpcodeResult, VMError> {
        // Validations that can cause out of gas.
        // 1. [EIP-3860] - Cant exceed init code max size
        if code_size_in_memory > INIT_CODE_MAX_SIZE && self.env.config.fork >= Fork::Shanghai {
            return Err(ExceptionalHalt::OutOfGas.into());
        }

        let current_call_frame = &mut self.current_call_frame;
        // 2. CREATE can't be called in a static context
        if current_call_frame.is_static {
            return Err(ExceptionalHalt::OpcodeNotAllowedInStaticContext.into());
        }

        // Clear callframe subreturn data
        current_call_frame.sub_return_data = Bytes::new();

        // Reserve gas for subcall
        let gas_limit = gas_cost::max_message_call_gas(current_call_frame)?;
        current_call_frame.increase_consumed_gas(gas_limit)?;

        // Load code from memory
        let code = self
            .current_call_frame
            .memory
            .load_range(code_offset_in_memory, code_size_in_memory)?;

        // Get account info of deployer
        let deployer = self.current_call_frame.to;
        let (deployer_balance, deployer_nonce) = {
            let deployer_account = self.db.get_account(deployer)?;
            (deployer_account.info.balance, deployer_account.info.nonce)
        };

        // Calculate create address
        let new_address = match salt {
            Some(salt) => calculate_create2_address(deployer, &code, salt)?,
            None => calculate_create_address(deployer, deployer_nonce),
        };

        // Log CREATE in tracer
        let call_type = match salt {
            Some(_) => CallType::CREATE2,
            None => CallType::CREATE,
        };
        self.tracer
            .enter(call_type, deployer, new_address, value, gas_limit, &code);

        let new_depth = self
            .current_call_frame
            .depth
            .checked_add(1)
            .ok_or(InternalError::Overflow)?;

        // Validations that push 0 (FAIL) to the stack and return reserved gas to deployer
        // Per reference: these checks happen BEFORE the new address is tracked for BAL.
        // 1. Sender doesn't have enough balance to send value.
        // 2. Depth limit has been reached
        // 3. Sender nonce is max.
        let checks = [
            (deployer_balance < value, "OutOfFund"),
            (new_depth > 1024, "MaxDepth"),
            (deployer_nonce == u64::MAX, "MaxNonce"),
        ];
        for (condition, reason) in checks {
            if condition {
                self.early_revert_message_call(gas_limit, reason.to_string())?;
                return Ok(OpcodeResult::Continue);
            }
        }

        // Add new contract to accessed addresses (after early checks pass, per reference)
        self.substate.add_accessed_address(new_address);

        // Record address touch for BAL (after early checks pass per EIP-7928 reference)
        if let Some(recorder) = self.db.bal_recorder.as_mut() {
            recorder.record_touched_address(new_address);
        }

        // Increment sender nonce (irreversible change)
        self.increment_account_nonce(deployer)?;

        // Deployment will fail (consuming all gas) if the contract already exists.
        let new_account = self.get_account_mut(new_address)?;
        if new_account.create_would_collide() {
            self.current_call_frame.stack.push(FAIL)?;
            self.tracer
                .exit_early(gas_limit, Some("CreateAccExists".to_string()))?;
            return Ok(OpcodeResult::Continue);
        }

        // Create BAL checkpoint before entering create call for potential revert per EIP-7928
        let bal_checkpoint = self.db.bal_recorder.as_ref().map(|r| r.checkpoint());

        let mut stack = self.stack_pool.pop().unwrap_or_default();
        stack.clear();

        let next_memory = self.current_call_frame.memory.next_memory();

        let mut new_call_frame = CallFrame::new(
            deployer,
            new_address,
            new_address,
            // SAFETY: init code hash is never used
            Code::from_bytecode_unchecked(code, H256::zero()),
            value,
            Bytes::new(),
            false,
            gas_limit,
            new_depth,
            true,
            true,
            0,
            0,
            stack,
            next_memory,
        );
        // Store BAL checkpoint in the call frame's backup for restoration on revert
        new_call_frame.call_frame_backup.bal_checkpoint = bal_checkpoint;

        self.add_callframe(new_call_frame);

        // Changes that revert in case the Create fails.
        self.increment_account_nonce(new_address)?; // 0 -> 1
        self.transfer(deployer, new_address, value)?;

        self.substate.push_backup();
        self.substate.add_created_account(new_address); // Mostly for SELFDESTRUCT during initcode.

        // EIP-7708: Emit transfer log for nonzero-value CREATE/CREATE2
        // Must be after push_backup() so the log reverts if the child context reverts
        if self.env.config.fork >= Fork::Amsterdam && !value.is_zero() {
            let log = create_eth_transfer_log(deployer, new_address, value);
            self.substate.add_log(log);
        }

        Ok(OpcodeResult::Continue)
    }

    /// Record BAL touched addresses for CALL-family opcodes per EIP-7928.
    /// Gated on intermediate gas checks matching the EELS reference.
    #[expect(
        clippy::too_many_arguments,
        reason = "matches EIP-7928 EELS reference parameters"
    )]
    fn record_bal_call_touch(
        &mut self,
        target: Address,
        code_address: Address,
        is_delegation_7702: bool,
        eip7702_gas_consumed: u64,
        new_memory_size: usize,
        current_memory_size: usize,
        address_was_cold: bool,
        value_cost: u64,
        create_cost: u64,
    ) {
        let Some(recorder) = self.db.bal_recorder.as_mut() else {
            return;
        };
        // Safe: expansion_cost only fails on usize→u64 overflow, which is infallible
        // (usize ≤ 64 bits). If it somehow did, u64::MAX makes the gas check fail
        // conservatively, skipping the BAL touch — a non-consensus recording path.
        let mem_cost =
            memory::expansion_cost(new_memory_size, current_memory_size).unwrap_or(u64::MAX);
        let access_cost = if address_was_cold {
            gas_cost::COLD_ADDRESS_ACCESS_COST
        } else {
            gas_cost::WARM_ADDRESS_ACCESS_COST
        };
        let basic_cost = mem_cost
            .saturating_add(access_cost)
            .saturating_add(value_cost);
        let gas_remaining = self.current_call_frame.gas_remaining;

        if gas_remaining >= i64::try_from(basic_cost).unwrap_or(i64::MAX) {
            recorder.record_touched_address(target);

            if is_delegation_7702 {
                let delegation_check = basic_cost
                    .saturating_add(create_cost)
                    .saturating_add(eip7702_gas_consumed);
                if gas_remaining >= i64::try_from(delegation_check).unwrap_or(i64::MAX) {
                    recorder.record_touched_address(code_address);
                }
            }
        }
    }

    /// This (should) be the only function where gas is used as a
    /// U256. This is because we have to use the values that are
    /// pushed to the stack.
    ///
    // Force inline, due to lot of arguments, inlining must be forced, and it is actually beneficial
    // because passing so much data is costly. Verified with samply.
    #[expect(
        clippy::too_many_arguments,
        reason = "inlined for performance, many args needed"
    )]
    #[inline(always)]
    pub fn generic_call(
        &mut self,
        gas_limit: u64,
        value: U256,
        msg_sender: Address,
        to: Address,
        code_address: Address,
        should_transfer_value: bool,
        is_static: bool,
        calldata: Bytes,
        ret_offset: usize,
        ret_size: usize,
        bytecode: Code,
        is_delegation_7702: bool,
    ) -> Result<OpcodeResult, VMError> {
        // Clear callframe subreturn data
        self.current_call_frame.sub_return_data.clear();

        // Validate sender has enough value
        if should_transfer_value && !value.is_zero() {
            let sender_balance = self.db.get_account(msg_sender)?.info.balance;
            if sender_balance < value {
                self.early_revert_message_call(gas_limit, "OutOfFund".to_string())?;
                return Ok(OpcodeResult::Continue);
            }
        }

        // Validate max depth has not been reached yet.
        let new_depth = self
            .current_call_frame
            .depth
            .checked_add(1)
            .ok_or(InternalError::Overflow)?;
        if new_depth > 1024 {
            self.early_revert_message_call(gas_limit, "MaxDepth".to_string())?;
            return Ok(OpcodeResult::Continue);
        }

        if precompiles::is_precompile(&code_address, self.env.config.fork, self.vm_type)
            && !is_delegation_7702
        {
            // Record precompile address touch for BAL per EIP-7928
            if let Some(recorder) = self.db.bal_recorder.as_mut() {
                recorder.record_touched_address(code_address);
            }

            let mut gas_remaining = gas_limit;
            let ctx_result = Self::execute_precompile(
                code_address,
                &calldata,
                gas_limit,
                &mut gas_remaining,
                self.env.config.fork,
                self.db.store.precompile_cache(),
            )?;

            let call_frame = &mut self.current_call_frame;

            // Return gas left from subcontext
            #[expect(clippy::as_conversions, reason = "remaining gas conversion")]
            if ctx_result.is_success() {
                call_frame.gas_remaining = (call_frame.gas_remaining as u64)
                    .checked_add(
                        gas_limit
                            .checked_sub(ctx_result.gas_used)
                            .ok_or(InternalError::Underflow)?,
                    )
                    .ok_or(InternalError::Overflow)?
                    as i64;
            }

            // Store return data of sub-context
            call_frame.memory.store_data(
                ret_offset,
                if ctx_result.output.len() >= ret_size {
                    ctx_result
                        .output
                        .get(..ret_size)
                        .ok_or(ExceptionalHalt::OutOfBounds)?
                } else {
                    &ctx_result.output
                },
            )?;
            call_frame.sub_return_data = ctx_result.output.clone();

            // What to do, depending on TxResult
            call_frame.stack.push(match &ctx_result.result {
                TxResult::Success => SUCCESS,
                TxResult::Revert(_) => FAIL,
            })?;

            // Transfer value from caller to callee.
            if should_transfer_value && ctx_result.is_success() {
                self.transfer(msg_sender, to, value)?;

                // EIP-7708: Emit transfer log for nonzero-value CALL/CALLCODE
                // Self-transfers (msg_sender == to) do NOT emit a log (includes CALLCODE)
                if self.env.config.fork >= Fork::Amsterdam && !value.is_zero() && msg_sender != to {
                    let log = create_eth_transfer_log(msg_sender, to, value);
                    self.substate.add_log(log);
                }
            }

            self.tracer.exit_context(&ctx_result, false)?;
        } else {
            // Create BAL checkpoint before entering nested call for potential revert per EIP-7928
            let bal_checkpoint = self.db.bal_recorder.as_ref().map(|r| r.checkpoint());

            let mut stack = self.stack_pool.pop().unwrap_or_default();
            stack.clear();

            let next_memory = self.current_call_frame.memory.next_memory();

            let mut new_call_frame = CallFrame::new(
                msg_sender,
                to,
                code_address,
                bytecode,
                value,
                calldata,
                is_static,
                gas_limit,
                new_depth,
                should_transfer_value,
                false,
                ret_offset,
                ret_size,
                stack,
                next_memory,
            );
            // Store BAL checkpoint in the call frame's backup for restoration on revert
            new_call_frame.call_frame_backup.bal_checkpoint = bal_checkpoint;

            self.add_callframe(new_call_frame);

            // Transfer value from caller to callee.
            if should_transfer_value {
                self.transfer(msg_sender, to, value)?;
            }

            self.substate.push_backup();

            // EIP-7708: Emit transfer log for nonzero-value CALL/CALLCODE
            // Must be after push_backup() so the log reverts if the child context reverts
            // Self-transfers (msg_sender == to) do NOT emit a log (includes CALLCODE)
            if should_transfer_value
                && self.env.config.fork >= Fork::Amsterdam
                && !value.is_zero()
                && msg_sender != to
            {
                let log = create_eth_transfer_log(msg_sender, to, value);
                self.substate.add_log(log);
            }
        }

        Ok(OpcodeResult::Continue)
    }

    /// Pop backup from stack and restore substate and cache if transaction reverted.
    pub fn handle_state_backup(&mut self, ctx_result: &ContextResult) -> Result<(), VMError> {
        if ctx_result.is_success() {
            self.substate.commit_backup();
        } else {
            self.substate.revert_backup();
            self.restore_cache_state()?;
        }

        Ok(())
    }

    /// Handles case in which callframe was initiated by another callframe (with CALL or CREATE family opcodes)
    ///
    /// Returns the pc increment.
    pub fn handle_return(&mut self, ctx_result: &ContextResult) -> Result<(), VMError> {
        self.handle_state_backup(ctx_result)?;
        let executed_call_frame = self.pop_call_frame()?;

        // Here happens the interaction between child (executed) and parent (caller) callframe.
        if executed_call_frame.is_create {
            self.handle_return_create(executed_call_frame, ctx_result)?;
        } else {
            self.handle_return_call(executed_call_frame, ctx_result)?;
        }

        Ok(())
    }

    #[expect(clippy::as_conversions, reason = "remaining gas conversion")]
    pub fn handle_return_call(
        &mut self,
        executed_call_frame: CallFrame,
        ctx_result: &ContextResult,
    ) -> Result<(), VMError> {
        let CallFrame {
            gas_limit,
            ret_offset,
            ret_size,
            memory: old_callframe_memory,
            ..
        } = executed_call_frame;

        old_callframe_memory.clean_from_base();

        let parent_call_frame = &mut self.current_call_frame;

        // Return gas left from subcontext
        let child_unused_gas = gas_limit
            .checked_sub(ctx_result.gas_used)
            .ok_or(InternalError::Underflow)?;
        parent_call_frame.gas_remaining = parent_call_frame
            .gas_remaining
            .checked_add(child_unused_gas as i64)
            .ok_or(InternalError::Overflow)?;

        // Store return data of sub-context
        parent_call_frame.memory.store_data(
            ret_offset,
            if ctx_result.output.len() >= ret_size {
                ctx_result
                    .output
                    .get(..ret_size)
                    .ok_or(ExceptionalHalt::OutOfBounds)?
            } else {
                &ctx_result.output
            },
        )?;

        parent_call_frame.sub_return_data = ctx_result.output.clone();

        // What to do, depending on TxResult
        match &ctx_result.result {
            TxResult::Success => {
                self.current_call_frame.stack.push(SUCCESS)?;
                self.merge_call_frame_backup_with_parent(&executed_call_frame.call_frame_backup)?;
            }
            TxResult::Revert(_) => {
                self.current_call_frame.stack.push(FAIL)?;
            }
        };

        self.tracer.exit_context(ctx_result, false)?;

        let mut stack = executed_call_frame.stack;
        stack.clear();
        self.stack_pool.push(stack);

        Ok(())
    }

    #[expect(clippy::as_conversions, reason = "remaining gas conversion")]
    pub fn handle_return_create(
        &mut self,
        executed_call_frame: CallFrame,
        ctx_result: &ContextResult,
    ) -> Result<(), VMError> {
        let CallFrame {
            gas_limit,
            to,
            call_frame_backup,
            memory: old_callframe_memory,
            ..
        } = executed_call_frame;

        old_callframe_memory.clean_from_base();

        let parent_call_frame = &mut self.current_call_frame;

        // Return unused gas
        let unused_gas = gas_limit
            .checked_sub(ctx_result.gas_used)
            .ok_or(InternalError::Underflow)?;
        parent_call_frame.gas_remaining = parent_call_frame
            .gas_remaining
            .checked_add(unused_gas as i64)
            .ok_or(InternalError::Overflow)?;

        // What to do, depending on TxResult
        match ctx_result.result.clone() {
            TxResult::Success => {
                parent_call_frame.stack.push(address_to_word(to))?;
                self.merge_call_frame_backup_with_parent(&call_frame_backup)?;
            }
            TxResult::Revert(err) => {
                // If revert we have to copy the return_data
                if err.is_revert_opcode() {
                    parent_call_frame.sub_return_data = ctx_result.output.clone();
                }

                parent_call_frame.stack.push(FAIL)?;
            }
        };

        self.tracer.exit_context(ctx_result, false)?;

        let mut stack = executed_call_frame.stack;
        stack.clear();
        self.stack_pool.push(stack);

        Ok(())
    }

    /// Obtains the values needed for CALL, CALLCODE, DELEGATECALL and STATICCALL opcodes to calculate total gas cost
    fn get_call_gas_params(
        &mut self,
        args_offset: usize,
        args_size: usize,
        return_data_offset: usize,
        return_data_size: usize,
        address: Address,
    ) -> Result<(usize, bool, bool), VMError> {
        // Creation of previously empty accounts and cold addresses have higher gas cost
        let address_was_cold = self.substate.add_accessed_address(address);
        let account_is_empty = self.db.get_account(address)?.is_empty();

        // Calculated here for memory expansion gas cost
        let new_memory_size_for_args = calculate_memory_size(args_offset, args_size)?;
        let new_memory_size_for_return_data =
            calculate_memory_size(return_data_offset, return_data_size)?;
        let new_memory_size = new_memory_size_for_args.max(new_memory_size_for_return_data);

        Ok((new_memory_size, account_is_empty, address_was_cold))
    }

    fn get_calldata(&mut self, offset: usize, size: usize) -> Result<Bytes, VMError> {
        self.current_call_frame.memory.load_range(offset, size)
    }

    #[expect(clippy::as_conversions, reason = "remaining gas conversion")]
    fn early_revert_message_call(&mut self, gas_limit: u64, reason: String) -> Result<(), VMError> {
        let callframe = &mut self.current_call_frame;

        // Return gas_limit to callframe.
        callframe.gas_remaining = callframe
            .gas_remaining
            .checked_add(gas_limit as i64)
            .ok_or(InternalError::Overflow)?;
        callframe.stack.push(FAIL)?; // It's the same as revert for CREATE

        self.tracer.exit_early(0, Some(reason))?;
        Ok(())
    }
}
