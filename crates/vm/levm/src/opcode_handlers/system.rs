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
    memory::calculate_memory_size,
    opcode_handlers::OpcodeHandler,
    precompiles,
    utils::{address_to_word, word_to_address, *},
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

        // Check and subtract EIP-7702.
        // Note: Do not reorder the gas increase after the `get_call_gas_params()`.
        let (is_delegation_7702, eip7702_gas_consumed, code_address, bytecode) =
            eip7702_get_code(vm.db, &mut vm.substate, callee)?;
        vm.current_call_frame
            .increase_consumed_gas(eip7702_gas_consumed)?;

        // Process gas usage.
        let (new_memory_size, address_is_empty, address_was_cold) =
            vm.get_call_gas_params(args_offset, args_len, return_offset, return_len, callee)?;
        let (gas_cost, gas_limit) = gas_cost::call(
            new_memory_size,
            vm.current_call_frame.memory.len(),
            address_was_cold,
            address_is_empty,
            value,
            gas,
            vm.current_call_frame.gas_remaining as u64,
        )?;
        vm.current_call_frame.increase_consumed_gas(gas_cost)?;

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

        // Check and subtract EIP-7702.
        // Note: Do not reorder the gas increase after the `get_call_gas_params()`.
        let (is_delegation_7702, eip7702_gas_consumed, code_address, bytecode) =
            eip7702_get_code(vm.db, &mut vm.substate, address)?;
        vm.current_call_frame
            .increase_consumed_gas(eip7702_gas_consumed)?;

        // Process gas usage.
        let (new_memory_size, _, address_was_cold) =
            vm.get_call_gas_params(args_offset, args_len, return_offset, return_len, address)?;
        let (gas_cost, gas_limit) = gas_cost::callcode(
            new_memory_size,
            vm.current_call_frame.memory.len(),
            address_was_cold,
            value,
            gas,
            vm.current_call_frame.gas_remaining as u64,
        )?;
        vm.current_call_frame.increase_consumed_gas(gas_cost)?;

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

        // Check and subtract EIP-7702.
        // Note: Do not reorder the gas increase after the `get_call_gas_params()`.
        let (is_delegation_7702, eip7702_gas_consumed, code_address, bytecode) =
            eip7702_get_code(vm.db, &mut vm.substate, address)?;
        vm.current_call_frame
            .increase_consumed_gas(eip7702_gas_consumed)?;

        // Process gas usage.
        let (new_memory_size, _, address_was_cold) =
            vm.get_call_gas_params(args_offset, args_len, return_offset, return_len, address)?;
        let (gas_cost, gas_limit) = gas_cost::delegatecall(
            new_memory_size,
            vm.current_call_frame.memory.len(),
            address_was_cold,
            gas,
            vm.current_call_frame.gas_remaining as u64,
        )?;
        vm.current_call_frame.increase_consumed_gas(gas_cost)?;

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

        // Check and subtract EIP-7702.
        // Note: Do not reorder the gas increase after the `get_call_gas_params()`.
        let (is_delegation_7702, eip7702_gas_consumed, _, bytecode) =
            eip7702_get_code(vm.db, &mut vm.substate, address)?;
        vm.current_call_frame
            .increase_consumed_gas(eip7702_gas_consumed)?;

        // Process gas usage.
        let (new_memory_size, _, address_was_cold) =
            vm.get_call_gas_params(args_offset, args_len, return_offset, return_len, address)?;
        let (gas_cost, gas_limit) = gas_cost::staticcall(
            new_memory_size,
            vm.current_call_frame.memory.len(),
            address_was_cold,
            gas,
            vm.current_call_frame.gas_remaining as u64,
        )?;
        vm.current_call_frame.increase_consumed_gas(gas_cost)?;

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

        let balance = vm.db.get_account(vm.current_call_frame.to)?.info.balance;
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::selfdestruct(
                vm.substate.add_accessed_address(beneficiary),
                vm.db.get_account(beneficiary)?.is_empty(),
                balance,
            )?)?;

        // EIP-6780: Self-destruct only in the same transaction (CANCUN).
        let do_selfdestruct = if vm.env.config.fork >= Fork::Cancun {
            vm.transfer(vm.current_call_frame.to, beneficiary, balance)?;
            vm.substate.is_account_created(&vm.current_call_frame.to)
        } else {
            vm.increase_account_balance(beneficiary, balance)?;
            true
        };
        if do_selfdestruct {
            // For `fork >= CANCUN`, if target is the same as caller, ether will be burnt.
            vm.substate.add_selfdestruct(vm.current_call_frame.to);
            vm.get_account_mut(vm.current_call_frame.to)?.info.balance = U256::zero();
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

        // Add new contract to accessed addresses
        self.substate.add_accessed_address(new_address);

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

        let mut stack = self.stack_pool.pop().unwrap_or_default();
        stack.clear();

        let next_memory = self.current_call_frame.memory.next_memory();

        let new_call_frame = CallFrame::new(
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
        self.add_callframe(new_call_frame);

        // Changes that revert in case the Create fails.
        self.increment_account_nonce(new_address)?; // 0 -> 1
        self.transfer(deployer, new_address, value)?;

        self.substate.push_backup();
        self.substate.add_created_account(new_address); // Mostly for SELFDESTRUCT during initcode.

        Ok(OpcodeResult::Continue)
    }

    #[allow(clippy::too_many_arguments)]
    /// This (should) be the only function where gas is used as a
    /// U256. This is because we have to use the values that are
    /// pushed to the stack.
    ///
    // Force inline, due to lot of arguments, inlining must be forced, and it is actually beneficial
    // because passing so much data is costly. Verified with samply.
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
            let mut gas_remaining = gas_limit;
            let ctx_result = Self::execute_precompile(
                code_address,
                &calldata,
                gas_limit,
                &mut gas_remaining,
                self.env.config.fork,
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
            }

            self.tracer.exit_context(&ctx_result, false)?;
        } else {
            let mut stack = self.stack_pool.pop().unwrap_or_default();
            stack.clear();

            let next_memory = self.current_call_frame.memory.next_memory();

            let new_call_frame = CallFrame::new(
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
            self.add_callframe(new_call_frame);

            // Transfer value from caller to callee.
            if should_transfer_value {
                self.transfer(msg_sender, to, value)?;
            }

            self.substate.push_backup();
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
