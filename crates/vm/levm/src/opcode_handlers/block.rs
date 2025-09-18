use crate::{
    constants::LAST_AVAILABLE_BLOCK_LIMIT,
    errors::{ExceptionalHalt, OpcodeResult, VMError},
    gas_cost,
    utils::*,
    vm::VM,
};
use ethrex_common::{U256, utils::u256_from_big_endian_const};

// Block Information (11)
// Opcodes: BLOCKHASH, COINBASE, TIMESTAMP, NUMBER, PREVRANDAO, GASLIMIT, CHAINID, SELFBALANCE, BASEFEE, BLOBHASH, BLOBBASEFEE

impl<'a> VM<'a> {
    // BLOCKHASH operation
    pub fn op_blockhash(&mut self) -> Result<OpcodeResult, VMError> {
        let current_block = self.env.block_number;
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::BLOCKHASH)?;

        let block_number = current_call_frame.stack.pop1()?;

        // If the block number is not valid, return zero
        if block_number < current_block.saturating_sub(LAST_AVAILABLE_BLOCK_LIMIT)
            || block_number >= current_block
        {
            current_call_frame.stack.push1(U256::zero())?;
            return Ok(OpcodeResult::Continue { pc_increment: 1 });
        }

        let block_number: u64 = block_number
            .try_into()
            .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;

        let block_hash = self.db.store.get_block_hash(block_number)?;
        self.current_call_frame
            .stack
            .push1(u256_from_big_endian_const(block_hash.to_fixed_bytes()))?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // COINBASE operation
    pub fn op_coinbase(&mut self) -> Result<OpcodeResult, VMError> {
        let coinbase = self.env.coinbase;
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::COINBASE)?;

        current_call_frame.stack.push1(address_to_word(coinbase))?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // TIMESTAMP operation
    pub fn op_timestamp(&mut self) -> Result<OpcodeResult, VMError> {
        let timestamp = self.env.timestamp;
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::TIMESTAMP)?;

        current_call_frame.stack.push1(timestamp)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // NUMBER operation
    pub fn op_number(&mut self) -> Result<OpcodeResult, VMError> {
        let block_number = self.env.block_number;
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::NUMBER)?;

        current_call_frame.stack.push1(block_number)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // PREVRANDAO operation
    pub fn op_prevrandao(&mut self) -> Result<OpcodeResult, VMError> {
        // https://eips.ethereum.org/EIPS/eip-4399
        // After Paris the prev randao is the prev_randao (or current_random) field
        let randao =
            u256_from_big_endian_const(self.env.prev_randao.unwrap_or_default().to_fixed_bytes());

        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::PREVRANDAO)?;
        current_call_frame.stack.push1(randao)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // GASLIMIT operation
    pub fn op_gaslimit(&mut self) -> Result<OpcodeResult, VMError> {
        let block_gas_limit = self.env.block_gas_limit;
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::GASLIMIT)?;

        current_call_frame.stack.push1(block_gas_limit.into())?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // CHAINID operation
    pub fn op_chainid(&mut self) -> Result<OpcodeResult, VMError> {
        let chain_id = self.env.chain_id;
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::CHAINID)?;

        current_call_frame.stack.push1(chain_id)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // SELFBALANCE operation
    pub fn op_selfbalance(&mut self) -> Result<OpcodeResult, VMError> {
        self.current_call_frame
            .increase_consumed_gas(gas_cost::SELFBALANCE)?;

        let balance = self
            .db
            .get_account(self.current_call_frame.to)?
            .info
            .balance;

        self.current_call_frame.stack.push1(balance)?;
        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // BASEFEE operation
    pub fn op_basefee(&mut self) -> Result<OpcodeResult, VMError> {
        // https://eips.ethereum.org/EIPS/eip-3198
        let base_fee_per_gas = self.env.base_fee_per_gas;
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::BASEFEE)?;

        current_call_frame.stack.push1(base_fee_per_gas)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // BLOBHASH operation
    /// Currently not tested
    pub fn op_blobhash(&mut self) -> Result<OpcodeResult, VMError> {
        self.current_call_frame
            .increase_consumed_gas(gas_cost::BLOBHASH)?;
        let index = self.current_call_frame.stack.pop1()?;
        let blob_hashes = &self.env.tx_blob_hashes;

        let index = match u256_to_usize(index) {
            Ok(index) if index < blob_hashes.len() => index,
            _ => {
                self.current_call_frame.stack.push1(U256::zero())?;
                return Ok(OpcodeResult::Continue { pc_increment: 1 });
            }
        };

        //This should never fail because we check if the index fits above
        #[expect(unsafe_code, reason = "bounds checked beforehand already")]
        let blob_hash = unsafe { blob_hashes.get_unchecked(index) };
        let hash = u256_from_big_endian_const(blob_hash.to_fixed_bytes());

        self.current_call_frame.stack.push1(hash)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // BLOBBASEFEE operation
    pub fn op_blobbasefee(&mut self) -> Result<OpcodeResult, VMError> {
        self.current_call_frame
            .increase_consumed_gas(gas_cost::BLOBBASEFEE)?;

        let blob_base_fee =
            get_base_fee_per_blob_gas(self.env.block_excess_blob_gas, &self.env.config)?;

        self.current_call_frame.stack.push1(blob_base_fee)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }
}
