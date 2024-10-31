use crate::{
    block::LAST_AVAILABLE_BLOCK_LIMIT,
    constants::{BLOB_BASE_FEE_UPDATE_FRACTION, MIN_BASE_FEE_PER_BLOB_GAS},
};
use keccak_hash::H256;

// Block Information (11)
// Opcodes: BLOCKHASH, COINBASE, TIMESTAMP, NUMBER, PREVRANDAO, GASLIMIT, CHAINID, SELFBALANCE, BASEFEE, BLOBHASH, BLOBBASEFEE
use super::*;

impl VM {
    // BLOCKHASH operation
    pub fn op_blockhash(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::BLOCKHASH)?;

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
            return Ok(OpcodeSuccess::Continue);
        }

        let block_number = block_number.as_u64();

        if let Some(block_hash) = self.db.get_block_hash(block_number) {
            current_call_frame
                .stack
                .push(U256::from_big_endian(block_hash.as_bytes()))?;
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
        self.increase_consumed_gas(current_call_frame, gas_cost::COINBASE)?;

        current_call_frame
            .stack
            .push(address_to_word(self.env.coinbase))?;

        Ok(OpcodeSuccess::Continue)
    }

    // TIMESTAMP operation
    pub fn op_timestamp(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::TIMESTAMP)?;

        current_call_frame.stack.push(self.env.timestamp)?;

        Ok(OpcodeSuccess::Continue)
    }

    // NUMBER operation
    pub fn op_number(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::NUMBER)?;

        current_call_frame.stack.push(self.env.block_number)?;

        Ok(OpcodeSuccess::Continue)
    }

    // PREVRANDAO operation
    pub fn op_prevrandao(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::PREVRANDAO)?;

        let randao = self.env.prev_randao.unwrap_or_default(); // Assuming block_env has been integrated
        current_call_frame
            .stack
            .push(U256::from_big_endian(randao.0.as_slice()))?;

        Ok(OpcodeSuccess::Continue)
    }

    // GASLIMIT operation
    pub fn op_gaslimit(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::GASLIMIT)?;

        current_call_frame.stack.push(self.env.gas_limit)?;

        Ok(OpcodeSuccess::Continue)
    }

    // CHAINID operation
    pub fn op_chainid(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::CHAINID)?;

        current_call_frame.stack.push(self.env.chain_id)?;

        Ok(OpcodeSuccess::Continue)
    }

    // SELFBALANCE operation
    pub fn op_selfbalance(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::SELFBALANCE)?;

        // the current account should have been cached when the contract was called
        let balance = self
            .cache
            .get_account(current_call_frame.code_address)
            .expect("The current account should always be cached")
            .info
            .balance;

        current_call_frame.stack.push(balance)?;
        Ok(OpcodeSuccess::Continue)
    }

    // BASEFEE operation
    pub fn op_basefee(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::BASEFEE)?;

        current_call_frame.stack.push(self.env.base_fee_per_gas)?;

        Ok(OpcodeSuccess::Continue)
    }

    // BLOBHASH operation
    /// Currently not tested
    pub fn op_blobhash(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::BLOBHASH)?;

        let index = current_call_frame.stack.pop()?.as_usize();

        let blob_hash: H256 = match &self.env.tx_blob_hashes {
            Some(vec) => match vec.get(index) {
                Some(el) => *el,
                None => {
                    return Err(VMError::BlobHashIndexOutOfBounds);
                }
            },
            None => {
                return Err(VMError::MissingBlobHashes);
            }
        };

        // Could not find a better way to translate from H256 to U256
        let u256_blob = U256::from(blob_hash.as_bytes());

        current_call_frame.stack.push(u256_blob)?;

        Ok(OpcodeSuccess::Continue)
    }

    fn get_blob_gasprice(&mut self) -> U256 {
        fake_exponential(
            MIN_BASE_FEE_PER_BLOB_GAS,
            // Use unwrap because env should have a Some value in excess_blob_gas attribute
            self.env.block_excess_blob_gas.unwrap(),
            BLOB_BASE_FEE_UPDATE_FRACTION,
        )
    }

    // BLOBBASEFEE operation
    pub fn op_blobbasefee(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::BLOBBASEFEE)?;

        let blob_base_fee = self.get_blob_gasprice();

        current_call_frame.stack.push(blob_base_fee)?;

        Ok(OpcodeSuccess::Continue)
    }
}

use std::str::FromStr;
fn address_to_word(address: Address) -> U256 {
    // This unwrap can't panic, as Address are 20 bytes long and U256 use 32 bytes
    U256::from_str(&format!("{address:?}")).unwrap()
}

// Fuction inspired in EIP 4844 helpers. Link: https://eips.ethereum.org/EIPS/eip-4844#helpers
fn fake_exponential(factor: U256, numerator: U256, denominator: U256) -> U256 {
    let mut i = U256::one();
    let mut output = U256::zero();
    let mut numerator_accum = factor * denominator;
    while numerator_accum > U256::zero() {
        output += numerator_accum;
        numerator_accum = (numerator_accum * numerator) / (denominator * i);
        i += U256::one();
    }
    output / denominator
}
