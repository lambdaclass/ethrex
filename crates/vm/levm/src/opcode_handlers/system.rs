use crate::{
    call_frame::CallFrame,
    constants::{FAIL, INIT_CODE_MAX_SIZE, SUCCESS},
    errors::{ContextResult, ExceptionalHalt, InternalError, OpcodeResult, TxResult, VMError},
    gas_cost::{self, max_message_call_gas},
    memory::calculate_memory_size,
    utils::{address_to_word, word_to_address, *},
    vm::VM,
};
use bytes::Bytes;
use ethrex_common::tracing::CallType::{
    self, CALL, CALLCODE, DELEGATECALL, SELFDESTRUCT, STATICCALL,
};
use ethrex_common::{Address, U256, types::Fork};

// System Operations (10)
// Opcodes: CREATE, CALL, CALLCODE, RETURN, DELEGATECALL, CREATE2, STATICCALL, REVERT, INVALID, SELFDESTRUCT

impl<'a> VM<'a> {
    // CALL operation
    pub fn op_call(&mut self) -> Result<OpcodeResult, VMError> {
        let (
            gas,
            callee,
            value,
            current_memory_size,
            args_start_offset,
            args_size,
            return_data_start_offset,
            return_data_size,
        ) = {
            let current_call_frame = &mut self.current_call_frame;
            let [
                gas,
                callee,
                value_to_transfer,
                args_start_offset,
                args_size,
                return_data_start_offset,
                return_data_size,
            ] = *current_call_frame.stack.pop()?;
            let callee: Address = word_to_address(callee);
            let args_start_offset = args_start_offset
                .try_into()
                .map_err(|_| ExceptionalHalt::VeryLargeNumber)?;
            let args_size = args_size
                .try_into()
                .map_err(|_| ExceptionalHalt::VeryLargeNumber)?;
            let return_data_start_offset: usize = return_data_start_offset
                .try_into()
                .map_err(|_| ExceptionalHalt::VeryLargeNumber)?;
            let return_data_size: usize = return_data_size
                .try_into()
                .map_err(|_| ExceptionalHalt::VeryLargeNumber)?;
            let current_memory_size = current_call_frame.memory.len();
            (
                gas,
                callee,
                value_to_transfer,
                current_memory_size,
                args_start_offset,
                args_size,
                return_data_start_offset,
                return_data_size,
            )
        };

        // VALIDATIONS
        if self.current_call_frame.is_static && !value.is_zero() {
            return Err(ExceptionalHalt::OpcodeNotAllowedInStaticContext.into());
        }

        // CHECK EIP7702
        let (is_delegation_7702, eip7702_gas_consumed, code_address, bytecode) =
            eip7702_get_code(self.db, &mut self.substate, callee)?;

        // GAS
        let (new_memory_size, gas_left, account_is_empty, address_was_cold) = self
            .get_call_gas_params(
                args_start_offset,
                args_size,
                return_data_start_offset,
                return_data_size,
                eip7702_gas_consumed,
                callee,
            )?;

        let (cost, gas_limit) = gas_cost::call(
            new_memory_size,
            current_memory_size,
            address_was_cold,
            account_is_empty,
            value,
            gas,
            gas_left,
        )?;

        let callframe = &mut self.current_call_frame;
        callframe.increase_consumed_gas(
            cost.checked_add(eip7702_gas_consumed)
                .ok_or(ExceptionalHalt::OutOfGas)?,
        )?;

        // Make sure we have enough memory to write the return data
        // This is also needed to make sure we expand the memory even in cases where we don't have return data (such as transfers)
        callframe.memory.resize(new_memory_size)?;

        // OPERATION
        let from = callframe.to; // The new sender will be the current contract.
        let to = callee; // In this case code_address and the sub-context account are the same. Unlike CALLCODE or DELEGATECODE.
        let is_static = callframe.is_static;
        let data = self.get_calldata(args_start_offset, args_size)?;

        self.tracer.enter(CALL, from, to, value, gas_limit, &data);

        self.generic_call(
            gas_limit,
            value,
            from,
            to,
            code_address,
            true,
            is_static,
            data,
            return_data_start_offset,
            return_data_size,
            bytecode,
            is_delegation_7702,
        )
    }

    // CALLCODE operation
    pub fn op_callcode(&mut self) -> Result<OpcodeResult, VMError> {
        // STACK
        let (
            gas,
            address,
            value,
            current_memory_size,
            args_start_offset,
            args_size,
            return_data_start_offset,
            return_data_size,
        ) = {
            let current_call_frame = &mut self.current_call_frame;
            let [
                gas,
                address,
                value_to_transfer,
                args_start_offset,
                args_size,
                return_data_start_offset,
                return_data_size,
            ] = *current_call_frame.stack.pop()?;
            let address = word_to_address(address);
            let args_size = args_size
                .try_into()
                .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;
            let args_start_offset = match args_start_offset.try_into() {
                Ok(x) => x,
                Err(_) => usize::MAX,
            };
            let return_data_start_offset = return_data_start_offset
                .try_into()
                .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;
            let return_data_size = return_data_size
                .try_into()
                .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;
            let current_memory_size = current_call_frame.memory.len();
            (
                gas,
                address,
                value_to_transfer,
                current_memory_size,
                args_start_offset,
                args_size,
                return_data_start_offset,
                return_data_size,
            )
        };

        // CHECK EIP7702
        let (is_delegation_7702, eip7702_gas_consumed, code_address, bytecode) =
            eip7702_get_code(self.db, &mut self.substate, address)?;
        // GAS
        let (new_memory_size, gas_left, _account_is_empty, address_was_cold) = self
            .get_call_gas_params(
                args_start_offset,
                args_size,
                return_data_start_offset,
                return_data_size,
                eip7702_gas_consumed,
                address,
            )?;

        let (cost, gas_limit) = gas_cost::callcode(
            new_memory_size,
            current_memory_size,
            address_was_cold,
            value,
            gas,
            gas_left,
        )?;

        let callframe = &mut self.current_call_frame;
        callframe.increase_consumed_gas(
            cost.checked_add(eip7702_gas_consumed)
                .ok_or(ExceptionalHalt::OutOfGas)?,
        )?;

        // Make sure we have enough memory to write the return data
        // This is also needed to make sure we expand the memory even in cases where we don't have return data (such as transfers)
        callframe.memory.resize(new_memory_size)?;

        // Sender and recipient are the same in this case. But the code executed is from another account.
        let from = callframe.to;
        let to = callframe.to;
        let is_static = callframe.is_static;
        let data = self.get_calldata(args_start_offset, args_size)?;

        self.tracer
            .enter(CALLCODE, from, code_address, value, gas_limit, &data);

        self.generic_call(
            gas_limit,
            value,
            from,
            to,
            code_address,
            true,
            is_static,
            data,
            return_data_start_offset,
            return_data_size,
            bytecode,
            is_delegation_7702,
        )
    }

    // RETURN operation
    pub fn op_return(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        let [offset, size] = *current_call_frame.stack.pop()?;
        let size = size
            .try_into()
            .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;

        if size == 0 {
            return Ok(OpcodeResult::Halt);
        }

        let offset: usize = match offset.try_into() {
            Ok(x) => x,
            Err(_) => usize::MAX,
        };

        let new_memory_size = calculate_memory_size(offset, size)?;
        let current_memory_size = current_call_frame.memory.len();

        current_call_frame
            .increase_consumed_gas(gas_cost::exit_opcode(new_memory_size, current_memory_size)?)?;

        current_call_frame.output = current_call_frame.memory.load_range(offset, size)?.into();

        Ok(OpcodeResult::Halt)
    }

    // DELEGATECALL operation
    pub fn op_delegatecall(&mut self) -> Result<OpcodeResult, VMError> {
        // STACK
        let (
            gas,
            address,
            current_memory_size,
            args_start_offset,
            args_size,
            return_data_start_offset,
            return_data_size,
        ) = {
            let current_call_frame = &mut self.current_call_frame;
            let [
                gas,
                address,
                args_start_offset,
                args_size,
                return_data_start_offset,
                return_data_size,
            ] = *current_call_frame.stack.pop()?;
            let address = word_to_address(address);
            let args_start_offset = args_start_offset
                .try_into()
                .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;
            let args_size = args_size
                .try_into()
                .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;
            let return_data_start_offset = return_data_start_offset
                .try_into()
                .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;
            let return_data_size = return_data_size
                .try_into()
                .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;
            let current_memory_size = current_call_frame.memory.len();
            (
                gas,
                address,
                current_memory_size,
                args_start_offset,
                args_size,
                return_data_start_offset,
                return_data_size,
            )
        };

        // CHECK EIP7702
        let (is_delegation_7702, eip7702_gas_consumed, code_address, bytecode) =
            eip7702_get_code(self.db, &mut self.substate, address)?;

        // GAS
        let (new_memory_size, gas_left, _account_is_empty, address_was_cold) = self
            .get_call_gas_params(
                args_start_offset,
                args_size,
                return_data_start_offset,
                return_data_size,
                eip7702_gas_consumed,
                address,
            )?;

        let (cost, gas_limit) = gas_cost::delegatecall(
            new_memory_size,
            current_memory_size,
            address_was_cold,
            gas,
            gas_left,
        )?;

        let callframe = &mut self.current_call_frame;
        callframe.increase_consumed_gas(
            cost.checked_add(eip7702_gas_consumed)
                .ok_or(ExceptionalHalt::OutOfGas)?,
        )?;

        // Make sure we have enough memory to write the return data
        // This is also needed to make sure we expand the memory even in cases where we don't have return data (such as transfers)
        callframe.memory.resize(new_memory_size)?;

        // OPERATION
        let from = callframe.msg_sender;
        let value = callframe.msg_value;
        let to = callframe.to;
        let is_static = callframe.is_static;
        let data = self.get_calldata(args_start_offset, args_size)?;

        self.tracer
            .enter(DELEGATECALL, from, code_address, value, gas_limit, &data);

        self.generic_call(
            gas_limit,
            value,
            from,
            to,
            code_address,
            false,
            is_static,
            data,
            return_data_start_offset,
            return_data_size,
            bytecode,
            is_delegation_7702,
        )
    }

    // STATICCALL operation
    pub fn op_staticcall(&mut self) -> Result<OpcodeResult, VMError> {
        // STACK
        let (
            gas,
            address,
            current_memory_size,
            args_start_offset,
            args_size,
            return_data_start_offset,
            return_data_size,
        ) = {
            let current_call_frame = &mut self.current_call_frame;
            let [
                gas,
                address,
                args_start_offset,
                args_size,
                return_data_start_offset,
                return_data_size,
            ] = *current_call_frame.stack.pop()?;
            let address = word_to_address(address);
            let args_start_offset = args_start_offset
                .try_into()
                .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;
            let args_size = args_size
                .try_into()
                .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;
            let return_data_start_offset = return_data_start_offset
                .try_into()
                .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;
            let return_data_size = return_data_size
                .try_into()
                .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;
            let current_memory_size = current_call_frame.memory.len();
            (
                gas,
                address,
                current_memory_size,
                args_start_offset,
                args_size,
                return_data_start_offset,
                return_data_size,
            )
        };

        // CHECK EIP7702
        let (is_delegation_7702, eip7702_gas_consumed, _, bytecode) =
            eip7702_get_code(self.db, &mut self.substate, address)?;

        // GAS
        let (new_memory_size, gas_left, _account_is_empty, address_was_cold) = self
            .get_call_gas_params(
                args_start_offset,
                args_size,
                return_data_start_offset,
                return_data_size,
                eip7702_gas_consumed,
                address,
            )?;

        let (cost, gas_limit) = gas_cost::staticcall(
            new_memory_size,
            current_memory_size,
            address_was_cold,
            gas,
            gas_left,
        )?;

        let callframe = &mut self.current_call_frame;
        callframe.increase_consumed_gas(
            cost.checked_add(eip7702_gas_consumed)
                .ok_or(ExceptionalHalt::OutOfGas)?,
        )?;

        // Make sure we have enough memory to write the return data
        // This is also needed to make sure we expand the memory even in cases where we don't have return data (such as transfers)
        callframe.memory.resize(new_memory_size)?;

        // OPERATION
        let value = U256::zero();
        let from = callframe.to; // The new sender will be the current contract.
        let to = address; // In this case address and the sub-context account are the same. Unlike CALLCODE or DELEGATECODE.
        let data = self.get_calldata(args_start_offset, args_size)?;

        self.tracer
            .enter(STATICCALL, from, to, value, gas_limit, &data);

        self.generic_call(
            gas_limit,
            value,
            from,
            to,
            address,
            true,
            true,
            data,
            return_data_start_offset,
            return_data_size,
            bytecode,
            is_delegation_7702,
        )
    }

    // CREATE operation
    pub fn op_create(&mut self) -> Result<OpcodeResult, VMError> {
        let fork = self.env.config.fork;
        let current_call_frame = &mut self.current_call_frame;
        let [
            value_in_wei_to_send,
            code_offset_in_memory,
            code_size_in_memory,
        ] = *current_call_frame.stack.pop()?;
        let code_size_in_memory: usize = code_size_in_memory
            .try_into()
            .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;
        let code_offset_in_memory: usize = match code_offset_in_memory.try_into() {
            Ok(x) => x,
            Err(_) => usize::MAX,
        };

        let new_size = calculate_memory_size(code_offset_in_memory, code_size_in_memory)?;

        current_call_frame.increase_consumed_gas(gas_cost::create(
            new_size,
            current_call_frame.memory.len(),
            code_size_in_memory,
            fork,
        )?)?;

        self.generic_create(
            value_in_wei_to_send,
            code_offset_in_memory,
            code_size_in_memory,
            None,
        )
    }

    // CREATE2 operation
    pub fn op_create2(&mut self) -> Result<OpcodeResult, VMError> {
        let fork = self.env.config.fork;
        let current_call_frame = &mut self.current_call_frame;
        let [
            value_in_wei_to_send,
            code_offset_in_memory,
            code_size_in_memory,
            salt,
        ] = *current_call_frame.stack.pop()?;
        let code_size_in_memory: usize = code_size_in_memory
            .try_into()
            .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;
        let code_offset_in_memory: usize = match code_offset_in_memory.try_into() {
            Ok(x) => x,
            Err(_) => usize::MAX,
        };

        let new_size = calculate_memory_size(code_offset_in_memory, code_size_in_memory)?;

        current_call_frame.increase_consumed_gas(gas_cost::create_2(
            new_size,
            current_call_frame.memory.len(),
            code_size_in_memory,
            fork,
        )?)?;

        self.generic_create(
            value_in_wei_to_send,
            code_offset_in_memory,
            code_size_in_memory,
            Some(salt),
        )
    }

    // REVERT operation
    pub fn op_revert(&mut self) -> Result<OpcodeResult, VMError> {
        // Description: Gets values from stack, calculates gas cost and sets return data.
        // Returns: VMError RevertOpcode if executed correctly.
        // Notes:
        //      The actual reversion of changes is made in the execute() function.
        let current_call_frame = &mut self.current_call_frame;

        let [offset, size] = *current_call_frame.stack.pop()?;

        let offset = match offset.try_into() {
            Ok(x) => x,
            Err(_) => usize::MAX,
        };

        let size = size
            .try_into()
            .map_err(|_| ExceptionalHalt::VeryLargeNumber)?;

        let new_memory_size = calculate_memory_size(offset, size)?;
        let current_memory_size = current_call_frame.memory.len();

        current_call_frame
            .increase_consumed_gas(gas_cost::exit_opcode(new_memory_size, current_memory_size)?)?;

        current_call_frame.output = current_call_frame.memory.load_range(offset, size)?.into();

        Err(VMError::RevertOpcode)
    }

    /// ### INVALID operation
    /// Reverts consuming all gas, no return data.
    pub fn op_invalid(&mut self) -> Result<OpcodeResult, VMError> {
        Err(ExceptionalHalt::InvalidOpcode.into())
    }

    // SELFDESTRUCT operation
    pub fn op_selfdestruct(&mut self) -> Result<OpcodeResult, VMError> {
        // Sends all ether in the account to the target address
        // Steps:
        // 1. Pop the target address from the stack
        // 2. Get current account and: Store the balance in a variable, set it's balance to 0
        // 3. Get the target account, checking if it is empty and if it is cold. Update gas cost accordingly.
        // 4. Add the balance of the current account to the target account
        // 5. Register account to be destroyed in accrued substate.
        // Notes:
        //      If context is Static, return error.
        //      If executed in the same transaction a contract was created, the current account is registered to be destroyed
        let (beneficiary, to) = {
            let current_call_frame = &mut self.current_call_frame;
            if current_call_frame.is_static {
                return Err(ExceptionalHalt::OpcodeNotAllowedInStaticContext.into());
            }
            let target_address = word_to_address(current_call_frame.stack.pop1()?);
            let to = current_call_frame.to;
            (target_address, to)
        };

        let target_account_is_cold = self.substate.accessed_addresses.insert(beneficiary);
        let target_account_is_empty = self.db.get_account(beneficiary)?.is_empty();

        let current_account = self.db.get_account(to)?;
        let balance = current_account.info.balance;

        self.current_call_frame
            .increase_consumed_gas(gas_cost::selfdestruct(
                target_account_is_cold,
                target_account_is_empty,
                balance,
            )?)?;

        // [EIP-6780] - SELFDESTRUCT only in same transaction from CANCUN
        if self.env.config.fork >= Fork::Cancun {
            self.transfer(to, beneficiary, balance)?;

            // Selfdestruct is executed in the same transaction as the contract was created
            if self.substate.created_accounts.contains(&to) {
                // If target is the same as the contract calling, Ether will be burnt.
                self.get_account_mut(to)?.info.balance = U256::zero();

                self.substate.selfdestruct_set.insert(to);
            }
        } else {
            self.increase_account_balance(beneficiary, balance)?;
            self.get_account_mut(to)?.info.balance = U256::zero();

            self.substate.selfdestruct_set.insert(to);
        }

        self.tracer
            .enter(SELFDESTRUCT, to, beneficiary, balance, 0, &Bytes::new());

        self.tracer.exit_early(0, None)?;

        Ok(OpcodeResult::Halt)
    }

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
        let gas_limit = max_message_call_gas(current_call_frame)?;
        current_call_frame.increase_consumed_gas(gas_limit)?;

        // Load code from memory
        let code = Bytes::from(
            self.current_call_frame
                .memory
                .load_range(code_offset_in_memory, code_size_in_memory)?,
        );

        // Get account info of deployer
        let deployer = self.current_call_frame.to;
        let (deployer_balance, deployer_nonce) = {
            let deployer_account = self.db.get_account(deployer)?;
            (deployer_account.info.balance, deployer_account.info.nonce)
        };

        // Calculate create address
        let new_address = match salt {
            Some(salt) => calculate_create2_address(deployer, &code, salt)?,
            None => calculate_create_address(deployer, deployer_nonce)?,
        };

        // Add new contract to accessed addresses
        self.substate.accessed_addresses.insert(new_address);

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
                return Ok(OpcodeResult::Continue { pc_increment: 1 });
            }
        }

        // Increment sender nonce (irreversible change)
        self.increment_account_nonce(deployer)?;

        // Deployment will fail (consuming all gas) if the contract already exists.
        let new_account = self.get_account_mut(new_address)?;
        if new_account.has_code_or_nonce() {
            self.current_call_frame.stack.push1(FAIL)?;
            self.tracer
                .exit_early(gas_limit, Some("CreateAccExists".to_string()))?;
            return Ok(OpcodeResult::Continue { pc_increment: 1 });
        }

        let mut stack = self.stack_pool.pop().unwrap_or_default();
        stack.clear();

        let next_memory = self.current_call_frame.memory.next_memory();

        let new_call_frame = CallFrame::new(
            deployer,
            new_address,
            new_address,
            code,
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

        self.backup_substate();

        self.substate.created_accounts.insert(new_address); // Mostly for SELFDESTRUCT during initcode.

        Ok(OpcodeResult::Continue { pc_increment: 0 })
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
        bytecode: Bytes,
        is_delegation_7702: bool,
    ) -> Result<OpcodeResult, VMError> {
        // Clear callframe subreturn data
        self.current_call_frame.sub_return_data.clear();

        // Validate sender has enough value
        if should_transfer_value && !value.is_zero() {
            let sender_balance = self.db.get_account(msg_sender)?.info.balance;
            if sender_balance < value {
                self.early_revert_message_call(gas_limit, "OutOfFund".to_string())?;
                return Ok(OpcodeResult::Continue { pc_increment: 1 });
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
            return Ok(OpcodeResult::Continue { pc_increment: 1 });
        }

        if self.is_precompile(&code_address) && !is_delegation_7702 {
            let mut gas_remaining = gas_limit;
            let ctx_result = Self::execute_precompile(
                self.vm_type,
                code_address,
                &calldata,
                gas_limit,
                &mut gas_remaining,
            )?;

            let call_frame = &mut self.current_call_frame;

            // Return gas left from subcontext
            if ctx_result.is_success() {
                call_frame.gas_remaining = call_frame
                    .gas_remaining
                    .checked_add(
                        gas_limit
                            .checked_sub(ctx_result.gas_used)
                            .ok_or(InternalError::Underflow)?,
                    )
                    .ok_or(InternalError::Overflow)?;
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
            call_frame.stack.push1(match &ctx_result.result {
                TxResult::Success => SUCCESS,
                TxResult::Revert(_) => FAIL,
            })?;

            // Increment PC of the parent callframe after execution of the child.
            call_frame.increment_pc_by(1)?;

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

            self.backup_substate();
        }

        Ok(OpcodeResult::Continue { pc_increment: 0 })
    }

    /// Pop backup from stack and restore substate and cache if transaction reverted.
    pub fn handle_state_backup(&mut self, ctx_result: &ContextResult) -> Result<(), VMError> {
        let backup = self
            .substate_backups
            .pop()
            .ok_or(InternalError::CallFrame)?;
        if !ctx_result.is_success() {
            self.substate = backup;
            self.restore_cache_state()?;
        }
        Ok(())
    }

    /// Handles case in which callframe was initiated by another callframe (with CALL or CREATE family opcodes)
    pub fn handle_return(&mut self, ctx_result: &ContextResult) -> Result<(), VMError> {
        self.handle_state_backup(ctx_result)?;
        let executed_call_frame = self.pop_call_frame()?;

        // Here happens the interaction between child (executed) and parent (caller) callframe.
        if executed_call_frame.is_create {
            self.handle_return_create(executed_call_frame, ctx_result)?;
        } else {
            self.handle_return_call(executed_call_frame, ctx_result)?;
        }

        // Increment PC of the parent callframe after execution of the child.
        self.increment_pc_by(1)?;

        Ok(())
    }

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
            .checked_add(child_unused_gas)
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
                self.current_call_frame.stack.push1(SUCCESS)?;
                self.merge_call_frame_backup_with_parent(&executed_call_frame.call_frame_backup)?;
            }
            TxResult::Revert(_) => {
                self.current_call_frame.stack.push1(FAIL)?;
            }
        };

        self.tracer.exit_context(ctx_result, false)?;

        let mut stack = executed_call_frame.stack;
        stack.clear();
        self.stack_pool.push(stack);

        Ok(())
    }

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
            .checked_add(unused_gas)
            .ok_or(InternalError::Overflow)?;

        // What to do, depending on TxResult
        match ctx_result.result.clone() {
            TxResult::Success => {
                parent_call_frame.stack.push1(address_to_word(to))?;
                self.merge_call_frame_backup_with_parent(&call_frame_backup)?;
            }
            TxResult::Revert(err) => {
                // If revert we have to copy the return_data
                if err.is_revert_opcode() {
                    parent_call_frame.sub_return_data = ctx_result.output.clone();
                }

                parent_call_frame.stack.push1(FAIL)?;
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
        args_start_offset: usize,
        args_size: usize,
        return_data_start_offset: usize,
        return_data_size: usize,
        eip7702_gas_consumed: u64,
        address: Address,
    ) -> Result<(usize, u64, bool, bool), VMError> {
        // Creation of previously empty accounts and cold addresses have higher gas cost
        let address_was_cold = self.substate.accessed_addresses.insert(address);
        let account_is_empty = self.db.get_account(address)?.is_empty();

        // Calculated here for memory expansion gas cost
        let new_memory_size_for_args = calculate_memory_size(args_start_offset, args_size)?;
        let new_memory_size_for_return_data =
            calculate_memory_size(return_data_start_offset, return_data_size)?;
        let new_memory_size = new_memory_size_for_args.max(new_memory_size_for_return_data);
        // Calculate remaining gas after EIP7702 consumption
        let gas_left = self
            .current_call_frame
            .gas_remaining
            .checked_sub(eip7702_gas_consumed)
            .ok_or(ExceptionalHalt::OutOfGas)?;

        Ok((
            new_memory_size,
            gas_left,
            account_is_empty,
            address_was_cold,
        ))
    }

    fn get_calldata(&mut self, offset: usize, size: usize) -> Result<Bytes, VMError> {
        Ok(Bytes::from(
            self.current_call_frame.memory.load_range(offset, size)?,
        ))
    }

    fn early_revert_message_call(&mut self, gas_limit: u64, reason: String) -> Result<(), VMError> {
        let callframe = &mut self.current_call_frame;

        // Return gas_limit to callframe.
        callframe.gas_remaining = callframe
            .gas_remaining
            .checked_add(gas_limit)
            .ok_or(InternalError::Overflow)?;
        callframe.stack.push1(FAIL)?; // It's the same as revert for CREATE

        self.tracer.exit_early(0, Some(reason))?;
        Ok(())
    }
}
