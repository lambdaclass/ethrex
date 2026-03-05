//! # Block operations
//!
//! Includes the following opcodes:
//!   - `BLOCKHASH`
//!   - `COINBASE`
//!   - `TIMESTAMP`
//!   - `NUMBER`
//!   - `PREVRANDAO`
//!   - `GASLIMIT`
//!   - `CHAINID`
//!   - `SELFBALANCE`
//!   - `BASEFEE`
//!   - `BLOBHASH`
//!   - `BLOBBASEFEE`

use std::mem;

use crate::{
    constants::LAST_AVAILABLE_BLOCK_LIMIT,
    errors::{OpcodeResult, VMError},
    gas_cost,
    opcode_handlers::OpcodeHandler,
    utils::*,
    vm::VM,
};
use ethrex_common::U256;

/// Implementation for the `BLOCKHASH` opcode.
pub struct OpBlockHashHandler;
impl OpcodeHandler for OpBlockHashHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::BLOCKHASH)?;

        // Some(_) if
        //   - is u64
        //   - 0 < current_number - block_number <= LAST_AVAILABLE_BLOCK_LIMIT
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "subtraction is guarded by take_if range check"
        )]
        if let Some(block_number) = u64::try_from(vm.current_call_frame.stack.pop1()?)
            .ok()
            .take_if(|&mut block_number| {
                block_number < vm.env.block_number
                    && vm.env.block_number - block_number <= LAST_AVAILABLE_BLOCK_LIMIT
            })
        {
            #[expect(unsafe_code, reason = "safe")]
            vm.current_call_frame.stack.push(unsafe {
                let mut bytes = vm.db.store.get_block_hash(block_number)?.0;
                bytes.reverse();
                U256(mem::transmute_copy::<[u8; 32], [u64; 4]>(&bytes))
            })?;
        } else {
            vm.current_call_frame.stack.push_zero()?;
        }

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `COINBASE` opcode.
pub struct OpCoinbaseHandler;
impl OpcodeHandler for OpCoinbaseHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::COINBASE)?;

        vm.current_call_frame
            .stack
            .push(address_to_word(vm.env.coinbase))?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `TIMESTAMP` opcode.
pub struct OpTimestampHandler;
impl OpcodeHandler for OpTimestampHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::TIMESTAMP)?;

        vm.current_call_frame.stack.push(vm.env.timestamp.into())?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `NUMBER` opcode.
pub struct OpNumberHandler;
impl OpcodeHandler for OpNumberHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::NUMBER)?;

        vm.current_call_frame
            .stack
            .push(vm.env.block_number.into())?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `PREVRANDAO` opcode.
pub struct OpPrevRandaoHandler;
impl OpcodeHandler for OpPrevRandaoHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::PREVRANDAO)?;

        // After Paris, `PREVRANDAO` is the prev_randao (or current_random) field.
        // Source: https://eips.ethereum.org/EIPS/eip-4399
        #[expect(unsafe_code, reason = "safe")]
        vm.current_call_frame.stack.push(U256(unsafe {
            let mut bytes = vm.env.prev_randao.unwrap_or_default().0;
            bytes.reverse();
            mem::transmute_copy::<[u8; 32], [u64; 4]>(&bytes)
        }))?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `GASLIMIT` opcode.
pub struct OpGasLimitHandler;
impl OpcodeHandler for OpGasLimitHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::GASLIMIT)?;

        vm.current_call_frame
            .stack
            .push(vm.env.block_gas_limit.into())?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `CHAINID` opcode.
pub struct OpChainIdHandler;
impl OpcodeHandler for OpChainIdHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::CHAINID)?;

        vm.current_call_frame.stack.push(vm.env.chain_id)?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `SELFBALANCE` opcode.
pub struct OpSelfBalanceHandler;
impl OpcodeHandler for OpSelfBalanceHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::SELFBALANCE)?;

        let address = vm.current_call_frame.to;
        let balance = vm.db.get_account(address)?.info.balance;

        // Record address touch for BAL per EIP-7928
        // SELFBALANCE has "Pre-state Cost: None" so always succeeds
        if let Some(recorder) = vm.db.bal_recorder.as_mut() {
            recorder.record_touched_address(address);
        }

        vm.current_call_frame.stack.push(balance)?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `BASEFEE` opcode.
pub struct OpBaseFeeHandler;
impl OpcodeHandler for OpBaseFeeHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::BASEFEE)?;

        // https://eips.ethereum.org/EIPS/eip-3198
        vm.current_call_frame.stack.push(vm.env.base_fee_per_gas)?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `BLOBHASH` opcode.
pub struct OpBlobHashHandler;
impl OpcodeHandler for OpBlobHashHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::BLOBHASH)?;

        match usize::try_from(vm.current_call_frame.stack.pop1()?)
            .ok()
            .and_then(|index| vm.env.tx_blob_hashes.get(index))
        {
            Some(hash) =>
            {
                #[expect(unsafe_code, reason = "safe")]
                vm.current_call_frame.stack.push(U256(unsafe {
                    let mut bytes = hash.0;
                    bytes.reverse();
                    mem::transmute_copy::<[u8; 32], [u64; 4]>(&bytes)
                }))?
            }
            None => vm.current_call_frame.stack.push_zero()?,
        }

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `BLOBBASEFEE` opcode.
pub struct OpBlobBaseFeeHandler;
impl OpcodeHandler for OpBlobBaseFeeHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::BLOBBASEFEE)?;

        vm.current_call_frame
            .stack
            .push(vm.env.base_blob_fee_per_gas)?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `SLOTNUM` opcode.
pub struct OpSlotNumHandler;
impl OpcodeHandler for OpSlotNumHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        // EIP-7843: Returns the slot number of the current block
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::SLOTNUM)?;

        vm.current_call_frame.stack.push(vm.env.slot_number)?;

        Ok(OpcodeResult::Continue)
    }
}
