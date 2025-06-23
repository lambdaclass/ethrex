use crate::{
    constants::*,
    errors::{ContextResult, ExceptionalHalt, InternalError, OpcodeResult, TxResult, VMError},
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
    ) -> Result<ContextResult, VMError> {
        match precompile_result {
            Ok(output) => Ok(ContextResult {
                result: TxResult::Success,
                gas_used: self.current_call_frame()?.gas_used,
                output,
            }),
            Err(error) => {
                if error.should_propagate() {
                    return Err(error);
                }

                Ok(ContextResult {
                    result: TxResult::Revert(error),
                    gas_used: self.current_call_frame()?.gas_limit,
                    output: Bytes::new(),
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
            Opcode::PUSH1 => self.op_push1(),
            Opcode::PUSH2 => self.op_push2(),
            // PUSHn
            op if (Opcode::PUSH3..=Opcode::PUSH32).contains(&op) => {
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

    pub fn handle_opcode_result(&mut self) -> Result<ContextResult, VMError> {
        // On successful create check output validity
        if self.is_create()? {
            let validate_create = self.validate_contract_creation();

            if let Err(error) = validate_create {
                if error.should_propagate() {
                    return Err(error);
                }

                // Consume all gas because error was exceptional.
                let callframe = self.current_call_frame_mut()?;
                callframe.gas_used = callframe.gas_limit;

                return Ok(ContextResult {
                    result: TxResult::Revert(error),
                    gas_used: callframe.gas_used,
                    output: Bytes::new(),
                });
            }

            // Set bytecode to the newly created contract.
            let contract_address = self.current_call_frame()?.to;
            let code = self.current_call_frame()?.output.clone();
            self.update_account_bytecode(contract_address, code)?;
        }

        Ok(ContextResult {
            result: TxResult::Success,
            gas_used: self.current_call_frame()?.gas_used,
            output: std::mem::take(&mut self.current_call_frame_mut()?.output),
        })
    }

    pub fn handle_opcode_error(&mut self, error: VMError) -> Result<ContextResult, VMError> {
        if error.should_propagate() {
            return Err(error);
        }

        let callframe = self.current_call_frame_mut()?;

        // Unless error is caused by Revert Opcode, consume all gas left.
        if !error.is_revert_opcode() {
            callframe.gas_used = callframe.gas_limit;
        }

        Ok(ContextResult {
            result: TxResult::Revert(error),
            gas_used: callframe.gas_used,
            output: std::mem::take(&mut callframe.output),
        })
    }

    /// Handles external create transaction.
    pub fn handle_create_transaction(&mut self) -> Result<Option<ContextResult>, VMError> {
        let new_contract_address = self.current_call_frame()?.to;
        let new_account = self.get_account_mut(new_contract_address)?;

        if new_account.has_code_or_nonce() {
            return Ok(Some(ContextResult {
                result: TxResult::Revert(ExceptionalHalt::AddressAlreadyOccupied.into()),
                gas_used: self.env.gas_limit,
                output: Bytes::new(),
            }));
        }

        self.increase_account_balance(new_contract_address, self.current_call_frame()?.msg_value)?;

        self.increment_account_nonce(new_contract_address)?;

        Ok(None)
    }

    /// Validates that the contract creation was successful, otherwise it returns an ExceptionalHalt.
    fn validate_contract_creation(&mut self) -> Result<(), VMError> {
        let callframe = self.current_call_frame_mut()?;
        let code = &callframe.output;

        let code_length: u64 = code
            .len()
            .try_into()
            .map_err(|_| InternalError::TypeConversion)?;

        let code_deposit_cost: u64 = code_length
            .checked_mul(CODE_DEPOSIT_COST)
            .ok_or(InternalError::Overflow)?;

        // Revert Scenarios
        // 1. If the first byte of code is 0xEF
        if code.first().is_some_and(|v| v == &EOF_PREFIX) {
            return Err(ExceptionalHalt::InvalidContractPrefix.into());
        }

        // 2. If the code_length > MAX_CODE_SIZE
        if code_length > MAX_CODE_SIZE {
            return Err(ExceptionalHalt::ContractOutputTooBig.into());
        }

        // 3. current_consumed_gas + code_deposit_cost > gas_limit
        callframe.increase_consumed_gas(code_deposit_cost)?;

        Ok(())
    }
}
