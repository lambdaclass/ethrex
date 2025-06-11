use crate::{
    constants::*,
    errors::{ExceptionalHalt, ExecutionReport, InternalError, OpcodeResult, TxResult, VMError},
    gas_cost::CODE_DEPOSIT_COST,
    opcodes::Opcode,
    utils::*,
    vm::VM,
};

use bytes::Bytes;

impl<'a> VM<'a> {
    pub fn handle_precompile_result(
        &mut self,
        precompile_result: Result<Bytes, VMError>,
    ) -> Result<ExecutionReport, VMError> {
        match precompile_result {
            Ok(output) => Ok(ExecutionReport {
                result: TxResult::Success,
                gas_used: self.current_call_frame()?.gas_used,
                gas_refunded: self.substate.refunded_gas,
                output,
                logs: vec![],
            }),
            Err(error) => {
                if error.should_propagate() {
                    return Err(error);
                }

                Ok(ExecutionReport {
                    result: TxResult::Revert(error),
                    gas_used: self.current_call_frame()?.gas_limit,
                    gas_refunded: self.substate.refunded_gas,
                    output: Bytes::new(),
                    logs: vec![],
                })
            }
        }
    }

    pub fn execute_opcode(&mut self, opcode: Opcode) -> Result<OpcodeResult, VMError> {
        match opcode {
            Opcode::STOP => Ok(OpcodeResult::Halt),
            Opcode::ADD => self.op_add(),
            Opcode::MUL => self.op_mul(),
            Opcode::SUB => self.op_sub(),
            Opcode::DIV => self.op_div(),
            Opcode::SDIV => self.op_sdiv(),
            Opcode::MOD => self.op_mod(),
            Opcode::SMOD => self.op_smod(),
            Opcode::ADDMOD => self.op_addmod(),
            Opcode::MULMOD => self.op_mulmod(),
            Opcode::EXP => self.op_exp(),
            Opcode::SIGNEXTEND => self.op_signextend(),
            Opcode::LT => self.op_lt(),
            Opcode::GT => self.op_gt(),
            Opcode::SLT => self.op_slt(),
            Opcode::SGT => self.op_sgt(),
            Opcode::EQ => self.op_eq(),
            Opcode::ISZERO => self.op_iszero(),
            Opcode::KECCAK256 => self.op_keccak256(),
            Opcode::CALLDATALOAD => self.op_calldataload(),
            Opcode::CALLDATASIZE => self.op_calldatasize(),
            Opcode::CALLDATACOPY => self.op_calldatacopy(),
            Opcode::RETURNDATASIZE => self.op_returndatasize(),
            Opcode::RETURNDATACOPY => self.op_returndatacopy(),
            Opcode::JUMP => self.op_jump(),
            Opcode::JUMPI => self.op_jumpi(),
            Opcode::JUMPDEST => self.op_jumpdest(),
            Opcode::PC => self.op_pc(),
            Opcode::BLOCKHASH => self.op_blockhash(),
            Opcode::COINBASE => self.op_coinbase(),
            Opcode::TIMESTAMP => self.op_timestamp(),
            Opcode::NUMBER => self.op_number(),
            Opcode::PREVRANDAO => self.op_prevrandao(),
            Opcode::GASLIMIT => self.op_gaslimit(),
            Opcode::CHAINID => self.op_chainid(),
            Opcode::BASEFEE => self.op_basefee(),
            Opcode::BLOBHASH => self.op_blobhash(),
            Opcode::BLOBBASEFEE => self.op_blobbasefee(),
            Opcode::PUSH0 => self.op_push0(),
            // PUSHn
            op if (Opcode::PUSH1..=Opcode::PUSH32).contains(&op) => {
                let n_bytes = get_n_value(op, Opcode::PUSH1)?;
                self.op_push(n_bytes)
            }
            Opcode::AND => self.op_and(),
            Opcode::OR => self.op_or(),
            Opcode::XOR => self.op_xor(),
            Opcode::NOT => self.op_not(),
            Opcode::BYTE => self.op_byte(),
            Opcode::SHL => self.op_shl(),
            Opcode::SHR => self.op_shr(),
            Opcode::SAR => self.op_sar(),
            // DUPn
            op if (Opcode::DUP1..=Opcode::DUP16).contains(&op) => {
                let depth = get_n_value(op, Opcode::DUP1)?;
                self.op_dup(depth)
            }
            // SWAPn
            op if (Opcode::SWAP1..=Opcode::SWAP16).contains(&op) => {
                let depth = get_n_value(op, Opcode::SWAP1)?;
                self.op_swap(depth)
            }
            Opcode::POP => self.op_pop(),
            op if (Opcode::LOG0..=Opcode::LOG4).contains(&op) => {
                let number_of_topics = get_number_of_topics(op)?;
                self.op_log(number_of_topics)
            }
            Opcode::MLOAD => self.op_mload(),
            Opcode::MSTORE => self.op_mstore(),
            Opcode::MSTORE8 => self.op_mstore8(),
            Opcode::SLOAD => self.op_sload(),
            Opcode::SSTORE => self.op_sstore(),
            Opcode::MSIZE => self.op_msize(),
            Opcode::GAS => self.op_gas(),
            Opcode::MCOPY => self.op_mcopy(),
            Opcode::CALL => self.op_call(),
            Opcode::CALLCODE => self.op_callcode(),
            Opcode::RETURN => self.op_return(),
            Opcode::DELEGATECALL => self.op_delegatecall(),
            Opcode::STATICCALL => self.op_staticcall(),
            Opcode::CREATE => self.op_create(),
            Opcode::CREATE2 => self.op_create2(),
            Opcode::TLOAD => self.op_tload(),
            Opcode::TSTORE => self.op_tstore(),
            Opcode::SELFBALANCE => self.op_selfbalance(),
            Opcode::ADDRESS => self.op_address(),
            Opcode::ORIGIN => self.op_origin(),
            Opcode::BALANCE => self.op_balance(),
            Opcode::CALLER => self.op_caller(),
            Opcode::CALLVALUE => self.op_callvalue(),
            Opcode::CODECOPY => self.op_codecopy(),
            Opcode::CODESIZE => self.op_codesize(),
            Opcode::GASPRICE => self.op_gasprice(),
            Opcode::EXTCODESIZE => self.op_extcodesize(),
            Opcode::EXTCODECOPY => self.op_extcodecopy(),
            Opcode::EXTCODEHASH => self.op_extcodehash(),
            Opcode::REVERT => self.op_revert(),
            Opcode::INVALID => self.op_invalid(),
            Opcode::SELFDESTRUCT => self.op_selfdestruct(),

            _ => Err(ExceptionalHalt::InvalidOpcode.into()),
        }
    }

    pub fn handle_opcode_result(&mut self) -> Result<ExecutionReport, VMError> {
        // On successful create check output validity
        if (self.is_create() && self.current_call_frame()?.depth == 0)
            || self.current_call_frame()?.create_op_called
        {
            let contract_code = self.current_call_frame_mut()?.output.clone();
            let code_length = contract_code.len();

            let code_length_u64: u64 = code_length
                .try_into()
                .map_err(|_| InternalError::TypeConversion)?;

            let code_deposit_cost: u64 = code_length_u64
                .checked_mul(CODE_DEPOSIT_COST)
                .ok_or(InternalError::Overflow)?;

            // Revert
            // If the first byte of code is 0xef
            // If the code_length > MAX_CODE_SIZE
            // If current_consumed_gas + code_deposit_cost > gas_limit
            let validate_create = if code_length > MAX_CODE_SIZE {
                Err(ExceptionalHalt::ContractOutputTooBig)
            } else if contract_code
                .first()
                .is_some_and(|val| val == &INVALID_CONTRACT_PREFIX)
            {
                Err(ExceptionalHalt::InvalidContractPrefix)
            } else if self
                .current_call_frame_mut()?
                .increase_consumed_gas(code_deposit_cost)
                .is_err()
            {
                Err(ExceptionalHalt::OutOfGas)
            } else {
                Ok(self.current_call_frame()?.to)
            };

            match validate_create {
                Ok(new_address) => {
                    // Set bytecode to new account if success
                    self.update_account_bytecode(new_address, contract_code)?;
                }
                Err(error) => {
                    // Revert if error
                    self.current_call_frame_mut()?.gas_used = self.current_call_frame()?.gas_limit;
                    let gas_refunded = self
                        .substate_backups
                        .last()
                        .ok_or(InternalError::CallFrame)?
                        .refunded_gas;

                    return Ok(ExecutionReport {
                        result: TxResult::Revert(error.into()),
                        gas_used: self.current_call_frame()?.gas_used,
                        gas_refunded,
                        output: Bytes::new(),
                        logs: vec![],
                    });
                }
            }
        }

        Ok(ExecutionReport {
            result: TxResult::Success,
            gas_used: self.current_call_frame()?.gas_used,
            gas_refunded: self.substate.refunded_gas,
            output: std::mem::take(&mut self.current_call_frame_mut()?.output),
            logs: std::mem::take(&mut self.current_call_frame_mut()?.logs),
        })
    }

    pub fn handle_opcode_error(&mut self, error: VMError) -> Result<ExecutionReport, VMError> {
        if error.should_propagate() {
            return Err(error);
        }

        // Unless error is from Revert opcode, all gas is consumed
        if error != VMError::RevertOpcode {
            let left_gas = self
                .current_call_frame()?
                .gas_limit
                .saturating_sub(self.current_call_frame()?.gas_used);
            self.current_call_frame_mut()?.gas_used =
                self.current_call_frame()?.gas_used.saturating_add(left_gas);
        }

        let gas_refunded = self
            .substate_backups
            .last()
            .ok_or(InternalError::CallFrame)?
            .refunded_gas;
        let output = std::mem::take(&mut self.current_call_frame_mut()?.output); // Bytes::new() if error is not RevertOpcode
        let gas_used = self.current_call_frame()?.gas_used;

        Ok(ExecutionReport {
            result: TxResult::Revert(error),
            gas_used,
            gas_refunded,
            output,
            logs: vec![],
        })
    }

    pub fn handle_create_transaction(&mut self) -> Result<Option<ExecutionReport>, VMError> {
        let new_contract_address = self.current_call_frame()?.to;
        let new_account = self.get_account_mut(new_contract_address)?;

        if new_account.has_code_or_nonce() {
            return Ok(Some(ExecutionReport {
                result: TxResult::Revert(ExceptionalHalt::AddressAlreadyOccupied.into()),
                gas_used: self.env.gas_limit,
                gas_refunded: 0,
                logs: vec![],
                output: Bytes::new(),
            }));
        }

        self.increase_account_balance(new_contract_address, self.current_call_frame()?.msg_value)?;

        self.increment_account_nonce(new_contract_address)?;

        Ok(None)
    }
}
