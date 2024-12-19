use crate::{
    call_frame::CallFrame,
    constants::LAST_AVAILABLE_BLOCK_LIMIT,
    errors::{InternalError, OpcodeSuccess, VMError},
    gas_cost,
    vm::{address_to_word, VM},
};
use ethrex_core::{
    types::{BLOB_BASE_FEE_UPDATE_FRACTION, MIN_BASE_FEE_PER_BLOB_GAS},
    U256,
};

// Block Information (11)
// Opcodes: BLOCKHASH, COINBASE, TIMESTAMP, NUMBER, PREVRANDAO, GASLIMIT, CHAINID, SELFBALANCE, BASEFEE, BLOBHASH, BLOBBASEFEE

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

        let block_number: u64 = block_number
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;

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

        current_call_frame.stack.push(self.env.block_gas_limit)?;

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

        let balance = self.get_account(current_call_frame.to).info.balance;

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

        let index = current_call_frame.stack.pop()?;

        let blob_hashes = &self.env.tx_blob_hashes;
        if index > blob_hashes.len().into() {
            current_call_frame.stack.push(U256::zero())?;
            return Ok(OpcodeSuccess::Continue);
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

        Ok(OpcodeSuccess::Continue)
    }

    fn get_blob_gasprice(&mut self) -> Result<U256, VMError> {
        fake_exponential(
            MIN_BASE_FEE_PER_BLOB_GAS.into(),
            // Use unwrap because env should have a Some value in excess_blob_gas attribute
            self.env.block_excess_blob_gas.ok_or(VMError::Internal(
                InternalError::ExcessBlobGasShouldNotBeNone,
            ))?,
            BLOB_BASE_FEE_UPDATE_FRACTION.into(),
        )
    }

    // BLOBBASEFEE operation
    pub fn op_blobbasefee(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::BLOBBASEFEE)?;

        let blob_base_fee = self.get_blob_gasprice()?;

        current_call_frame.stack.push(blob_base_fee)?;

        Ok(OpcodeSuccess::Continue)
    }
}

// Fuction inspired in EIP 4844 helpers. Link: https://eips.ethereum.org/EIPS/eip-4844#helpers
fn fake_exponential(factor: U256, numerator: U256, denominator: U256) -> Result<U256, VMError> {
    let mut i = U256::one();
    let mut output = U256::zero();
    let mut numerator_accum = factor.checked_mul(denominator).ok_or(VMError::Internal(
        InternalError::ArithmeticOperationOverflow,
    ))?;
    while numerator_accum > U256::zero() {
        output = output
            .checked_add(numerator_accum)
            .ok_or(VMError::Internal(
                InternalError::ArithmeticOperationOverflow,
            ))?;
        let mult_numerator = numerator_accum
            .checked_mul(numerator)
            .ok_or(VMError::Internal(
                InternalError::ArithmeticOperationOverflow,
            ))?;
        let mult_denominator = denominator.checked_mul(i).ok_or(VMError::Internal(
            InternalError::ArithmeticOperationOverflow,
        ))?;
        numerator_accum =
            (mult_numerator)
                .checked_div(mult_denominator)
                .ok_or(VMError::Internal(
                    InternalError::ArithmeticOperationDividedByZero,
                ))?; // Neither denominator or i can be zero
        i = i.checked_add(U256::one()).ok_or(VMError::Internal(
            InternalError::ArithmeticOperationOverflow,
        ))?;
    }
    output.checked_div(denominator).ok_or(VMError::Internal(
        InternalError::ArithmeticOperationDividedByZero,
    )) // Denominator is a const
}
