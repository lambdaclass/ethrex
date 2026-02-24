use crate::{
    constants::LAST_AVAILABLE_BLOCK_LIMIT,
    errors::{ExceptionalHalt, OpcodeResult, VMError},
    utils::*,
    vm::VM,
};
use ethrex_common::utils::u256_from_big_endian_const;

// Block Information (12)
// Opcodes: BLOCKHASH, COINBASE, TIMESTAMP, NUMBER, PREVRANDAO, GASLIMIT, CHAINID, SELFBALANCE, BASEFEE, BLOBHASH, BLOBBASEFEE, SLOTNUM

impl<'a> VM<'a> {
    // BLOCKHASH operation
    pub fn op_blockhash(&mut self) -> Result<OpcodeResult, VMError> {
        let current_block = self.env.block_number;
        let current_call_frame = &mut self.current_call_frame;
        let block_number = current_call_frame.stack.pop1()?;

        // If the block number is not valid, return zero
        if block_number < current_block.saturating_sub(LAST_AVAILABLE_BLOCK_LIMIT)
            || block_number >= current_block
        {
            current_call_frame.stack.push_zero()?;
            return Ok(OpcodeResult::Continue);
        }

        let block_number: u64 = block_number
            .try_into()
            .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;

        let block_hash = self.db.store.get_block_hash(block_number)?;
        self.current_call_frame
            .stack
            .push(u256_from_big_endian_const(block_hash.to_fixed_bytes()))?;

        Ok(OpcodeResult::Continue)
    }

    // COINBASE operation
    pub fn op_coinbase(&mut self) -> Result<OpcodeResult, VMError> {
        let coinbase = self.env.coinbase;
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.stack.push(address_to_word(coinbase))?;

        Ok(OpcodeResult::Continue)
    }

    // TIMESTAMP operation
    pub fn op_timestamp(&mut self) -> Result<OpcodeResult, VMError> {
        let timestamp = self.env.timestamp;
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.stack.push(timestamp)?;

        Ok(OpcodeResult::Continue)
    }

    // NUMBER operation
    pub fn op_number(&mut self) -> Result<OpcodeResult, VMError> {
        let block_number = self.env.block_number;
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.stack.push(block_number)?;

        Ok(OpcodeResult::Continue)
    }

    // PREVRANDAO operation
    pub fn op_prevrandao(&mut self) -> Result<OpcodeResult, VMError> {
        // https://eips.ethereum.org/EIPS/eip-4399
        // After Paris the prev randao is the prev_randao (or current_random) field
        let randao =
            u256_from_big_endian_const(self.env.prev_randao.unwrap_or_default().to_fixed_bytes());

        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.stack.push(randao)?;

        Ok(OpcodeResult::Continue)
    }

    // GASLIMIT operation
    pub fn op_gaslimit(&mut self) -> Result<OpcodeResult, VMError> {
        let block_gas_limit = self.env.block_gas_limit;
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.stack.push(block_gas_limit.into())?;

        Ok(OpcodeResult::Continue)
    }

    // CHAINID operation
    pub fn op_chainid(&mut self) -> Result<OpcodeResult, VMError> {
        let chain_id = self.env.chain_id;
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.stack.push(chain_id)?;

        Ok(OpcodeResult::Continue)
    }

    // SELFBALANCE operation
    pub fn op_selfbalance(&mut self) -> Result<OpcodeResult, VMError> {
        let address = self.current_call_frame.to;
        let balance = self.db.get_account(address)?.info.balance;

        // Record address touch for BAL per EIP-7928
        // SELFBALANCE has "Pre-state Cost: None" so always succeeds
        if let Some(recorder) = self.db.bal_recorder.as_mut() {
            recorder.record_touched_address(address);
        }

        self.current_call_frame.stack.push(balance)?;
        Ok(OpcodeResult::Continue)
    }

    // BASEFEE operation
    pub fn op_basefee(&mut self) -> Result<OpcodeResult, VMError> {
        // https://eips.ethereum.org/EIPS/eip-3198
        let base_fee_per_gas = self.env.base_fee_per_gas;
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.stack.push(base_fee_per_gas)?;

        Ok(OpcodeResult::Continue)
    }

    // BLOBHASH operation
    /// Currently not tested
    pub fn op_blobhash(&mut self) -> Result<OpcodeResult, VMError> {
        let index = self.current_call_frame.stack.pop1()?;
        let blob_hashes = &self.env.tx_blob_hashes;

        let index = match u256_to_usize(index) {
            Ok(index) if index < blob_hashes.len() => index,
            _ => {
                self.current_call_frame.stack.push_zero()?;
                return Ok(OpcodeResult::Continue);
            }
        };

        //This should never fail because we check if the index fits above
        #[expect(unsafe_code, reason = "bounds checked beforehand already")]
        let blob_hash = unsafe { blob_hashes.get_unchecked(index) };
        let hash = u256_from_big_endian_const(blob_hash.to_fixed_bytes());

        self.current_call_frame.stack.push(hash)?;

        Ok(OpcodeResult::Continue)
    }

    // BLOBBASEFEE operation
    pub fn op_blobbasefee(&mut self) -> Result<OpcodeResult, VMError> {
        self.current_call_frame
            .stack
            .push(self.env.base_blob_fee_per_gas)?;

        Ok(OpcodeResult::Continue)
    }

    // SLOTNUM operation
    pub fn op_slotnum(&mut self) -> Result<OpcodeResult, VMError> {
        // EIP-7843: Returns the slot number of the current block
        self.current_call_frame.stack.push(self.env.slot_number)?;

        Ok(OpcodeResult::Continue)
    }
}
