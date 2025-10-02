use std::cell::OnceCell;

use crate::{
    call_frame::CallFrame,
    constants::{FAIL, INIT_CODE_MAX_SIZE, SUCCESS},
    errors::{ContextResult, ExceptionalHalt, InternalError, OpcodeResult, TxResult, VMError},
    gas_cost::{self, max_message_call_gas},
    memory::calculate_memory_size,
    precompiles,
    utils::{address_to_word, word_to_address, *},
    vm::VM,
};
use bytes::Bytes;
use ethrex_common::tracing::CallType::{
    self, CALL, CALLCODE, DELEGATECALL, SELFDESTRUCT, STATICCALL,
};
use ethrex_common::{Address, U256, evm::calculate_create_address, types::Fork};

// System Operations (10)
// Opcodes: CREATE, CALL, CALLCODE, RETURN, DELEGATECALL, CREATE2, STATICCALL, REVERT, INVALID, SELFDESTRUCT

impl<'a> VM<'a> {
    // CALL operation
    pub fn op_call(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        let (
            gas,
            callee,
            value,
            current_memory_size,
            args_offset,
            args_size,
            return_data_offset,
            return_data_size,
        ) = {
            let [
                gas,
                callee,
                value_to_transfer,
                args_offset,
                args_size,
                return_data_offset,
                return_data_size,
            ] = match self.current_call_frame.stack.pop() {
                Ok(x) => *x,
                Err(err) => {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            };
            let callee: Address = word_to_address(callee);
            let (args_size, args_offset) = match size_offset_to_usize(args_size, args_offset) {
                Ok(x) => x,
                Err(err) => {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            };
            let (return_data_size, return_data_offset) =
                match size_offset_to_usize(return_data_size, return_data_offset) {
                    Ok(x) => x,
                    Err(err) => {
                        error.set(err.into());
                        return OpcodeResult::Halt;
                    }
                };
            let current_memory_size = self.current_call_frame.memory.len();
            (
                gas,
                callee,
                value_to_transfer,
                current_memory_size,
                args_offset,
                args_size,
                return_data_offset,
                return_data_size,
            )
        };

        // VALIDATIONS
        if self.current_call_frame.is_static && !value.is_zero() {
            error.set(ExceptionalHalt::OpcodeNotAllowedInStaticContext.into());
            return OpcodeResult::Halt;
        }

        // CHECK EIP7702
        let (is_delegation_7702, eip7702_gas_consumed, code_address, bytecode) =
            match eip7702_get_code(self.db, &mut self.substate, callee) {
                Ok(x) => x,
                Err(err) => {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            };

        // GAS
        let (new_memory_size, gas_left, account_is_empty, address_was_cold) = match self
            .get_call_gas_params(
                args_offset,
                args_size,
                return_data_offset,
                return_data_size,
                eip7702_gas_consumed,
                callee,
            ) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        let (cost, gas_limit) = match gas_cost::call(
            new_memory_size,
            current_memory_size,
            address_was_cold,
            account_is_empty,
            value,
            gas,
            gas_left,
        ) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        if let Err(err) = cost
            .checked_add(eip7702_gas_consumed)
            .ok_or(ExceptionalHalt::OutOfGas)
            .and_then(|x| self.current_call_frame.increase_consumed_gas(x))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        // Make sure we have enough memory to write the return data
        // This is also needed to make sure we expand the memory even in cases where we don't have return data (such as transfers)
        if let Err(err) = self.current_call_frame.memory.resize(new_memory_size) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        // OPERATION
        let from = self.current_call_frame.to; // The new sender will be the current contract.
        let to = callee; // In this case code_address and the sub-context account are the same. Unlike CALLCODE or DELEGATECODE.
        let is_static = self.current_call_frame.is_static;
        let data = match self.get_calldata(args_offset, args_size) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

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
            return_data_offset,
            return_data_size,
            bytecode,
            is_delegation_7702,
            error,
        )
    }

    // CALLCODE operation
    pub fn op_callcode(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        // STACK
        let (
            gas,
            address,
            value,
            current_memory_size,
            args_offset,
            args_size,
            return_data_offset,
            return_data_size,
        ) = {
            let [
                gas,
                address,
                value_to_transfer,
                args_offset,
                args_size,
                return_data_offset,
                return_data_size,
            ] = match self.current_call_frame.stack.pop() {
                Ok(x) => *x,
                Err(err) => {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            };
            let address = word_to_address(address);
            let (args_size, args_offset) = match size_offset_to_usize(args_size, args_offset) {
                Ok(x) => x,
                Err(err) => {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            };
            let (return_data_size, return_data_offset) =
                match size_offset_to_usize(return_data_size, return_data_offset) {
                    Ok(x) => x,
                    Err(err) => {
                        error.set(err.into());
                        return OpcodeResult::Halt;
                    }
                };
            let current_memory_size = self.current_call_frame.memory.len();
            (
                gas,
                address,
                value_to_transfer,
                current_memory_size,
                args_offset,
                args_size,
                return_data_offset,
                return_data_size,
            )
        };

        // CHECK EIP7702
        let (is_delegation_7702, eip7702_gas_consumed, code_address, bytecode) =
            match eip7702_get_code(self.db, &mut self.substate, address) {
                Ok(x) => x,
                Err(err) => {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            };

        // GAS
        let (new_memory_size, gas_left, _account_is_empty, address_was_cold) = match self
            .get_call_gas_params(
                args_offset,
                args_size,
                return_data_offset,
                return_data_size,
                eip7702_gas_consumed,
                address,
            ) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        let (cost, gas_limit) = match gas_cost::callcode(
            new_memory_size,
            current_memory_size,
            address_was_cold,
            value,
            gas,
            gas_left,
        ) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        if let Err(err) = cost
            .checked_add(eip7702_gas_consumed)
            .ok_or(ExceptionalHalt::OutOfGas)
            .and_then(|x| self.current_call_frame.increase_consumed_gas(x))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        // Make sure we have enough memory to write the return data
        // This is also needed to make sure we expand the memory even in cases where we don't have return data (such as transfers)
        if let Err(err) = self.current_call_frame.memory.resize(new_memory_size) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        // Sender and recipient are the same in this case. But the code executed is from another account.
        let from = self.current_call_frame.to;
        let to = self.current_call_frame.to;
        let is_static = self.current_call_frame.is_static;
        let data = match self.get_calldata(args_offset, args_size) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

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
            return_data_offset,
            return_data_size,
            bytecode,
            is_delegation_7702,
            error,
        )
    }

    // RETURN operation
    pub fn op_return(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        let [offset, size] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        if size.is_zero() {
            return OpcodeResult::Halt;
        }

        let (size, offset) = match size_offset_to_usize(size, offset) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        let new_memory_size = match calculate_memory_size(offset, size) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        let current_memory_size = self.current_call_frame.memory.len();

        if let Err(err) = gas_cost::exit_opcode(new_memory_size, current_memory_size)
            .and_then(|x| Ok(self.current_call_frame.increase_consumed_gas(x)?))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        self.current_call_frame.output =
            match self.current_call_frame.memory.load_range(offset, size) {
                Ok(x) => x,
                Err(err) => {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            };

        OpcodeResult::Halt
    }

    // DELEGATECALL operation
    pub fn op_delegatecall(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        // STACK
        let (
            gas,
            address,
            current_memory_size,
            args_offset,
            args_size,
            return_data_offset,
            return_data_size,
        ) = {
            let [
                gas,
                address,
                args_offset,
                args_size,
                return_data_offset,
                return_data_size,
            ] = match self.current_call_frame.stack.pop() {
                Ok(x) => *x,
                Err(err) => {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            };
            let address = word_to_address(address);
            let (args_size, args_offset) = match size_offset_to_usize(args_size, args_offset) {
                Ok(x) => x,
                Err(err) => {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            };
            let (return_data_size, return_data_offset) =
                match size_offset_to_usize(return_data_size, return_data_offset) {
                    Ok(x) => x,
                    Err(err) => {
                        error.set(err.into());
                        return OpcodeResult::Halt;
                    }
                };
            let current_memory_size = self.current_call_frame.memory.len();
            (
                gas,
                address,
                current_memory_size,
                args_offset,
                args_size,
                return_data_offset,
                return_data_size,
            )
        };

        // CHECK EIP7702
        let (is_delegation_7702, eip7702_gas_consumed, code_address, bytecode) =
            match eip7702_get_code(self.db, &mut self.substate, address) {
                Ok(x) => x,
                Err(err) => {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            };

        // GAS
        let (new_memory_size, gas_left, _account_is_empty, address_was_cold) = match self
            .get_call_gas_params(
                args_offset,
                args_size,
                return_data_offset,
                return_data_size,
                eip7702_gas_consumed,
                address,
            ) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        let (cost, gas_limit) = match gas_cost::delegatecall(
            new_memory_size,
            current_memory_size,
            address_was_cold,
            gas,
            gas_left,
        ) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        if let Err(err) = cost
            .checked_add(eip7702_gas_consumed)
            .ok_or(ExceptionalHalt::OutOfGas)
            .and_then(|x| self.current_call_frame.increase_consumed_gas(x))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        // Make sure we have enough memory to write the return data
        // This is also needed to make sure we expand the memory even in cases where we don't have return data (such as transfers)
        if let Err(err) = self.current_call_frame.memory.resize(new_memory_size) {
            error.set(err.into());
            return OpcodeResult::Halt;
        };

        // OPERATION
        let from = self.current_call_frame.msg_sender;
        let value = self.current_call_frame.msg_value;
        let to = self.current_call_frame.to;
        let is_static = self.current_call_frame.is_static;
        let data = match self.get_calldata(args_offset, args_size) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        // In this trace the `from` is the current contract, we don't want the `from` to be, for example, the EOA that sent the transaction
        self.tracer
            .enter(DELEGATECALL, to, code_address, value, gas_limit, &data);

        self.generic_call(
            gas_limit,
            value,
            from,
            to,
            code_address,
            false,
            is_static,
            data,
            return_data_offset,
            return_data_size,
            bytecode,
            is_delegation_7702,
            error,
        )
    }

    // STATICCALL operation
    pub fn op_staticcall(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        // STACK
        let (
            gas,
            address,
            current_memory_size,
            args_offset,
            args_size,
            return_data_offset,
            return_data_size,
        ) = {
            let [
                gas,
                address,
                args_offset,
                args_size,
                return_data_offset,
                return_data_size,
            ] = match self.current_call_frame.stack.pop() {
                Ok(x) => *x,
                Err(err) => {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            };
            let address = word_to_address(address);
            let (args_size, args_offset) = match size_offset_to_usize(args_size, args_offset) {
                Ok(x) => x,
                Err(err) => {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            };
            let (return_data_size, return_data_offset) =
                match size_offset_to_usize(return_data_size, return_data_offset) {
                    Ok(x) => x,
                    Err(err) => {
                        error.set(err.into());
                        return OpcodeResult::Halt;
                    }
                };
            let current_memory_size = self.current_call_frame.memory.len();
            (
                gas,
                address,
                current_memory_size,
                args_offset,
                args_size,
                return_data_offset,
                return_data_size,
            )
        };

        // CHECK EIP7702
        let (is_delegation_7702, eip7702_gas_consumed, _, bytecode) =
            match eip7702_get_code(self.db, &mut self.substate, address) {
                Ok(x) => x,
                Err(err) => {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            };

        // GAS
        let (new_memory_size, gas_left, _account_is_empty, address_was_cold) = match self
            .get_call_gas_params(
                args_offset,
                args_size,
                return_data_offset,
                return_data_size,
                eip7702_gas_consumed,
                address,
            ) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        let (cost, gas_limit) = match gas_cost::staticcall(
            new_memory_size,
            current_memory_size,
            address_was_cold,
            gas,
            gas_left,
        ) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        if let Err(err) = cost
            .checked_add(eip7702_gas_consumed)
            .ok_or(ExceptionalHalt::OutOfGas)
            .and_then(|x| self.current_call_frame.increase_consumed_gas(x))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        // Make sure we have enough memory to write the return data
        // This is also needed to make sure we expand the memory even in cases where we don't have return data (such as transfers)
        if let Err(err) = self.current_call_frame.memory.resize(new_memory_size) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        // OPERATION
        let value = U256::zero();
        let from = self.current_call_frame.to; // The new sender will be the current contract.
        let to = address; // In this case address and the sub-context account are the same. Unlike CALLCODE or DELEGATECODE.
        let data = match self.get_calldata(args_offset, args_size) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

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
            return_data_offset,
            return_data_size,
            bytecode,
            is_delegation_7702,
            error,
        )
    }

    // CREATE operation
    pub fn op_create(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        let fork = self.env.config.fork;
        let [
            value_in_wei_to_send,
            code_offset_in_memory,
            code_size_in_memory,
        ] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        let (code_size_in_memory, code_offset_in_memory) =
            match size_offset_to_usize(code_size_in_memory, code_offset_in_memory) {
                Ok(x) => x,
                Err(err) => {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            };

        let new_size = match calculate_memory_size(code_offset_in_memory, code_size_in_memory) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        if let Err(err) = gas_cost::create(
            new_size,
            self.current_call_frame.memory.len(),
            code_size_in_memory,
            fork,
        )
        .and_then(|x| Ok(self.current_call_frame.increase_consumed_gas(x)?))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        self.generic_create(
            value_in_wei_to_send,
            code_offset_in_memory,
            code_size_in_memory,
            None,
            error,
        )
    }

    // CREATE2 operation
    pub fn op_create2(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        let fork = self.env.config.fork;
        let [
            value_in_wei_to_send,
            code_offset_in_memory,
            code_size_in_memory,
            salt,
        ] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        let (code_size_in_memory, code_offset_in_memory) =
            match size_offset_to_usize(code_size_in_memory, code_offset_in_memory) {
                Ok(x) => x,
                Err(err) => {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            };
        let new_size = match calculate_memory_size(code_offset_in_memory, code_size_in_memory) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        if let Err(err) = gas_cost::create_2(
            new_size,
            self.current_call_frame.memory.len(),
            code_size_in_memory,
            fork,
        )
        .and_then(|x| Ok(self.current_call_frame.increase_consumed_gas(x)?))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        self.generic_create(
            value_in_wei_to_send,
            code_offset_in_memory,
            code_size_in_memory,
            Some(salt),
            error,
        )
    }

    // REVERT operation
    pub fn op_revert(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        // Description: Gets values from stack, calculates gas cost and sets return data.
        // Returns: VMError RevertOpcode if executed correctly.
        // Notes:
        //      The actual reversion of changes is made in the execute() function.

        let [offset, size] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        let (size, offset) = match size_offset_to_usize(size, offset) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        let new_memory_size = match calculate_memory_size(offset, size) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        let current_memory_size = self.current_call_frame.memory.len();

        if let Err(err) = gas_cost::exit_opcode(new_memory_size, current_memory_size)
            .and_then(|x| Ok(self.current_call_frame.increase_consumed_gas(x)?))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        self.current_call_frame.output =
            match self.current_call_frame.memory.load_range(offset, size) {
                Ok(x) => x,
                Err(err) => {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            };

        error.set(VMError::RevertOpcode);
        OpcodeResult::Halt
    }

    /// ### INVALID operation
    /// Reverts consuming all gas, no return data.
    pub fn op_invalid(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        error.set(ExceptionalHalt::InvalidOpcode.into());
        OpcodeResult::Halt
    }

    // SELFDESTRUCT operation
    pub fn op_selfdestruct(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
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
                error.set(ExceptionalHalt::OpcodeNotAllowedInStaticContext.into());
                return OpcodeResult::Halt;
            }
            let target_address = match current_call_frame.stack.pop1().map(word_to_address) {
                Ok(x) => x,
                Err(err) => {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            };
            let to = current_call_frame.to;
            (target_address, to)
        };

        let target_account_is_cold = !self.substate.add_accessed_address(beneficiary);
        let target_account_is_empty = match self.db.get_account(beneficiary) {
            Ok(x) => x.is_empty(),
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        let current_account = match self.db.get_account(to) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        let balance = current_account.info.balance;

        if let Err(err) =
            gas_cost::selfdestruct(target_account_is_cold, target_account_is_empty, balance)
                .and_then(|x| Ok(self.current_call_frame.increase_consumed_gas(x)?))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        // [EIP-6780] - SELFDESTRUCT only in same transaction from CANCUN
        if self.env.config.fork >= Fork::Cancun {
            if let Err(err) = self.transfer(to, beneficiary, balance) {
                error.set(err.into());
                return OpcodeResult::Halt;
            }

            // Selfdestruct is executed in the same transaction as the contract was created
            if self.substate.is_account_created(&to) {
                // If target is the same as the contract calling, Ether will be burnt.
                match self.get_account_mut(to) {
                    Ok(x) => x.info.balance = U256::zero(),
                    Err(err) => {
                        error.set(err.into());
                        return OpcodeResult::Halt;
                    }
                }

                self.substate.add_selfdestruct(to);
            }
        } else {
            if let Err(err) = self.increase_account_balance(beneficiary, balance) {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
            match self.get_account_mut(to) {
                Ok(x) => x.info.balance = U256::zero(),
                Err(err) => {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            };

            self.substate.add_selfdestruct(to);
        }

        self.tracer
            .enter(SELFDESTRUCT, to, beneficiary, balance, 0, &Bytes::new());

        if let Err(err) = self.tracer.exit_early(0, None) {
            error.set(err.into());
            return OpcodeResult::Halt;
        };

        OpcodeResult::Halt
    }

    /// Common behavior for CREATE and CREATE2 opcodes
    pub fn generic_create(
        &mut self,
        value: U256,
        code_offset_in_memory: usize,
        code_size_in_memory: usize,
        salt: Option<U256>,
        error: &mut OnceCell<VMError>,
    ) -> OpcodeResult {
        // Validations that can cause out of gas.
        // 1. [EIP-3860] - Cant exceed init code max size
        if code_size_in_memory > INIT_CODE_MAX_SIZE && self.env.config.fork >= Fork::Shanghai {
            error.set(ExceptionalHalt::OutOfGas.into());
            return OpcodeResult::Halt;
        }

        // 2. CREATE can't be called in a static context
        if self.current_call_frame.is_static {
            error.set(ExceptionalHalt::OpcodeNotAllowedInStaticContext.into());
            return OpcodeResult::Halt;
        }

        // Clear callframe subreturn data
        self.current_call_frame.sub_return_data = Bytes::new();

        // Reserve gas for subcall
        let gas_limit = match max_message_call_gas(&self.current_call_frame) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        if let Err(err) = self.current_call_frame.increase_consumed_gas(gas_limit) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        // Load code from memory
        let code = match self
            .current_call_frame
            .memory
            .load_range(code_offset_in_memory, code_size_in_memory)
        {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        // Get account info of deployer
        let deployer = self.current_call_frame.to;
        let (deployer_balance, deployer_nonce) = {
            let deployer_account = match self.db.get_account(deployer) {
                Ok(x) => x,
                Err(err) => {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            };
            (deployer_account.info.balance, deployer_account.info.nonce)
        };

        // Calculate create address
        let new_address = match salt {
            Some(salt) => match calculate_create2_address(deployer, &code, salt) {
                Ok(x) => x,
                Err(err) => {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            },
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

        let new_depth = match self
            .current_call_frame
            .depth
            .checked_add(1)
            .ok_or(InternalError::Overflow)
        {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

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
                if let Err(err) = self.early_revert_message_call(gas_limit, reason.to_string()) {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                };
                return OpcodeResult::Continue;
            }
        }

        // Increment sender nonce (irreversible change)
        if let Err(err) = self.increment_account_nonce(deployer) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        // Deployment will fail (consuming all gas) if the contract already exists.
        let new_account = match self.get_account_mut(new_address) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        if new_account.has_code_or_nonce() {
            if let Err(err) = self.current_call_frame.stack.push1(FAIL) {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
            if let Err(err) = self
                .tracer
                .exit_early(gas_limit, Some("CreateAccExists".to_string()))
            {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
            return OpcodeResult::Continue;
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
        // 0 -> 1
        if let Err(err) = self.increment_account_nonce(new_address) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }
        if let Err(err) = self.transfer(deployer, new_address, value) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        self.substate.push_backup();
        self.substate.add_created_account(new_address); // Mostly for SELFDESTRUCT during initcode.

        OpcodeResult::Continue
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
        error: &mut OnceCell<VMError>,
    ) -> OpcodeResult {
        // Clear callframe subreturn data
        self.current_call_frame.sub_return_data.clear();

        // Validate sender has enough value
        if should_transfer_value && !value.is_zero() {
            let sender_balance = match self.db.get_account(msg_sender) {
                Ok(x) => x,
                Err(err) => {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            }
            .info
            .balance;
            if sender_balance < value {
                if let Err(err) = self.early_revert_message_call(gas_limit, "OutOfFund".to_string())
                {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
                return OpcodeResult::Continue;
            }
        }

        // Validate max depth has not been reached yet.
        let new_depth = match self
            .current_call_frame
            .depth
            .checked_add(1)
            .ok_or(InternalError::Overflow)
        {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        if new_depth > 1024 {
            if let Err(err) = self.early_revert_message_call(gas_limit, "MaxDepth".to_string()) {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
            return OpcodeResult::Continue;
        }

        if precompiles::is_precompile(&code_address, self.env.config.fork, self.vm_type)
            && !is_delegation_7702
        {
            let mut gas_remaining = gas_limit;
            let ctx_result = match Self::execute_precompile(
                code_address,
                &calldata,
                gas_limit,
                &mut gas_remaining,
                self.env.config.fork,
            ) {
                Ok(x) => x,
                Err(err) => {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            };

            // Return gas left from subcontext
            #[expect(clippy::as_conversions, reason = "remaining gas conversion")]
            if ctx_result.is_success() {
                self.current_call_frame.gas_remaining =
                    match (self.current_call_frame.gas_remaining as u64).checked_add(
                        match gas_limit
                            .checked_sub(ctx_result.gas_used)
                            .ok_or(InternalError::Underflow)
                        {
                            Ok(x) => x,
                            Err(err) => {
                                error.set(err.into());
                                return OpcodeResult::Halt;
                            }
                        },
                    ) {
                        Some(x) => x as i64,
                        None => {
                            error.set(InternalError::Overflow.into());
                            return OpcodeResult::Halt;
                        }
                    };
            }

            // Store return data of sub-context
            if let Err(err) = self.current_call_frame.memory.store_data(
                ret_offset,
                if ctx_result.output.len() >= ret_size {
                    match ctx_result
                        .output
                        .get(..ret_size)
                        .ok_or(ExceptionalHalt::OutOfBounds)
                    {
                        Ok(x) => x,
                        Err(err) => {
                            error.set(err.into());
                            return OpcodeResult::Halt;
                        }
                    }
                } else {
                    &ctx_result.output
                },
            ) {
                error.set(err.into());
                return OpcodeResult::Halt;
            };
            self.current_call_frame.sub_return_data = ctx_result.output.clone();

            // What to do, depending on TxResult
            if let Err(err) = self
                .current_call_frame
                .stack
                .push1(match &ctx_result.result {
                    TxResult::Success => SUCCESS,
                    TxResult::Revert(_) => FAIL,
                })
            {
                error.set(err.into());
                return OpcodeResult::Halt;
            }

            // Transfer value from caller to callee.
            if should_transfer_value && ctx_result.is_success() {
                if let Err(err) = self.transfer(msg_sender, to, value) {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                };
            }

            if let Err(err) = self.tracer.exit_context(&ctx_result, false) {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
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
                if let Err(err) = self.transfer(msg_sender, to, value) {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            }

            self.substate.push_backup();
        }

        OpcodeResult::Continue
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
    #[expect(clippy::as_conversions, reason = "remaining gas conversion")]
    fn get_call_gas_params(
        &mut self,
        args_offset: usize,
        args_size: usize,
        return_data_offset: usize,
        return_data_size: usize,
        eip7702_gas_consumed: u64,
        address: Address,
    ) -> Result<(usize, u64, bool, bool), VMError> {
        // Creation of previously empty accounts and cold addresses have higher gas cost
        let address_was_cold = !self.substate.add_accessed_address(address);
        let account_is_empty = self.db.get_account(address)?.is_empty();

        // Calculated here for memory expansion gas cost
        let new_memory_size_for_args = calculate_memory_size(args_offset, args_size)?;
        let new_memory_size_for_return_data =
            calculate_memory_size(return_data_offset, return_data_size)?;
        let new_memory_size = new_memory_size_for_args.max(new_memory_size_for_return_data);
        // Calculate remaining gas after EIP7702 consumption
        let gas_left = self
            .current_call_frame
            .gas_remaining
            .checked_sub(eip7702_gas_consumed as i64)
            .ok_or(ExceptionalHalt::OutOfGas)?;

        Ok((
            new_memory_size,
            gas_left as u64,
            account_is_empty,
            address_was_cold,
        ))
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
        callframe.stack.push1(FAIL)?; // It's the same as revert for CREATE

        self.tracer.exit_early(0, Some(reason))?;
        Ok(())
    }
}
