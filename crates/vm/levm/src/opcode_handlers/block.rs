use crate::{
    block::LAST_AVAILABLE_BLOCK_LIMIT,
    constants::{call_opcode, WORD_SIZE},
    vm::word_to_address,
};
use sha3::{Digest, Keccak256};

// Block Information (11)
// Opcodes: BLOCKHASH, COINBASE, TIMESTAMP, NUMBER, PREVRANDAO, GASLIMIT, CHAINID, SELFBALANCE, BASEFEE, BLOBHASH, BLOBBASEFEE
use super::*;

impl VM {
    // BLOCKHASH operation
    pub fn op_blockhash(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        if self.env.consumed_gas + gas_cost::BLOCKHASH > self.env.gas_limit {
            return Err(VMError::OutOfGas);
        }

        let block_number = current_call_frame.stack.pop()?;
        self.env.consumed_gas += gas_cost::BLOCKHASH;

        // If the block number is not valid, return zero
        if block_number
            < self
                .env
                .block_number
                .saturating_sub(LAST_AVAILABLE_BLOCK_LIMIT)
            || block_number >= self.env.block_number
        {
            current_call_frame.stack.push(U256::zero())?;
            return Ok(OpcodeSuccess::Continue);
        }

        if let Some(block_hash) = self.db.block_hashes.get(&block_number) {
            current_call_frame
                .stack
                .push(U256::from_big_endian(&block_hash.0))?;
        } else {
            current_call_frame.stack.push(U256::zero())?;
        }

        Ok(OpcodeSuccess::Continue)
    }

    // COINBASE operation
    pub fn op_coinbase(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        if self.env.consumed_gas + gas_cost::COINBASE > self.env.gas_limit {
            return Err(VMError::OutOfGas);
        }
        current_call_frame
            .stack
            .push(address_to_word(self.env.coinbase))?;
        self.env.consumed_gas += gas_cost::COINBASE;

        Ok(OpcodeSuccess::Continue)
    }

    // TIMESTAMP operation
    pub fn op_timestamp(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        if self.env.consumed_gas + gas_cost::TIMESTAMP > self.env.gas_limit {
            return Err(VMError::OutOfGas);
        }
        current_call_frame.stack.push(self.env.timestamp)?;
        self.env.consumed_gas += gas_cost::TIMESTAMP;

        Ok(OpcodeSuccess::Continue)
    }

    // NUMBER operation
    pub fn op_number(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        if self.env.consumed_gas + gas_cost::NUMBER > self.env.gas_limit {
            return Err(VMError::OutOfGas);
        }

        current_call_frame.stack.push(self.env.block_number)?;
        self.env.consumed_gas += gas_cost::NUMBER;

        Ok(OpcodeSuccess::Continue)
    }

    // PREVRANDAO operation
    pub fn op_prevrandao(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        if self.env.consumed_gas + gas_cost::PREVRANDAO > self.env.gas_limit {
            return Err(VMError::OutOfGas);
        }
        let randao = self.env.prev_randao.unwrap_or_default(); // Assuming block_env has been integrated
        current_call_frame
            .stack
            .push(U256::from_big_endian(randao.0.as_slice()))?;
        self.env.consumed_gas += gas_cost::PREVRANDAO;

        Ok(OpcodeSuccess::Continue)
    }

    // GASLIMIT operation
    pub fn op_gaslimit(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        if self.env.consumed_gas + gas_cost::GASLIMIT > self.env.gas_limit {
            return Err(VMError::OutOfGas);
        }
        current_call_frame.stack.push(self.env.gas_limit)?;
        self.env.consumed_gas += gas_cost::GASLIMIT;

        Ok(OpcodeSuccess::Continue)
    }

    // CHAINID operation
    pub fn op_chainid(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        if self.env.consumed_gas + gas_cost::CHAINID > self.env.gas_limit {
            return Err(VMError::OutOfGas);
        }
        current_call_frame.stack.push(self.env.chain_id)?;
        self.env.consumed_gas += gas_cost::CHAINID;

        Ok(OpcodeSuccess::Continue)
    }

    // SELFBALANCE operation
    pub fn op_selfbalance(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        if self.env.consumed_gas + gas_cost::SELFBALANCE > self.env.gas_limit {
            return Err(VMError::OutOfGas);
        }

        let balance = self.db.balance(&current_call_frame.code_address);
        current_call_frame.stack.push(balance)?;

        self.env.consumed_gas += gas_cost::SELFBALANCE;

        Ok(OpcodeSuccess::Continue)
    }

    // BASEFEE operation
    pub fn op_basefee(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        if self.env.consumed_gas + gas_cost::BASEFEE > self.env.gas_limit {
            return Err(VMError::OutOfGas);
        }
        current_call_frame.stack.push(self.env.base_fee_per_gas)?;
        self.env.consumed_gas += gas_cost::BASEFEE;

        Ok(OpcodeSuccess::Continue)
    }

    // BLOBHASH operation
    pub fn op_blobhash(
        &mut self,
        _current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        if self.env.consumed_gas + gas_cost::BLOBHASH > self.env.gas_limit {
            return Err(VMError::OutOfGas);
        }

        self.env.consumed_gas += gas_cost::BLOBHASH;

        unimplemented!("when we have tx implemented");

        // Ok(OpcodeSuccess::Continue)
    }

    // BLOBBASEFEE operation
    pub fn op_blobbasefee(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        if self.env.consumed_gas + gas_cost::BLOBBASEFEE > self.env.gas_limit {
            return Err(VMError::OutOfGas);
        }
        // TODO: Calculate blob gas price.
        let blob_base_fee = U256::zero();
        current_call_frame.stack.push(blob_base_fee)?;
        self.env.consumed_gas += gas_cost::BLOBBASEFEE;

        Ok(OpcodeSuccess::Continue)
    }

    // ADDRESS operation
    pub fn op_address(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        if self.env.consumed_gas + gas_cost::ADDRESS > self.env.gas_limit {
            return Err(VMError::OutOfGas);
        }

        let addr = if current_call_frame.delegate.is_some() {
            current_call_frame.msg_sender
        } else {
            current_call_frame.code_address
        };

        current_call_frame.stack.push(U256::from(addr.as_bytes()))?;
        self.env.consumed_gas += gas_cost::ADDRESS;

        Ok(OpcodeSuccess::Continue)
    }

    // BALANCE operation
    pub fn op_balance(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        if self.env.consumed_gas + gas_cost::BALANCE > self.env.gas_limit {
            return Err(VMError::OutOfGas);
        }

        let addr = current_call_frame.stack.pop()?;

        let balance = self.db.balance(&word_to_address(addr));
        current_call_frame.stack.push(balance)?;

        self.env.consumed_gas += gas_cost::BALANCE;

        Ok(OpcodeSuccess::Continue)
    }

    // ORIGIN operation
    pub fn op_origin(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        if self.env.consumed_gas + gas_cost::ORIGIN > self.env.gas_limit {
            return Err(VMError::OutOfGas);
        }

        let origin = self.env.origin;
        current_call_frame
            .stack
            .push(U256::from(origin.as_bytes()))?;

        self.env.consumed_gas += gas_cost::ORIGIN;

        Ok(OpcodeSuccess::Continue)
    }

    // CALLER operation
    pub fn op_caller(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        if self.env.consumed_gas + gas_cost::CALLER > self.env.gas_limit {
            return Err(VMError::OutOfGas);
        }

        let caller = current_call_frame.msg_sender;
        current_call_frame
            .stack
            .push(U256::from(caller.as_bytes()))?;

        self.env.consumed_gas += gas_cost::CALLER;

        Ok(OpcodeSuccess::Continue)
    }

    // CALLVALUE operation
    pub fn op_callvalue(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        if self.env.consumed_gas + gas_cost::CALLVALUE > self.env.gas_limit {
            return Err(VMError::OutOfGas);
        }

        let callvalue = current_call_frame.msg_value;

        current_call_frame.stack.push(callvalue)?;

        self.env.consumed_gas += gas_cost::CALLVALUE;

        Ok(OpcodeSuccess::Continue)
    }

    // CODESIZE operation
    pub fn op_codesize(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        if self.env.consumed_gas + gas_cost::CODESIZE > self.env.gas_limit {
            return Err(VMError::OutOfGas);
        }

        current_call_frame
            .stack
            .push(U256::from(current_call_frame.bytecode.len()))?;

        self.env.consumed_gas += gas_cost::CODESIZE;

        Ok(OpcodeSuccess::Continue)
    }

    // CODECOPY operation
    pub fn op_codecopy(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let dest_offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;
        let offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;
        let size: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;

        let minimum_word_size = (size + WORD_SIZE - 1) / WORD_SIZE;

        let memory_expansion_cost = current_call_frame
            .memory
            .expansion_cost(dest_offset + size)?;

        let gas_cost = gas_cost::CODECOPY_STATIC
            + gas_cost::CODECOPY_DYNAMIC_BASE * minimum_word_size
            + memory_expansion_cost;

        if self.env.consumed_gas + gas_cost > self.env.gas_limit {
            return Err(VMError::OutOfGas);
        }

        let code = current_call_frame.bytecode.slice(offset..offset + size);

        current_call_frame.memory.store_bytes(dest_offset, &code);

        self.env.consumed_gas += gas_cost;

        Ok(OpcodeSuccess::Continue)
    }

    // GASPRICE operation
    pub fn op_gasprice(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        if self.env.consumed_gas + gas_cost::GASPRICE > self.env.gas_limit {
            return Err(VMError::OutOfGas);
        }

        current_call_frame.stack.push(self.env.gas_price)?;

        self.env.consumed_gas += gas_cost::GASPRICE;

        Ok(OpcodeSuccess::Continue)
    }

    // EXTCODESIZE operation
    pub fn op_extcodesize(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let address = word_to_address(current_call_frame.stack.pop()?);
        let gas_cost = if self.accrued_substate.warm_addresses.contains(&address) {
            call_opcode::WARM_ADDRESS_ACCESS_COST
        } else {
            call_opcode::COLD_ADDRESS_ACCESS_COST
        };
        if self.env.consumed_gas + gas_cost > self.env.gas_limit {
            return Err(VMError::OutOfGas);
        }
        let code_size = self.db.get_account_bytecode(&address).len();
        current_call_frame.stack.push(code_size.into())?;

        self.env.consumed_gas += gas_cost;
        Ok(OpcodeSuccess::Continue)
    }

    // EXTCODECOPY operation
    pub fn op_extcodecopy(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let address = word_to_address(current_call_frame.stack.pop()?);
        let dest_offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;
        let offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;
        let size: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;

        let minimum_word_size = (size + WORD_SIZE - 1) / WORD_SIZE;
        let memory_expansion_cost = current_call_frame
            .memory
            .expansion_cost(dest_offset + size)?;
        let address_access_cost = if self.accrued_substate.warm_addresses.contains(&address) {
            call_opcode::WARM_ADDRESS_ACCESS_COST
        } else {
            call_opcode::COLD_ADDRESS_ACCESS_COST
        };
        let gas_cost = gas_cost::EXTCODECOPY_DYNAMIC_BASE * minimum_word_size
            + memory_expansion_cost
            + address_access_cost;
        if self.env.consumed_gas + gas_cost > self.env.gas_limit {
            return Err(VMError::OutOfGas);
        }

        let mut code = self.db.get_account_bytecode(&address);
        if code.len() < offset + size {
            let mut extended_code = code.to_vec();
            extended_code.resize(offset + size, 0);
            code = Bytes::from(extended_code);
        }
        current_call_frame
            .memory
            .store_bytes(dest_offset, &code[offset..offset + size]);

        self.env.consumed_gas += gas_cost;
        Ok(OpcodeSuccess::Continue)
    }

    // EXTCODEHASH operation
    pub fn op_extcodehash(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let address = word_to_address(current_call_frame.stack.pop()?);
        let gas_cost = if self.accrued_substate.warm_addresses.contains(&address) {
            call_opcode::WARM_ADDRESS_ACCESS_COST
        } else {
            call_opcode::COLD_ADDRESS_ACCESS_COST
        };
        if self.env.consumed_gas + gas_cost > self.env.gas_limit {
            return Err(VMError::OutOfGas);
        }

        let code = self.db.get_account_bytecode(&address);
        let mut hasher = Keccak256::new();
        hasher.update(code);
        let result = hasher.finalize();
        current_call_frame
            .stack
            .push(U256::from_big_endian(&result))?;

        self.env.consumed_gas += gas_cost;
        Ok(OpcodeSuccess::Continue)
    }
}

use std::str::FromStr;
fn address_to_word(address: Address) -> U256 {
    // This unwrap can't panic, as Address are 20 bytes long and U256 use 32 bytes
    U256::from_str(&format!("{address:?}")).unwrap()
}
