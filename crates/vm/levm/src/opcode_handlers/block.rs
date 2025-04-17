use crate::{
    call_frame::CallFrame,
    constants::LAST_AVAILABLE_BLOCK_LIMIT,
    errors::{InternalError, OpcodeResult, VMError},
    gas_cost,
    utils::*,
    vm::VM,
};
use ethrex_common::{types::Fork, U256};

// Block Information (11)
// Opcodes: BLOCKHASH, COINBASE, TIMESTAMP, NUMBER, PREVRANDAO, GASLIMIT, CHAINID, SELFBALANCE, BASEFEE, BLOBHASH, BLOBBASEFEE

impl<'a> VM<'a> {
    // BLOCKHASH operation
    pub fn op_blockhash(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeResult, VMError> {
        current_call_frame.increase_consumed_gas(gas_cost::BLOCKHASH)?;

        let block_number = current_call_frame.stack.pop()?;

        // If the block number is not valid, return zero
        if block_number
            < self
                .env
                .block_number
                .saturating_sub(LAST_AVAILABLE_BLOCK_LIMIT)
            || block_number >= self.env.block_number
        {
            current_call_frame.stack.push(U256::zero())?;
            return Ok(OpcodeResult::Continue { pc_increment: 1 });
        }

        let block_number: u64 = block_number
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;

        if let Some(block_hash) = self.db.store.get_block_hash(block_number)? {
            current_call_frame
                .stack
                .push(U256::from_big_endian(block_hash.as_bytes()))?;
        } else {
            current_call_frame.stack.push(U256::zero())?;
        }

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // COINBASE operation
    pub fn op_coinbase(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeResult, VMError> {
        current_call_frame.increase_consumed_gas(gas_cost::COINBASE)?;

        current_call_frame
            .stack
            .push(address_to_word(self.env.coinbase))?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // TIMESTAMP operation
    pub fn op_timestamp(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeResult, VMError> {
        current_call_frame.increase_consumed_gas(gas_cost::TIMESTAMP)?;

        current_call_frame.stack.push(self.env.timestamp)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // NUMBER operation
    pub fn op_number(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeResult, VMError> {
        current_call_frame.increase_consumed_gas(gas_cost::NUMBER)?;

        current_call_frame.stack.push(self.env.block_number)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // PREVRANDAO operation
    pub fn op_prevrandao(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeResult, VMError> {
        current_call_frame.increase_consumed_gas(gas_cost::PREVRANDAO)?;

        // https://eips.ethereum.org/EIPS/eip-4399
        // After Paris the prev randao is the prev_randao (or current_random) field
        let randao = if self.env.config.fork >= Fork::Paris {
            let randao = self.env.prev_randao.unwrap_or_default(); // Assuming prev_randao has been integrated
            U256::from_big_endian(randao.0.as_slice())
        } else {
            self.env.difficulty
        };
        current_call_frame.stack.push(randao)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // GASLIMIT operation
    pub fn op_gaslimit(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeResult, VMError> {
        current_call_frame.increase_consumed_gas(gas_cost::GASLIMIT)?;

        current_call_frame
            .stack
            .push(self.env.block_gas_limit.into())?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // CHAINID operation
    pub fn op_chainid(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeResult, VMError> {
        // https://eips.ethereum.org/EIPS/eip-1344
        if self.env.config.fork < Fork::Istanbul {
            return Err(VMError::InvalidOpcode);
        }
        current_call_frame.increase_consumed_gas(gas_cost::CHAINID)?;

        current_call_frame.stack.push(self.env.chain_id)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // SELFBALANCE operation
    pub fn op_selfbalance(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeResult, VMError> {
        // https://eips.ethereum.org/EIPS/eip-1884
        if self.env.config.fork < Fork::London {
            return Err(VMError::InvalidOpcode);
        }
        current_call_frame.increase_consumed_gas(gas_cost::SELFBALANCE)?;

        let balance = get_account(self.db, current_call_frame.to)?.info.balance;

        current_call_frame.stack.push(balance)?;
        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // BASEFEE operation
    pub fn op_basefee(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeResult, VMError> {
        // https://eips.ethereum.org/EIPS/eip-3198
        if self.env.config.fork < Fork::London {
            return Err(VMError::InvalidOpcode);
        }
        current_call_frame.increase_consumed_gas(gas_cost::BASEFEE)?;

        current_call_frame.stack.push(self.env.base_fee_per_gas)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // BLOBHASH operation
    /// Currently not tested
    pub fn op_blobhash(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeResult, VMError> {
        // [EIP-4844] - BLOBHASH is only available from CANCUN
        if self.env.config.fork < Fork::Cancun {
            return Err(VMError::InvalidOpcode);
        }

        current_call_frame.increase_consumed_gas(gas_cost::BLOBHASH)?;

        let index = current_call_frame.stack.pop()?;

        let blob_hashes = &self.env.tx_blob_hashes;
        if index >= blob_hashes.len().into() {
            current_call_frame.stack.push(U256::zero())?;
            return Ok(OpcodeResult::Continue { pc_increment: 1 });
        }

        let index: usize = index
            .try_into()
            .map_err(|_| VMError::Internal(InternalError::ConversionError))?;

        //This should never fail because we check if the index fits above
        let blob_hash = blob_hashes
            .get(index)
            .ok_or(VMError::Internal(InternalError::BlobHashOutOfRange))?;

        current_call_frame
            .stack
            .push(U256::from_big_endian(blob_hash.as_bytes()))?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // BLOBBASEFEE operation
    pub fn op_blobbasefee(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeResult, VMError> {
        // [EIP-7516] - BLOBBASEFEE is only available from CANCUN
        if self.env.config.fork < Fork::Cancun {
            return Err(VMError::InvalidOpcode);
        }
        current_call_frame.increase_consumed_gas(gas_cost::BLOBBASEFEE)?;

        let blob_base_fee =
            get_base_fee_per_blob_gas(self.env.block_excess_blob_gas, &self.env.config)?;

        current_call_frame.stack.push(blob_base_fee)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }
}
