use crate::{
    call_frame::CallFrame,
    constants::*,
    db::CacheDB,
    errors::{InternalError, OpcodeResult, OutOfGasError, TransactionReport, TxResult, VMError},
    gas_cost::CODE_DEPOSIT_COST,
    opcodes::Opcode,
    utils::*,
    vm::{StateBackup, VM},
};

use bytes::Bytes;

impl VM {
    pub fn handle_precompile_result(
        &mut self,
        precompile_result: Result<Bytes, VMError>,
        current_call_frame: &mut CallFrame,
        backup: StateBackup,
    ) -> Result<TransactionReport, VMError> {
        match precompile_result {
            Ok(output) => {
                self.call_frames.push(current_call_frame.clone());

                Ok(TransactionReport {
                    result: TxResult::Success,
                    new_state: self.cache.clone(),
                    gas_used: current_call_frame.gas_used,
                    gas_refunded: 0,
                    output,
                    logs: std::mem::take(&mut current_call_frame.logs),
                })
            }
            Err(error) => {
                if error.is_internal() {
                    return Err(error);
                }

                self.call_frames.push(current_call_frame.clone());

                self.restore_state(backup);

                Ok(TransactionReport {
                    result: TxResult::Revert(error),
                    new_state: CacheDB::default(),
                    gas_used: current_call_frame.gas_limit,
                    gas_refunded: 0,
                    output: Bytes::new(),
                    logs: std::mem::take(&mut current_call_frame.logs),
                })
            }
        }
    }
    pub fn handle_current_opcode(
        &mut self,
        opcode: Opcode,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeResult, VMError> {
        match opcode {
            Opcode::STOP => Ok(OpcodeResult::Halt),
            Opcode::ADD => self.op_add(current_call_frame),
            Opcode::MUL => self.op_mul(current_call_frame),
            Opcode::SUB => self.op_sub(current_call_frame),
            Opcode::DIV => self.op_div(current_call_frame),
            Opcode::SDIV => self.op_sdiv(current_call_frame),
            Opcode::MOD => self.op_mod(current_call_frame),
            Opcode::SMOD => self.op_smod(current_call_frame),
            Opcode::ADDMOD => self.op_addmod(current_call_frame),
            Opcode::MULMOD => self.op_mulmod(current_call_frame),
            Opcode::EXP => self.op_exp(current_call_frame),
            Opcode::SIGNEXTEND => self.op_signextend(current_call_frame),
            Opcode::LT => self.op_lt(current_call_frame),
            Opcode::GT => self.op_gt(current_call_frame),
            Opcode::SLT => self.op_slt(current_call_frame),
            Opcode::SGT => self.op_sgt(current_call_frame),
            Opcode::EQ => self.op_eq(current_call_frame),
            Opcode::ISZERO => self.op_iszero(current_call_frame),
            Opcode::KECCAK256 => self.op_keccak256(current_call_frame),
            Opcode::CALLDATALOAD => self.op_calldataload(current_call_frame),
            Opcode::CALLDATASIZE => self.op_calldatasize(current_call_frame),
            Opcode::CALLDATACOPY => self.op_calldatacopy(current_call_frame),
            Opcode::RETURNDATASIZE => self.op_returndatasize(current_call_frame),
            Opcode::RETURNDATACOPY => self.op_returndatacopy(current_call_frame),
            Opcode::JUMP => self.op_jump(current_call_frame),
            Opcode::JUMPI => self.op_jumpi(current_call_frame),
            Opcode::JUMPDEST => self.op_jumpdest(current_call_frame),
            Opcode::PC => self.op_pc(current_call_frame),
            Opcode::BLOCKHASH => self.op_blockhash(current_call_frame),
            Opcode::COINBASE => self.op_coinbase(current_call_frame),
            Opcode::TIMESTAMP => self.op_timestamp(current_call_frame),
            Opcode::NUMBER => self.op_number(current_call_frame),
            Opcode::PREVRANDAO => self.op_prevrandao(current_call_frame),
            Opcode::GASLIMIT => self.op_gaslimit(current_call_frame),
            Opcode::CHAINID => self.op_chainid(current_call_frame),
            Opcode::BASEFEE => self.op_basefee(current_call_frame),
            Opcode::BLOBHASH => self.op_blobhash(current_call_frame),
            Opcode::BLOBBASEFEE => self.op_blobbasefee(current_call_frame),
            Opcode::PUSH0 => self.op_push0(current_call_frame),
            // PUSHn
            op if (Opcode::PUSH1..=Opcode::PUSH32).contains(&op) => {
                let n_bytes = get_n_value(op, Opcode::PUSH1)?;
                self.op_push(current_call_frame, n_bytes)
            }
            Opcode::AND => self.op_and(current_call_frame),
            Opcode::OR => self.op_or(current_call_frame),
            Opcode::XOR => self.op_xor(current_call_frame),
            Opcode::NOT => self.op_not(current_call_frame),
            Opcode::BYTE => self.op_byte(current_call_frame),
            Opcode::SHL => self.op_shl(current_call_frame),
            Opcode::SHR => self.op_shr(current_call_frame),
            Opcode::SAR => self.op_sar(current_call_frame),
            // DUPn
            op if (Opcode::DUP1..=Opcode::DUP16).contains(&op) => {
                let depth = get_n_value(op, Opcode::DUP1)?;
                self.op_dup(current_call_frame, depth)
            }
            // SWAPn
            op if (Opcode::SWAP1..=Opcode::SWAP16).contains(&op) => {
                let depth = get_n_value(op, Opcode::SWAP1)?;
                self.op_swap(current_call_frame, depth)
            }
            Opcode::POP => self.op_pop(current_call_frame),
            op if (Opcode::LOG0..=Opcode::LOG4).contains(&op) => {
                let number_of_topics = get_number_of_topics(op)?;
                self.op_log(current_call_frame, number_of_topics)
            }
            Opcode::MLOAD => self.op_mload(current_call_frame),
            Opcode::MSTORE => self.op_mstore(current_call_frame),
            Opcode::MSTORE8 => self.op_mstore8(current_call_frame),
            Opcode::SLOAD => self.op_sload(current_call_frame),
            Opcode::SSTORE => self.op_sstore(current_call_frame),
            Opcode::MSIZE => self.op_msize(current_call_frame),
            Opcode::GAS => self.op_gas(current_call_frame),
            Opcode::MCOPY => self.op_mcopy(current_call_frame),
            Opcode::CALL => self.op_call(current_call_frame),
            Opcode::CALLCODE => self.op_callcode(current_call_frame),
            Opcode::RETURN => self.op_return(current_call_frame),
            Opcode::DELEGATECALL => self.op_delegatecall(current_call_frame),
            Opcode::STATICCALL => self.op_staticcall(current_call_frame),
            Opcode::CREATE => self.op_create(current_call_frame),
            Opcode::CREATE2 => self.op_create2(current_call_frame),
            Opcode::TLOAD => self.op_tload(current_call_frame),
            Opcode::TSTORE => self.op_tstore(current_call_frame),
            Opcode::SELFBALANCE => self.op_selfbalance(current_call_frame),
            Opcode::ADDRESS => self.op_address(current_call_frame),
            Opcode::ORIGIN => self.op_origin(current_call_frame),
            Opcode::BALANCE => self.op_balance(current_call_frame),
            Opcode::CALLER => self.op_caller(current_call_frame),
            Opcode::CALLVALUE => self.op_callvalue(current_call_frame),
            Opcode::CODECOPY => self.op_codecopy(current_call_frame),
            Opcode::CODESIZE => self.op_codesize(current_call_frame),
            Opcode::GASPRICE => self.op_gasprice(current_call_frame),
            Opcode::EXTCODESIZE => self.op_extcodesize(current_call_frame),
            Opcode::EXTCODECOPY => self.op_extcodecopy(current_call_frame),
            Opcode::EXTCODEHASH => self.op_extcodehash(current_call_frame),
            Opcode::REVERT => self.op_revert(current_call_frame),
            Opcode::INVALID => self.op_invalid(),
            Opcode::SELFDESTRUCT => self.op_selfdestruct(current_call_frame),

            _ => Err(VMError::OpcodeNotFound),
        }
    }

    pub fn handle_opcode_result(
        &mut self,
        current_call_frame: &mut CallFrame,
        backup: StateBackup,
    ) -> Result<TransactionReport, VMError> {
        self.call_frames.push(current_call_frame.clone());
        // On successful create check output validity
        if (self.is_create() && current_call_frame.depth == 0)
            || current_call_frame.create_op_called
        {
            let contract_code = std::mem::take(&mut current_call_frame.output);
            let code_length = contract_code.len();

            let code_length_u64: u64 = code_length
                .try_into()
                .map_err(|_| VMError::Internal(InternalError::ConversionError))?;

            let code_deposit_cost: u64 =
                code_length_u64
                    .checked_mul(CODE_DEPOSIT_COST)
                    .ok_or(VMError::Internal(
                        InternalError::ArithmeticOperationOverflow,
                    ))?;

            // Revert
            // If the first byte of code is 0xef
            // If the code_length > MAX_CODE_SIZE
            // If current_consumed_gas + code_deposit_cost > gas_limit
            let validate_create = if code_length > MAX_CODE_SIZE {
                Err(VMError::ContractOutputTooBig)
            } else if contract_code.first().unwrap_or(&0) == &INVALID_CONTRACT_PREFIX {
                Err(VMError::InvalidContractPrefix)
            } else if self
                .increase_consumed_gas(current_call_frame, code_deposit_cost)
                .is_err()
            {
                Err(VMError::OutOfGas(OutOfGasError::MaxGasLimitExceeded))
            } else {
                Ok(current_call_frame.to)
            };

            match validate_create {
                Ok(new_address) => {
                    // Set bytecode to new account if success
                    update_account_bytecode(&mut self.cache, &self.db, new_address, contract_code)?;
                }
                Err(error) => {
                    // Revert if error
                    current_call_frame.gas_used = current_call_frame.gas_limit;
                    self.restore_state(backup);

                    return Ok(TransactionReport {
                        result: TxResult::Revert(error),
                        new_state: CacheDB::default(),
                        gas_used: current_call_frame.gas_used,
                        gas_refunded: self.env.refunded_gas,
                        output: std::mem::take(&mut current_call_frame.output),
                        logs: std::mem::take(&mut current_call_frame.logs),
                    });
                }
            }
        }

        Ok(TransactionReport {
            result: TxResult::Success,
            new_state: CacheDB::default(),
            gas_used: current_call_frame.gas_used,
            gas_refunded: self.env.refunded_gas,
            output: std::mem::take(&mut current_call_frame.output),
            logs: std::mem::take(&mut current_call_frame.logs),
        })
    }

    pub fn handle_opcode_error(
        &mut self,
        error: VMError,
        current_call_frame: &mut CallFrame,
        backup: StateBackup,
    ) -> Result<TransactionReport, VMError> {
        self.call_frames.push(current_call_frame.clone());

        if error.is_internal() {
            return Err(error);
        }

        // Unless error is from Revert opcode, all gas is consumed
        if error != VMError::RevertOpcode {
            let left_gas = current_call_frame
                .gas_limit
                .saturating_sub(current_call_frame.gas_used);
            current_call_frame.gas_used = current_call_frame.gas_used.saturating_add(left_gas);
        }

        self.restore_state(backup);

        Ok(TransactionReport {
            result: TxResult::Revert(error),
            new_state: CacheDB::default(),
            gas_used: current_call_frame.gas_used,
            gas_refunded: self.env.refunded_gas,
            output: std::mem::take(&mut current_call_frame.output), // Bytes::new() if error is not RevertOpcode
            logs: std::mem::take(&mut current_call_frame.logs),
        })
    }
}
