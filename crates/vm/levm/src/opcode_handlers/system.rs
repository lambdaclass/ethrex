use crate::{
    call_frame::CallFrame,
    constants::{CREATE_DEPLOYMENT_FAIL, INIT_CODE_MAX_SIZE},
    db::cache,
    errors::{InternalError, OpcodeSuccess, OutOfGasError, ResultReason, TxResult, VMError},
    gas_cost::{self, max_message_call_gas},
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
        let gas_for_call = current_call_frame.stack.pop()?;
        let callee: Address = word_to_address(current_call_frame.stack.pop()?);
        let value_to_transfer: U256 = current_call_frame.stack.pop()?;

        if current_call_frame.is_static && !value_to_transfer.is_zero() {
            return Err(VMError::OpcodeNotAllowedInStaticContext);
        }

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

        let msg_sender = current_call_frame.to; // The new sender will be the current contract.
        let to = callee; // In this case code_address and the sub-context account are the same. Unlike CALLCODE or DELEGATECODE.
        let is_static = current_call_frame.is_static;

        self.generic_call(
            current_call_frame,
            gas_for_call,
            value_to_transfer,
            msg_sender,
            to,
            callee,
            false,
            is_static,
            args_start_offset,
            args_size,
            return_data_start_offset,
            return_data_size,
            true,
        )
    }

    // CALLCODE operation
    // TODO: https://github.com/lambdaclass/ethrex/issues/1086
    pub fn op_callcode(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
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

        self.generic_call(
            current_call_frame,
            gas,
            value_to_transfer,
            msg_sender,
            to,
            code_address,
            false,
            is_static,
            args_start_offset,
            args_size,
            return_data_start_offset,
            return_data_size,
            true,
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

        current_call_frame.returndata =
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

        let msg_sender = current_call_frame.msg_sender;
        let value = current_call_frame.msg_value;
        let to = current_call_frame.to;
        let is_static = current_call_frame.is_static;

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
            false,
        )
    }

    // STATICCALL operation
    // TODO: https://github.com/lambdaclass/ethrex/issues/1086
    pub fn op_staticcall(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
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
            false,
            true,
            args_start_offset,
            args_size,
            return_data_start_offset,
            return_data_size,
            true,
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

        self.create(
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

        self.create(
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

        current_call_frame.returndata =
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
    ///
    /// Could be used for CREATE type transactions
    // TODO: Improve and test REVERT behavior for CREATE. Issue: https://github.com/lambdaclass/ethrex/issues/1061
    pub fn create(
        &mut self,
        value_in_wei_to_send: U256,
        code_offset_in_memory: U256,
        code_size_in_memory: usize,
        salt: Option<U256>,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        // 1. Cant be called in a static context
        if current_call_frame.is_static {
            return Err(VMError::OpcodeNotAllowedInStaticContext);
        }

        // 2. Cant exceed init code max size
        if code_size_in_memory > INIT_CODE_MAX_SIZE {
            return Err(VMError::OutOfGas(OutOfGasError::ConsumedGasOverflow));
        }
        let sender_address = current_call_frame.to;

        let (sender_account_info, _sender_address_was_cold) = self.access_account(sender_address);

        // Push 0 to stack if sender doesn't have enough value.
        if sender_account_info.balance < value_in_wei_to_send {
            current_call_frame
                .stack
                .push(U256::from(CREATE_DEPLOYMENT_FAIL))?;
            return Ok(OpcodeSuccess::Continue);
        }

        self.decrease_account_balance(sender_address, value_in_wei_to_send)?;

        let new_depth = current_call_frame
            .depth
            .checked_add(1)
            .ok_or(InternalError::ArithmeticOperationOverflow)?;

        if new_depth > 1024 {
            // Push 0 to stack if depth limit has been reached.
            current_call_frame
                .stack
                .push(U256::from(CREATE_DEPLOYMENT_FAIL))?;
            return Ok(OpcodeSuccess::Continue);
        }

        let code = Bytes::from(
            memory::load_range(
                &mut current_call_frame.memory,
                code_offset_in_memory,
                code_size_in_memory,
            )?
            .to_vec(),
        );

        let new_address = match salt {
            Some(salt) => Self::calculate_create2_address(sender_address, &code, salt)?,
            None => Self::calculate_create_address(sender_address, sender_account_info.nonce)?,
        };

        // Note that nonce is incremented AFTER calculating create address.
        if self.increment_account_nonce(sender_address).is_err() {
            // Push 0 to stack if sender has max nonce
            current_call_frame
                .stack
                .push(U256::from(CREATE_DEPLOYMENT_FAIL))?;
            return Ok(OpcodeSuccess::Continue);
        }

        if self.get_account(new_address).has_code_or_nonce() {
            // Push 0 to stack if account has code or nonce.
            current_call_frame
                .stack
                .push(U256::from(CREATE_DEPLOYMENT_FAIL))?;
            return Ok(OpcodeSuccess::Continue);
        }

        let new_account = Account::new(value_in_wei_to_send, code.clone(), 1, Default::default());
        cache::insert_account(&mut self.cache, new_address, new_account);

        let max_message_call_gas = max_message_call_gas(current_call_frame)?;

        let mut new_call_frame = CallFrame::new(
            sender_address,
            new_address,
            new_address,
            code,
            value_in_wei_to_send,
            Bytes::new(),
            false,
            max_message_call_gas,
            U256::zero(),
            new_depth,
        );

        self.accrued_substate.created_accounts.insert(new_address);
        let tx_report = self.execute(&mut new_call_frame)?;

        current_call_frame.gas_used = current_call_frame
            .gas_used
            .checked_add(tx_report.gas_used.into())
            .ok_or(VMError::OutOfGas(OutOfGasError::ConsumedGasOverflow))?;
        current_call_frame.logs.extend(tx_report.logs);

        match tx_report.result {
            TxResult::Success => {
                self.update_account_bytecode(new_address, tx_report.output)?;
                current_call_frame
                    .stack
                    .push(address_to_word(new_address))?;
            }
            TxResult::Revert(_) => {
                // Push 0 to stack
                current_call_frame
                    .stack
                    .push(U256::from(CREATE_DEPLOYMENT_FAIL))?;
            }
        }

        Ok(OpcodeSuccess::Continue)
    }
}
