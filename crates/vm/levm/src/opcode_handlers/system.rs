use crate::{
    call_frame::CallFrame,
    constants::{CREATE_DEPLOYMENT_FAIL, INIT_CODE_MAX_SIZE, REVERT_FOR_CALL, SUCCESS_FOR_CALL},
    db::cache,
    errors::{InternalError, OpcodeSuccess, OutOfGasError, ResultReason, TxResult, VMError},
    gas_cost::{
        self, max_message_call_gas, CALLCODE_POSITIVE_VALUE_STIPEND, CALL_POSITIVE_VALUE_STIPEND,
    },
    memory::{self, calculate_memory_size},
    vm::{address_to_word, word_to_address, VM},
    Account,
};
use bytes::Bytes;
use ethrex_core::{Address, U256};

// System Operations (10)
// Opcodes: CREATE, CALL, CALLCODE, RETURN, DELEGATECALL, CREATE2, STATICCALL, REVERT, INVALID, SELFDESTRUCT

impl VM {
    // CALL operation
    pub fn op_call(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        // STACK
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

        // VALIDATIONS
        if current_call_frame.is_static && !value_to_transfer.is_zero() {
            return Err(VMError::OpcodeNotAllowedInStaticContext);
        }

        // GAS
        let current_memory_size = current_call_frame.memory.len();
        let new_memory_size_for_args = calculate_memory_size(args_start_offset, args_size)?;
        let new_memory_size_for_return_data =
            calculate_memory_size(return_data_start_offset, return_data_size)?;
        let new_memory_size = new_memory_size_for_args.max(new_memory_size_for_return_data);

        let (account_info, address_was_cold) = self.access_account(callee);

        self.increase_consumed_gas(
            current_call_frame,
            gas_cost::call(
                new_memory_size,
                current_memory_size,
                address_was_cold,
                account_info.is_empty(),
                value_to_transfer,
            )?,
        )?;

        // OPERATION
        let msg_sender = current_call_frame.to; // The new sender will be the current contract.
        let to = callee; // In this case code_address and the sub-context account are the same. Unlike CALLCODE or DELEGATECODE.
        let is_static = current_call_frame.is_static;

        // We add the stipend gas for the subcall. This ensures that the callee has enough gas to perform basic operations
        let gas_for_subcall = if !value_to_transfer.is_zero() {
            gas.saturating_add(CALL_POSITIVE_VALUE_STIPEND)
        } else {
            gas
        };

        self.generic_call(
            current_call_frame,
            gas_for_subcall,
            value_to_transfer,
            msg_sender,
            to,
            callee,
            true,
            is_static,
            args_start_offset,
            args_size,
            return_data_start_offset,
            return_data_size,
        )
    }

    // CALLCODE operation
    // TODO: https://github.com/lambdaclass/ethrex/issues/1086
    pub fn op_callcode(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        // STACK
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

        // GAS
        let current_memory_size = current_call_frame.memory.len();
        let new_memory_size_for_args = calculate_memory_size(args_start_offset, args_size)?;

        let new_memory_size_for_return_data =
            calculate_memory_size(return_data_start_offset, return_data_size)?;
        let new_memory_size = new_memory_size_for_args.max(new_memory_size_for_return_data);

        let (_account_info, address_was_cold) = self.access_account(code_address);

        self.increase_consumed_gas(
            current_call_frame,
            gas_cost::callcode(
                new_memory_size,
                current_memory_size,
                address_was_cold,
                value_to_transfer,
            )?,
        )?;

        // Sender and recipient are the same in this case. But the code executed is from another account.
        let msg_sender = current_call_frame.to;
        let to = current_call_frame.to;
        let is_static = current_call_frame.is_static;

        // We add the stipend gas for the subcall. This ensures that the callee has enough gas to perform basic operations
        let gas_for_subcall = if !value_to_transfer.is_zero() {
            gas.saturating_add(CALLCODE_POSITIVE_VALUE_STIPEND)
        } else {
            gas
        };

        self.generic_call(
            current_call_frame,
            gas_for_subcall,
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
        )
    }

    // RETURN operation
    pub fn op_return(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let offset = current_call_frame.stack.pop()?;
        let size = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;

        if size == 0 {
            return Ok(OpcodeSuccess::Result(ResultReason::Return));
        }

        let new_memory_size = calculate_memory_size(offset, size)?;
        self.increase_consumed_gas(
            current_call_frame,
            memory::expansion_cost(new_memory_size, current_call_frame.memory.len())?.into(),
        )?;

        current_call_frame.output =
            memory::load_range(&mut current_call_frame.memory, offset, size)?
                .to_vec()
                .into();

        Ok(OpcodeSuccess::Result(ResultReason::Return))
    }

    // DELEGATECALL operation
    // TODO: https://github.com/lambdaclass/ethrex/issues/1086
    pub fn op_delegatecall(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        // STACK
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

        // GAS
        let (_account_info, address_was_cold) = self.access_account(code_address);

        let current_memory_size = current_call_frame.memory.len();
        let new_memory_size_for_args = calculate_memory_size(args_start_offset, args_size)?;
        let new_memory_size_for_return_data =
            calculate_memory_size(return_data_start_offset, return_data_size)?;
        let new_memory_size = new_memory_size_for_args.max(new_memory_size_for_return_data);

        self.increase_consumed_gas(
            current_call_frame,
            gas_cost::delegatecall(new_memory_size, current_memory_size, address_was_cold)?,
        )?;

        // OPERATION
        let msg_sender = current_call_frame.msg_sender;
        let value = current_call_frame.msg_value;
        let to = current_call_frame.to;
        let is_static = current_call_frame.is_static;

        self.generic_call(
            current_call_frame,
            gas,
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
        )
    }

    // STATICCALL operation
    // TODO: https://github.com/lambdaclass/ethrex/issues/1086
    pub fn op_staticcall(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        // STACK
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

        // GAS
        let (_account_info, address_was_cold) = self.access_account(code_address);

        let current_memory_size = current_call_frame.memory.len();
        let new_memory_size_for_args = calculate_memory_size(args_start_offset, args_size)?;
        let new_memory_size_for_return_data =
            calculate_memory_size(return_data_start_offset, return_data_size)?;
        let new_memory_size = new_memory_size_for_args.max(new_memory_size_for_return_data);

        self.increase_consumed_gas(
            current_call_frame,
            gas_cost::staticcall(new_memory_size, current_memory_size, address_was_cold)?,
        )?;

        // OPERATION
        let value = U256::zero();
        let msg_sender = current_call_frame.to; // The new sender will be the current contract.
        let to = code_address; // In this case code_address and the sub-context account are the same. Unlike CALLCODE or DELEGATECODE.

        self.generic_call(
            current_call_frame,
            gas,
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
        )
    }

    // CREATE operation
    // TODO: https://github.com/lambdaclass/ethrex/issues/1086
    pub fn op_create(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let value_in_wei_to_send = current_call_frame.stack.pop()?;
        let code_offset_in_memory = current_call_frame.stack.pop()?;
        let code_size_in_memory: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;

        let new_size = calculate_memory_size(code_offset_in_memory, code_size_in_memory)?;

        self.increase_consumed_gas(
            current_call_frame,
            gas_cost::create(
                new_size,
                current_call_frame.memory.len(),
                code_size_in_memory,
            )?,
        )?;

        self.generic_create(
            value_in_wei_to_send,
            code_offset_in_memory,
            code_size_in_memory,
            None,
            current_call_frame,
        )
    }

    // CREATE2 operation
    // TODO: https://github.com/lambdaclass/ethrex/issues/1086
    pub fn op_create2(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let value_in_wei_to_send = current_call_frame.stack.pop()?;
        let code_offset_in_memory = current_call_frame.stack.pop()?;
        let code_size_in_memory: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;
        let salt = current_call_frame.stack.pop()?;

        let new_size = calculate_memory_size(code_offset_in_memory, code_size_in_memory)?;

        self.increase_consumed_gas(
            current_call_frame,
            gas_cost::create_2(
                new_size,
                current_call_frame.memory.len(),
                code_size_in_memory,
            )?,
        )?;

        self.generic_create(
            value_in_wei_to_send,
            code_offset_in_memory,
            code_size_in_memory,
            Some(salt),
            current_call_frame,
        )
    }

    // REVERT operation
    pub fn op_revert(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        // Description: Gets values from stack, calculates gas cost and sets return data.
        // Returns: VMError RevertOpcode if executed correctly.
        // Notes:
        //      The actual reversion of changes is made in the execute() function.

        let offset = current_call_frame.stack.pop()?;

        let size = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;

        let new_memory_size = calculate_memory_size(offset, size)?;
        self.increase_consumed_gas(
            current_call_frame,
            memory::expansion_cost(new_memory_size, current_call_frame.memory.len())?.into(),
        )?;

        current_call_frame.output =
            memory::load_range(&mut current_call_frame.memory, offset, size)?
                .to_vec()
                .into();

        Err(VMError::RevertOpcode)
    }

    /// ### INVALID operation
    /// Reverts consuming all gas, no return data.
    pub fn op_invalid(&mut self) -> Result<OpcodeSuccess, VMError> {
        Err(VMError::InvalidOpcode)
    }

    // SELFDESTRUCT operation
    pub fn op_selfdestruct(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
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
        if current_call_frame.is_static {
            return Err(VMError::OpcodeNotAllowedInStaticContext);
        }

        let target_address = word_to_address(current_call_frame.stack.pop()?);

        let (target_account_info, target_account_is_cold) = self.access_account(target_address);

        let (current_account_info, _current_account_is_cold) =
            self.access_account(current_call_frame.to);
        let balance_to_transfer = current_account_info.balance;

        self.increase_consumed_gas(
            current_call_frame,
            gas_cost::selfdestruct(
                target_account_is_cold,
                target_account_info.is_empty(),
                balance_to_transfer,
            )?,
        )?;

        self.increase_account_balance(target_address, balance_to_transfer)?;
        self.decrease_account_balance(current_call_frame.to, balance_to_transfer)?;

        if self
            .accrued_substate
            .created_accounts
            .contains(&current_call_frame.to)
        {
            self.accrued_substate
                .selfdestrutct_set
                .insert(current_call_frame.to);
        }

        Ok(OpcodeSuccess::Result(ResultReason::SelfDestruct))
    }

    /// Common behavior for CREATE and CREATE2 opcodes
    pub fn generic_create(
        &mut self,
        value_in_wei_to_send: U256,
        code_offset_in_memory: U256,
        code_size_in_memory: usize,
        salt: Option<U256>,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        // First: Validations that can cause out of gas.
        // 1. Cant be called in a static context
        if current_call_frame.is_static {
            return Err(VMError::OpcodeNotAllowedInStaticContext);
        }
        // 2. Cant exceed init code max size
        if code_size_in_memory > INIT_CODE_MAX_SIZE {
            return Err(VMError::OutOfGas(OutOfGasError::ConsumedGasOverflow));
        }

        // Reserve gas for subcall
        let max_message_call_gas = max_message_call_gas(current_call_frame)?;
        self.increase_consumed_gas(current_call_frame, max_message_call_gas.into())?;

        // Clear callframe subreturn data
        current_call_frame.sub_return_data = Bytes::new();

        let deployer_address = current_call_frame.to;

        let deployer_account_info = self.access_account(deployer_address).0;

        let code = Bytes::from(
            memory::load_range(
                &mut current_call_frame.memory,
                code_offset_in_memory,
                code_size_in_memory,
            )?
            .to_vec(),
        );

        let new_address = match salt {
            Some(salt) => Self::calculate_create2_address(deployer_address, &code, salt)?,
            None => Self::calculate_create_address(deployer_address, deployer_account_info.nonce)?,
        };

        // touch account
        self.accrued_substate.touched_accounts.insert(new_address);

        let new_depth = current_call_frame
            .depth
            .checked_add(1)
            .ok_or(InternalError::ArithmeticOperationOverflow)?;
        // SECOND: Validations that push 0 to the stack and return reserved_gas
        // 1. Sender doesn't have enough balance to send value.
        // 2. Depth limit has been reached
        // 3. Sender nonce is max.
        if deployer_account_info.balance < value_in_wei_to_send
            || new_depth > 1024
            || deployer_account_info.nonce == u64::MAX
        {
            // Return reserved gas
            current_call_frame.gas_used = current_call_frame
                .gas_used
                .checked_sub(max_message_call_gas.into())
                .ok_or(VMError::Internal(InternalError::GasOverflow))?;
            // Push 0
            current_call_frame.stack.push(CREATE_DEPLOYMENT_FAIL)?;
            return Ok(OpcodeSuccess::Continue);
        }

        // THIRD: Validations that push 0 to the stack without returning reserved gas but incrementing deployer's nonce
        let new_account = self.get_account(new_address);
        if new_account.has_code_or_nonce() {
            self.increment_account_nonce(deployer_address)?;
            current_call_frame.stack.push(CREATE_DEPLOYMENT_FAIL)?;
            return Ok(OpcodeSuccess::Continue);
        }

        // FOURTH: Changes to the state
        // 1. Creating contract.

        // If the address has balance but there is no account associated with it, we need to add the value to it
        let new_balance = value_in_wei_to_send
            .checked_add(new_account.info.balance)
            .ok_or(VMError::BalanceOverflow)?;

        let new_account = Account::new(new_balance, Bytes::new(), 1, Default::default());
        cache::insert_account(&mut self.cache, new_address, new_account);

        // 2. Increment sender's nonce.
        self.increment_account_nonce(deployer_address)?;

        // 3. Decrease sender's balance.
        self.decrease_account_balance(deployer_address, value_in_wei_to_send)?;

        let mut new_call_frame = CallFrame::new(
            deployer_address,
            new_address,
            new_address,
            code,
            value_in_wei_to_send,
            Bytes::new(),
            false,
            U256::from(max_message_call_gas),
            U256::zero(),
            new_depth,
            true,
        );

        self.accrued_substate.created_accounts.insert(new_address); // Mostly for SELFDESTRUCT during initcode.

        let tx_report = self.execute(&mut new_call_frame)?;
        let unused_gas = max_message_call_gas
            .checked_sub(tx_report.gas_used)
            .ok_or(InternalError::GasOverflow)?;

        // Return reserved gas
        current_call_frame.gas_used = current_call_frame
            .gas_used
            .checked_sub(unused_gas.into())
            .ok_or(InternalError::GasOverflow)?;
        current_call_frame.logs.extend(tx_report.logs);

        match tx_report.result {
            TxResult::Success => {
                current_call_frame
                    .stack
                    .push(address_to_word(new_address))?;
            }
            TxResult::Revert(err) => {
                // Return value to sender
                self.increase_account_balance(deployer_address, value_in_wei_to_send)?;

                // Deployment failed so account shouldn't exist
                cache::remove_account(&mut self.cache, &new_address);
                self.accrued_substate.created_accounts.remove(&new_address);

                // If revert we have to copy the return_data
                if err == VMError::RevertOpcode {
                    current_call_frame.sub_return_data = tx_report.output;
                }
                current_call_frame.stack.push(CREATE_DEPLOYMENT_FAIL)?;
            }
        }

        Ok(OpcodeSuccess::Continue)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn generic_call(
        &mut self,
        current_call_frame: &mut CallFrame,
        gas_limit: U256,
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
    ) -> Result<OpcodeSuccess, VMError> {
        // Clear callframe subreturn data
        current_call_frame.sub_return_data = Bytes::new();

        // 1. Validate sender has enough value
        let sender_account_info = self.access_account(msg_sender).0;
        if should_transfer_value && sender_account_info.balance < value {
            current_call_frame.stack.push(REVERT_FOR_CALL)?;
            return Ok(OpcodeSuccess::Continue);
        }

        // 2. Validate max depth has not been reached yet.
        let new_depth = current_call_frame
            .depth
            .checked_add(1)
            .ok_or(InternalError::ArithmeticOperationOverflow)?;

        if new_depth > 1024 {
            current_call_frame.stack.push(REVERT_FOR_CALL)?;
            return Ok(OpcodeSuccess::Continue);
        }

        let recipient_bytecode = self.access_account(code_address).0.bytecode;
        let calldata =
            memory::load_range(&mut current_call_frame.memory, args_offset, args_size)?.to_vec();
        // Gas Limit for the child context is capped.
        let gas_cap = max_message_call_gas(current_call_frame)?;
        let gas_limit = std::cmp::min(gas_limit, gas_cap.into());

        let mut new_call_frame = CallFrame::new(
            msg_sender,
            to,
            code_address,
            recipient_bytecode,
            value,
            calldata.into(),
            is_static,
            gas_limit,
            U256::zero(),
            new_depth,
            false,
        );

        // Transfer value from caller to callee.
        if should_transfer_value {
            self.decrease_account_balance(msg_sender, value)?;
            self.increase_account_balance(to, value)?;
        }

        let tx_report = self.execute(&mut new_call_frame)?;

        // Add gas used by the sub-context to the current one after it's execution.
        current_call_frame.gas_used = current_call_frame
            .gas_used
            .checked_add(tx_report.gas_used.into())
            .ok_or(VMError::OutOfGas(OutOfGasError::ConsumedGasOverflow))?;
        current_call_frame.logs.extend(tx_report.logs);
        memory::try_store_range(
            &mut current_call_frame.memory,
            ret_offset,
            ret_size,
            &tx_report.output,
        )?;
        current_call_frame.sub_return_data = tx_report.output;

        // What to do, depending on TxResult
        match tx_report.result {
            TxResult::Success => {
                current_call_frame.stack.push(SUCCESS_FOR_CALL)?;
            }
            TxResult::Revert(_) => {
                // Revert value transfer
                if should_transfer_value {
                    self.decrease_account_balance(to, value)?;
                    self.increase_account_balance(msg_sender, value)?;
                }
                // Push 0 to stack
                current_call_frame.stack.push(REVERT_FOR_CALL)?;
            }
        }

        Ok(OpcodeSuccess::Continue)
    }
}
