use std::cell::OnceCell;

use crate::{
    constants::LAST_AVAILABLE_BLOCK_LIMIT,
    errors::{ExceptionalHalt, OpcodeResult, VMError},
    gas_cost,
    utils::*,
    vm::VM,
};
use ethrex_common::utils::u256_from_big_endian_const;

// Block Information (11)
// Opcodes: BLOCKHASH, COINBASE, TIMESTAMP, NUMBER, PREVRANDAO, GASLIMIT, CHAINID, SELFBALANCE, BASEFEE, BLOBHASH, BLOBBASEFEE

impl<'a> VM<'a> {
    // BLOCKHASH operation
    pub fn op_blockhash(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::BLOCKHASH)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let current_block = self.env.block_number;
        let block_number = match self.current_call_frame.stack.pop1() {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        // If the block number is not valid, return zero
        if block_number < current_block.saturating_sub(LAST_AVAILABLE_BLOCK_LIMIT)
            || block_number >= current_block
        {
            if let Err(err) = self.current_call_frame.stack.push_zero() {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
            return OpcodeResult::Continue;
        }

        let block_number: u64 = match block_number
            .try_into()
            .map_err(|_err| ExceptionalHalt::VeryLargeNumber)
        {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        let block_hash = match self.db.store.get_block_hash(block_number) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        if let Err(err) = self
            .current_call_frame
            .stack
            .push1(u256_from_big_endian_const(block_hash.to_fixed_bytes()))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // COINBASE operation
    pub fn op_coinbase(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        let coinbase = self.env.coinbase;
        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::COINBASE)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        if let Err(err) = self
            .current_call_frame
            .stack
            .push1(address_to_word(coinbase))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // TIMESTAMP operation
    pub fn op_timestamp(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        let timestamp = self.env.timestamp;
        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::TIMESTAMP)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        if let Err(err) = self.current_call_frame.stack.push1(timestamp) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // NUMBER operation
    pub fn op_number(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        let block_number = self.env.block_number;
        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::NUMBER)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        if let Err(err) = self.current_call_frame.stack.push1(block_number) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // PREVRANDAO operation
    pub fn op_prevrandao(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        // https://eips.ethereum.org/EIPS/eip-4399
        // After Paris the prev randao is the prev_randao (or current_random) field
        let randao =
            u256_from_big_endian_const(self.env.prev_randao.unwrap_or_default().to_fixed_bytes());

        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::PREVRANDAO)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        if let Err(err) = self.current_call_frame.stack.push1(randao) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // GASLIMIT operation
    pub fn op_gaslimit(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        let block_gas_limit = self.env.block_gas_limit;
        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::GASLIMIT)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        if let Err(err) = self.current_call_frame.stack.push1(block_gas_limit.into()) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // CHAINID operation
    pub fn op_chainid(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        let chain_id = self.env.chain_id;
        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::CHAINID)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        if let Err(err) = self.current_call_frame.stack.push1(chain_id) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // SELFBALANCE operation
    pub fn op_selfbalance(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::SELFBALANCE)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let balance = match self.db.get_account(self.current_call_frame.to) {
            Ok(x) => x.info.balance,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        if let Err(err) = self.current_call_frame.stack.push1(balance) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // BASEFEE operation
    pub fn op_basefee(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        // https://eips.ethereum.org/EIPS/eip-3198
        let base_fee_per_gas = self.env.base_fee_per_gas;
        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::BASEFEE)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        if let Err(err) = self.current_call_frame.stack.push1(base_fee_per_gas) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // BLOBHASH operation
    /// Currently not tested
    pub fn op_blobhash(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::BLOBHASH)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let index = match self.current_call_frame.stack.pop1() {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        let blob_hashes = &self.env.tx_blob_hashes;

        let index = match u256_to_usize(index) {
            Ok(index) if index < blob_hashes.len() => index,
            _ => {
                if let Err(err) = self.current_call_frame.stack.push_zero() {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
                return OpcodeResult::Continue;
            }
        };

        //This should never fail because we check if the index fits above
        #[expect(unsafe_code, reason = "bounds checked beforehand already")]
        let blob_hash = unsafe { blob_hashes.get_unchecked(index) };
        let hash = u256_from_big_endian_const(blob_hash.to_fixed_bytes());

        if let Err(err) = self.current_call_frame.stack.push1(hash) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // BLOBBASEFEE operation
    pub fn op_blobbasefee(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::BLOBBASEFEE)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let blob_base_fee =
            match get_base_fee_per_blob_gas(self.env.block_excess_blob_gas, &self.env.config) {
                Ok(x) => x,
                Err(err) => {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            };

        if let Err(err) = self.current_call_frame.stack.push1(blob_base_fee) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }
}
