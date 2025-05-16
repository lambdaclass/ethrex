use crate::{
    call_frame::CallFrame,
    constants::{CREATE_DEPLOYMENT_FAIL, INIT_CODE_MAX_SIZE, REVERT_FOR_CALL, SUCCESS_FOR_CALL},
    db::cache,
    errors::{ExecutionReport, InternalError, OpcodeResult, OutOfGasError, TxResult, VMError},
    gas_cost::{self, max_message_call_gas},
    memory::{self, calculate_memory_size},
    utils::{address_to_word, word_to_address, *},
    vm::VM,
};
use bytes::Bytes;
use ethrex_common::{
    types::{Account, Fork},
    Address, U256,
};

// System Operations (10)
// Opcodes: CREATE, CALL, CALLCODE, RETURN, DELEGATECALL, CREATE2, STATICCALL, REVERT, INVALID, SELFDESTRUCT

impl<'a> VM<'a> {
    // CALL operation
    pub fn op_call(&mut self) -> Result<OpcodeResult, VMError> {
        let (
            gas,
            callee,
            value_to_transfer,
            current_memory_size,
            args_start_offset,
            args_size,
            return_data_start_offset,
            return_data_size,
        ) = {
            let current_call_frame = self.current_call_frame_mut()?;
            let gas = current_call_frame.stack.pop()?;
            let callee: Address = word_to_address(current_call_frame.stack.pop()?);
            let value_to_transfer: U256 = current_call_frame.stack.pop()?;
            let args_start_offset = current_call_frame.stack.pop()?;
            let args_size = current_call_frame
                .stack
                .pop()?
                .try_into()
                .map_err(|_| VMError::VeryLargeNumber)?;
            let return_data_start_offset = current_call_frame.stack.pop()?;
            let return_data_size: usize = current_call_frame
                .stack
                .pop()?
                .try_into()
                .map_err(|_| VMError::VeryLargeNumber)?;
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
        if self.current_call_frame()?.is_static && !value_to_transfer.is_zero() {
            return Err(VMError::OpcodeNotAllowedInStaticContext);
        }

        // GAS
        let new_memory_size_for_args = calculate_memory_size(args_start_offset, args_size)?;
        let new_memory_size_for_return_data =
            calculate_memory_size(return_data_start_offset, return_data_size)?;
        let new_memory_size = new_memory_size_for_args.max(new_memory_size_for_return_data);

        let (account, address_was_cold) = self.db.access_account(&mut self.substate, callee)?;

        let (is_delegation, eip7702_gas_consumed, code_address, bytecode) =
            eip7702_get_code(self.db, &mut self.substate, callee)?;

        let gas_left = self
            .current_call_frame()?
            .gas_limit
            .checked_sub(self.current_call_frame()?.gas_used)
            .ok_or(InternalError::GasOverflow)?
            .checked_sub(eip7702_gas_consumed)
            .ok_or(InternalError::GasOverflow)?;

        let (cost, gas_limit) = gas_cost::call(
            new_memory_size,
            current_memory_size,
            address_was_cold,
            account.is_empty(),
            value_to_transfer,
            gas,
            gas_left,
        )?;

        self.current_call_frame_mut()?.increase_consumed_gas(cost)?;
        self.current_call_frame_mut()?
            .increase_consumed_gas(eip7702_gas_consumed)?;

        let current_call_frame = self.current_call_frame()?;

        // OPERATION
        let msg_sender = current_call_frame.to; // The new sender will be the current contract.
        let to = callee; // In this case code_address and the sub-context account are the same. Unlike CALLCODE or DELEGATECODE.
        let is_static = current_call_frame.is_static;

        self.generic_call(
            gas_limit,
            value_to_transfer,
            msg_sender,
            to,
            code_address,
            true,
            is_static,
            args_start_offset,
            args_size,
            return_data_start_offset,
            return_data_size,
            bytecode,
            is_delegation,
        )
    }

    // CALLCODE operation
    pub fn op_callcode(&mut self) -> Result<OpcodeResult, VMError> {
        // STACK
        let (
            gas,
            code_address,
            value_to_transfer,
            current_memory_size,
            args_start_offset,
            args_size,
            return_data_start_offset,
            return_data_size,
        ) = {
            let current_call_frame = self.current_call_frame_mut()?;
            let gas = current_call_frame.stack.pop()?;
            let code_address = word_to_address(current_call_frame.stack.pop()?);
            let value_to_transfer = current_call_frame.stack.pop()?;
            let args_start_offset = current_call_frame.stack.pop()?;
            let args_size = current_call_frame
                .stack
                .pop()?
                .try_into()
                .map_err(|_err| VMError::VeryLargeNumber)?;
            let return_data_start_offset = current_call_frame.stack.pop()?;
            let return_data_size = current_call_frame
                .stack
                .pop()?
                .try_into()
                .map_err(|_err| VMError::VeryLargeNumber)?;
            let current_memory_size = current_call_frame.memory.len();
            (
                gas,
                code_address,
                value_to_transfer,
                current_memory_size,
                args_start_offset,
                args_size,
                return_data_start_offset,
                return_data_size,
            )
        };

        // GAS
        let new_memory_size_for_args = calculate_memory_size(args_start_offset, args_size)?;

        let new_memory_size_for_return_data =
            calculate_memory_size(return_data_start_offset, return_data_size)?;
        let new_memory_size = new_memory_size_for_args.max(new_memory_size_for_return_data);

        let (_account_info, address_was_cold) =
            self.db.access_account(&mut self.substate, code_address)?;

        let (is_delegation, eip7702_gas_consumed, code_address, bytecode) =
            eip7702_get_code(self.db, &mut self.substate, code_address)?;

        let gas_left = self
            .current_call_frame()?
            .gas_limit
            .checked_sub(self.current_call_frame()?.gas_used)
            .ok_or(InternalError::GasOverflow)?
            .checked_sub(eip7702_gas_consumed)
            .ok_or(InternalError::GasOverflow)?;

        let (cost, gas_limit) = gas_cost::callcode(
            new_memory_size,
            current_memory_size,
            address_was_cold,
            value_to_transfer,
            gas,
            gas_left,
        )?;

        self.current_call_frame_mut()?.increase_consumed_gas(cost)?;
        self.current_call_frame_mut()?
            .increase_consumed_gas(eip7702_gas_consumed)?;

        let current_call_frame = self.current_call_frame()?;

        // Sender and recipient are the same in this case. But the code executed is from another account.
        let msg_sender = current_call_frame.to;
        let to = current_call_frame.to;
        let is_static = current_call_frame.is_static;

        self.generic_call(
            gas_limit,
            value_to_transfer,
            msg_sender,
            to,
            code_address,
            true,
            is_static,
            args_start_offset,
            args_size,
            return_data_start_offset,
            return_data_size,
            bytecode,
            is_delegation,
        )
    }

    // RETURN operation
    pub fn op_return(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        let offset = current_call_frame.stack.pop()?;
        let size = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;

        if size == 0 {
            return Ok(OpcodeResult::Halt);
        }

        let new_memory_size = calculate_memory_size(offset, size)?;
        let current_memory_size = current_call_frame.memory.len();

        current_call_frame
            .increase_consumed_gas(gas_cost::exit_opcode(new_memory_size, current_memory_size)?)?;

        current_call_frame.output =
            memory::load_range(&mut current_call_frame.memory, offset, size)?
                .to_vec()
                .into();

        Ok(OpcodeResult::Halt)
    }

    // DELEGATECALL operation
    pub fn op_delegatecall(&mut self) -> Result<OpcodeResult, VMError> {
        // STACK
        let (
            gas,
            code_address,
            current_memory_size,
            args_start_offset,
            args_size,
            return_data_start_offset,
            return_data_size,
        ) = {
            let current_call_frame = self.current_call_frame_mut()?;
            let gas = current_call_frame.stack.pop()?;
            let code_address = word_to_address(current_call_frame.stack.pop()?);
            let args_start_offset = current_call_frame.stack.pop()?;
            let args_size = current_call_frame
                .stack
                .pop()?
                .try_into()
                .map_err(|_err| VMError::VeryLargeNumber)?;
            let return_data_start_offset = current_call_frame.stack.pop()?;
            let return_data_size = current_call_frame
                .stack
                .pop()?
                .try_into()
                .map_err(|_err| VMError::VeryLargeNumber)?;
            let current_memory_size = current_call_frame.memory.len();
            (
                gas,
                code_address,
                current_memory_size,
                args_start_offset,
                args_size,
                return_data_start_offset,
                return_data_size,
            )
        };

        // GAS
        let (_account_info, address_was_cold) =
            self.db.access_account(&mut self.substate, code_address)?;

        let new_memory_size_for_args = calculate_memory_size(args_start_offset, args_size)?;
        let new_memory_size_for_return_data =
            calculate_memory_size(return_data_start_offset, return_data_size)?;
        let new_memory_size = new_memory_size_for_args.max(new_memory_size_for_return_data);

        let (is_delegation, eip7702_gas_consumed, code_address, bytecode) =
            eip7702_get_code(self.db, &mut self.substate, code_address)?;

        let gas_left = self
            .current_call_frame()?
            .gas_limit
            .checked_sub(self.current_call_frame()?.gas_used)
            .ok_or(InternalError::GasOverflow)?
            .checked_sub(eip7702_gas_consumed)
            .ok_or(InternalError::GasOverflow)?;

        let (cost, gas_limit) = gas_cost::delegatecall(
            new_memory_size,
            current_memory_size,
            address_was_cold,
            gas,
            gas_left,
        )?;

        self.current_call_frame_mut()?.increase_consumed_gas(cost)?;
        self.current_call_frame_mut()?
            .increase_consumed_gas(eip7702_gas_consumed)?;

        let current_call_frame = self.current_call_frame()?;

        // OPERATION
        let msg_sender = current_call_frame.msg_sender;
        let value = current_call_frame.msg_value;
        let to = current_call_frame.to;
        let is_static = current_call_frame.is_static;

        self.generic_call(
            gas_limit,
            value,
            msg_sender,
            to,
            code_address,
            false,
            is_static,
            args_start_offset,
            args_size,
            return_data_start_offset,
            return_data_size,
            bytecode,
            is_delegation,
        )
    }

    // STATICCALL operation
    pub fn op_staticcall(&mut self) -> Result<OpcodeResult, VMError> {
        // STACK
        let (
            gas,
            code_address,
            current_memory_size,
            args_start_offset,
            args_size,
            return_data_start_offset,
            return_data_size,
        ) = {
            let current_call_frame = self.current_call_frame_mut()?;
            let gas = current_call_frame.stack.pop()?;
            let code_address = word_to_address(current_call_frame.stack.pop()?);
            let args_start_offset = current_call_frame.stack.pop()?;
            let args_size = current_call_frame
                .stack
                .pop()?
                .try_into()
                .map_err(|_err| VMError::VeryLargeNumber)?;
            let return_data_start_offset = current_call_frame.stack.pop()?;
            let return_data_size = current_call_frame
                .stack
                .pop()?
                .try_into()
                .map_err(|_err| VMError::VeryLargeNumber)?;
            let current_memory_size = current_call_frame.memory.len();
            (
                gas,
                code_address,
                current_memory_size,
                args_start_offset,
                args_size,
                return_data_start_offset,
                return_data_size,
            )
        };

        // GAS
        let (_account_info, address_was_cold) =
            self.db.access_account(&mut self.substate, code_address)?;

        let new_memory_size_for_args = calculate_memory_size(args_start_offset, args_size)?;
        let new_memory_size_for_return_data =
            calculate_memory_size(return_data_start_offset, return_data_size)?;
        let new_memory_size = new_memory_size_for_args.max(new_memory_size_for_return_data);

        let (is_delegation, eip7702_gas_consumed, _, bytecode) =
            eip7702_get_code(self.db, &mut self.substate, code_address)?;

        let gas_left = self
            .current_call_frame()?
            .gas_limit
            .checked_sub(self.current_call_frame()?.gas_used)
            .ok_or(InternalError::GasOverflow)?
            .checked_sub(eip7702_gas_consumed)
            .ok_or(InternalError::GasOverflow)?;

        let (cost, gas_limit) = gas_cost::staticcall(
            new_memory_size,
            current_memory_size,
            address_was_cold,
            gas,
            gas_left,
        )?;

        self.current_call_frame_mut()?.increase_consumed_gas(cost)?;
        self.current_call_frame_mut()?
            .increase_consumed_gas(eip7702_gas_consumed)?;

        // OPERATION
        let value = U256::zero();
        let msg_sender = self.current_call_frame()?.to; // The new sender will be the current contract.
        let to = code_address; // In this case code_address and the sub-context account are the same. Unlike CALLCODE or DELEGATECODE.

        self.generic_call(
            gas_limit,
            value,
            msg_sender,
            to,
            code_address,
            true,
            true,
            args_start_offset,
            args_size,
            return_data_start_offset,
            return_data_size,
            bytecode,
            is_delegation,
        )
    }

    // CREATE operation
    pub fn op_create(&mut self) -> Result<OpcodeResult, VMError> {
        let fork = self.env.config.fork;
        let current_call_frame = self.current_call_frame_mut()?;
        let value_in_wei_to_send = current_call_frame.stack.pop()?;
        let code_offset_in_memory = current_call_frame.stack.pop()?;
        let code_size_in_memory: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;

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
        let current_call_frame = self.current_call_frame_mut()?;
        let value_in_wei_to_send = current_call_frame.stack.pop()?;
        let code_offset_in_memory = current_call_frame.stack.pop()?;
        let code_size_in_memory: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;
        let salt = current_call_frame.stack.pop()?;

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
        let current_call_frame = self.current_call_frame_mut()?;

        let offset = current_call_frame.stack.pop()?;

        let size = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;

        let new_memory_size = calculate_memory_size(offset, size)?;
        let current_memory_size = current_call_frame.memory.len();

        current_call_frame
            .increase_consumed_gas(gas_cost::exit_opcode(new_memory_size, current_memory_size)?)?;

        current_call_frame.output =
            memory::load_range(&mut current_call_frame.memory, offset, size)?
                .to_vec()
                .into();

        Err(VMError::RevertOpcode)
    }

    /// ### INVALID operation
    /// Reverts consuming all gas, no return data.
    pub fn op_invalid(&mut self) -> Result<OpcodeResult, VMError> {
        Err(VMError::InvalidOpcode)
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
        let (target_address, to) = {
            let current_call_frame = self.current_call_frame_mut()?;
            if current_call_frame.is_static {
                return Err(VMError::OpcodeNotAllowedInStaticContext);
            }
            let target_address = word_to_address(current_call_frame.stack.pop()?);
            let to = current_call_frame.to;
            (target_address, to)
        };

        let (target_account, target_account_is_cold) =
            self.db.access_account(&mut self.substate, target_address)?;

        let (current_account, _current_account_is_cold) =
            self.db.access_account(&mut self.substate, to)?;
        let balance_to_transfer = current_account.info.balance;

        self.current_call_frame_mut()?
            .increase_consumed_gas(gas_cost::selfdestruct(
                target_account_is_cold,
                target_account.is_empty(),
                balance_to_transfer,
            )?)?;

        // [EIP-6780] - SELFDESTRUCT only in same transaction from CANCUN
        if self.env.config.fork >= Fork::Cancun {
            self.increase_account_balance(target_address, balance_to_transfer)?;
            self.decrease_account_balance(to, balance_to_transfer)?;

            // Selfdestruct is executed in the same transaction as the contract was created
            if self.substate.created_accounts.contains(&to) {
                // If target is the same as the contract calling, Ether will be burnt.
                self.get_account_mut(to)?.info.balance = U256::zero();

                self.substate.selfdestruct_set.insert(to);
            }
        } else {
            self.increase_account_balance(target_address, balance_to_transfer)?;
            self.get_account_mut(to)?.info.balance = U256::zero();

            self.substate.selfdestruct_set.insert(to);
        }

        Ok(OpcodeResult::Halt)
    }

    /// Common behavior for CREATE and CREATE2 opcodes
    pub fn generic_create(
        &mut self,
        value_in_wei_to_send: U256,
        code_offset_in_memory: U256,
        code_size_in_memory: usize,
        salt: Option<U256>,
    ) -> Result<OpcodeResult, VMError> {
        let fork = self.env.config.fork;
        let (deployer_address, max_message_call_gas) = {
            let current_call_frame = self.current_call_frame_mut()?;
            // First: Validations that can cause out of gas.
            // 1. Cant be called in a static context
            if current_call_frame.is_static {
                return Err(VMError::OpcodeNotAllowedInStaticContext);
            }
            // 2. [EIP-3860] - Cant exceed init code max size
            if code_size_in_memory > INIT_CODE_MAX_SIZE && fork >= Fork::Shanghai {
                return Err(VMError::OutOfGas(OutOfGasError::ConsumedGasOverflow));
            }

            // Reserve gas for subcall
            let max_message_call_gas = max_message_call_gas(current_call_frame)?;
            current_call_frame.increase_consumed_gas(max_message_call_gas)?;

            // Clear callframe subreturn data
            current_call_frame.sub_return_data = Bytes::new();

            let deployer_address = current_call_frame.to;
            (deployer_address, max_message_call_gas)
        };

        let deployer_account = self
            .db
            .access_account(&mut self.substate, deployer_address)?
            .0;

        let code = Bytes::from(
            memory::load_range(
                &mut self.current_call_frame_mut()?.memory,
                code_offset_in_memory,
                code_size_in_memory,
            )?
            .to_vec(),
        );

        let new_address = match salt {
            Some(salt) => calculate_create2_address(deployer_address, &code, salt)?,
            None => calculate_create_address(deployer_address, deployer_account.info.nonce)?,
        };

        // touch account
        self.substate.touched_accounts.insert(new_address);

        let new_depth = {
            let current_call_frame = self.current_call_frame_mut()?;
            let new_depth = current_call_frame
                .depth
                .checked_add(1)
                .ok_or(InternalError::ArithmeticOperationOverflow)?;
            // SECOND: Validations that push 0 to the stack and return reserved_gas
            // 1. Sender doesn't have enough balance to send value.
            // 2. Depth limit has been reached
            // 3. Sender nonce is max.
            if deployer_account.info.balance < value_in_wei_to_send
                || new_depth > 1024
                || deployer_account.info.nonce == u64::MAX
            {
                // Return reserved gas
                current_call_frame.gas_used = current_call_frame
                    .gas_used
                    .checked_sub(max_message_call_gas)
                    .ok_or(VMError::Internal(InternalError::GasOverflow))?;
                // Push 0
                current_call_frame.stack.push(CREATE_DEPLOYMENT_FAIL)?;
                return Ok(OpcodeResult::Continue { pc_increment: 1 });
            }
            new_depth
        };

        // THIRD: Validations that push 0 to the stack without returning reserved gas but incrementing deployer's nonce
        let new_account = self.db.get_account(new_address)?;
        if new_account.has_code_or_nonce() {
            self.increment_account_nonce(deployer_address)?;
            self.current_call_frame_mut()?
                .stack
                .push(CREATE_DEPLOYMENT_FAIL)?;
            return Ok(OpcodeResult::Continue { pc_increment: 1 });
        }

        // FOURTH: Changes to the state
        // 1. Creating contract.

        // If the address has balance but there is no account associated with it, we need to add the value to it
        let new_balance = value_in_wei_to_send
            .checked_add(new_account.info.balance)
            .ok_or(VMError::BalanceOverflow)?;

        // https://github.com/ethereum/EIPs/blob/master/EIPS/eip-161.md
        let new_account = Account::new(new_balance, Bytes::new(), 1, Default::default());

        self.insert_account(new_address, new_account)?;

        // 2. Increment sender's nonce.
        self.increment_account_nonce(deployer_address)?;

        // 3. Decrease sender's balance.
        self.decrease_account_balance(deployer_address, value_in_wei_to_send)?;

        let new_call_frame = CallFrame::new(
            deployer_address,
            new_address,
            new_address,
            code,
            value_in_wei_to_send,
            Bytes::new(),
            false,
            max_message_call_gas,
            new_depth,
            true,
            true,
            U256::zero(),
            0,
        );
        self.call_frames.push(new_call_frame);

        self.substate.created_accounts.insert(new_address); // Mostly for SELFDESTRUCT during initcode.

        self.backup_substate();

        Ok(OpcodeResult::Continue { pc_increment: 0 })
    }

    #[allow(clippy::too_many_arguments)]
    /// This (should) be the only function where gas is used as a
    /// U256. This is because we have to use the values that are
    /// pushed to the stack.
    pub fn generic_call(
        &mut self,
        gas_limit: u64,
        value: U256,
        msg_sender: Address,
        to: Address,
        code_address: Address,
        should_transfer_value: bool,
        is_static: bool,
        args_offset: U256,
        args_size: usize,
        ret_offset: U256,
        ret_size: usize,
        bytecode: Bytes,
        is_delegation: bool,
    ) -> Result<OpcodeResult, VMError> {
        let sender_account = self.db.access_account(&mut self.substate, msg_sender)?.0;

        let calldata = {
            let current_call_frame = self.current_call_frame_mut()?;
            // Clear callframe subreturn data
            current_call_frame.sub_return_data = Bytes::new();

            let calldata =
                memory::load_range(&mut current_call_frame.memory, args_offset, args_size)?
                    .to_vec();

            // 1. Validate sender has enough value
            if should_transfer_value && sender_account.info.balance < value {
                current_call_frame.gas_used = current_call_frame
                    .gas_used
                    .checked_sub(gas_limit)
                    .ok_or(InternalError::GasOverflow)?;
                current_call_frame.stack.push(REVERT_FOR_CALL)?;
                return Ok(OpcodeResult::Continue { pc_increment: 1 });
            }
            calldata
        };

        let new_depth = {
            let current_call_frame = self.current_call_frame_mut()?;

            // 2. Validate max depth has not been reached yet.
            let new_depth = current_call_frame
                .depth
                .checked_add(1)
                .ok_or(InternalError::ArithmeticOperationOverflow)?;

            if new_depth > 1024 {
                current_call_frame.gas_used = current_call_frame
                    .gas_used
                    .checked_sub(gas_limit)
                    .ok_or(InternalError::GasOverflow)?;
                current_call_frame.stack.push(REVERT_FOR_CALL)?;
                return Ok(OpcodeResult::Continue { pc_increment: 1 });
            }

            if bytecode.is_empty() && is_delegation {
                current_call_frame.gas_used = current_call_frame
                    .gas_used
                    .checked_sub(gas_limit)
                    .ok_or(InternalError::GasOverflow)?;
                current_call_frame.stack.push(SUCCESS_FOR_CALL)?;
                return Ok(OpcodeResult::Continue { pc_increment: 1 });
            }
            new_depth
        };
        // Transfer value from caller to callee.
        if should_transfer_value {
            self.decrease_account_balance(msg_sender, value)?;
            self.increase_account_balance(to, value)?;
        }

        let new_call_frame = CallFrame::new(
            msg_sender,
            to,
            code_address,
            bytecode,
            value,
            calldata.into(),
            is_static,
            gas_limit,
            new_depth,
            should_transfer_value,
            false,
            ret_offset,
            ret_size,
        );
        self.call_frames.push(new_call_frame);

        if self.is_precompile()? {
            // Execute precompile immediately and handle result.
            let report = self.execute_precompile()?;
            self.handle_return(&report)?;
        } else {
            // Backup Substate before executing opcodes of new callframe.
            self.backup_substate();
        }

        Ok(OpcodeResult::Continue { pc_increment: 0 })
    }

    /// Handles case in which callframe was initiated by another callframe (with CALL or CREATE family opcodes)
    pub fn handle_return(&mut self, tx_report: &ExecutionReport) -> Result<(), VMError> {
        let executed_call_frame = self.pop_call_frame()?;

        // Here happens the interaction between child (executed) and parent (caller) callframe.
        if executed_call_frame.create_op_called {
            self.handle_return_create(&executed_call_frame, tx_report)?;
        } else {
            self.handle_return_call(&executed_call_frame, tx_report)?;
        }

        // Increment PC of the parent callframe after execution of the child.
        self.increment_pc_by(1)?;

        Ok(())
    }

    pub fn handle_return_call(
        &mut self,
        executed_call_frame: &CallFrame,
        tx_report: &ExecutionReport,
    ) -> Result<(), VMError> {
        // Return gas left from subcontext
        let gas_left_from_new_call_frame = executed_call_frame
            .gas_limit
            .checked_sub(tx_report.gas_used)
            .ok_or(InternalError::GasOverflow)?;
        {
            let parent_call_frame = self.current_call_frame_mut()?;
            parent_call_frame.gas_used = parent_call_frame
                .gas_used
                .checked_sub(gas_left_from_new_call_frame)
                .ok_or(InternalError::GasOverflow)?;

            parent_call_frame.logs.extend(tx_report.logs.clone());
            memory::try_store_range(
                &mut parent_call_frame.memory,
                executed_call_frame.ret_offset,
                executed_call_frame.ret_size,
                &tx_report.output,
            )?;
            parent_call_frame.sub_return_data = tx_report.output.clone();
        }

        // What to do, depending on TxResult
        match tx_report.result {
            TxResult::Success => {
                self.current_call_frame_mut()?
                    .stack
                    .push(SUCCESS_FOR_CALL)?;
                self.merge_call_frame_backup_with_parent(&executed_call_frame.call_frame_backup)?;
            }
            TxResult::Revert(_) => {
                // Revert value transfer
                if executed_call_frame.should_transfer_value {
                    self.decrease_account_balance(
                        executed_call_frame.to,
                        executed_call_frame.msg_value,
                    )?;

                    self.increase_account_balance(
                        executed_call_frame.msg_sender,
                        executed_call_frame.msg_value,
                    )?;
                }
                // Push 0 to stack
                self.current_call_frame_mut()?.stack.push(REVERT_FOR_CALL)?;
            }
        }
        Ok(())
    }

    pub fn handle_return_create(
        &mut self,
        executed_call_frame: &CallFrame,
        tx_report: &ExecutionReport,
    ) -> Result<(), VMError> {
        let unused_gas = executed_call_frame
            .gas_limit
            .checked_sub(tx_report.gas_used)
            .ok_or(InternalError::GasOverflow)?;

        {
            let parent_call_frame = self.current_call_frame_mut()?;
            // Return reserved gas
            parent_call_frame.gas_used = parent_call_frame
                .gas_used
                .checked_sub(unused_gas)
                .ok_or(InternalError::GasOverflow)?;

            parent_call_frame.logs.extend(tx_report.logs.clone());
        }

        match tx_report.result.clone() {
            TxResult::Success => {
                self.current_call_frame_mut()?
                    .stack
                    .push(address_to_word(executed_call_frame.to))?;
                self.merge_call_frame_backup_with_parent(&executed_call_frame.call_frame_backup)?;
            }
            TxResult::Revert(err) => {
                // Return value to sender
                self.increase_account_balance(
                    executed_call_frame.msg_sender,
                    executed_call_frame.msg_value,
                )?;

                // Deployment failed so account shouldn't exist
                cache::remove_account(&mut self.db.cache, &executed_call_frame.to);
                self.substate
                    .created_accounts
                    .remove(&executed_call_frame.to);

                let current_call_frame = self.current_call_frame_mut()?;
                // If revert we have to copy the return_data
                if err == VMError::RevertOpcode {
                    current_call_frame.sub_return_data = tx_report.output.clone();
                }
                current_call_frame.stack.push(CREATE_DEPLOYMENT_FAIL)?;
            }
        }
        Ok(())
    }
}
