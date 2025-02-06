use crate::{
    account::{Account, StorageSlot},
    call_frame::CallFrame,
    constants::*,
    db::{
        cache::{self, get_account_mut, remove_account},
        CacheDB, Database,
    },
    environment::Environment,
    errors::{ExecutionReport, InternalError, OpcodeResult, TxResult, TxValidationError, VMError},
    gas_cost::{self, STANDARD_TOKEN_COST, TOTAL_COST_FLOOR_PER_TOKEN},
    precompiles::{
        execute_precompile, is_precompile, SIZE_PRECOMPILES_CANCUN, SIZE_PRECOMPILES_PRAGUE,
        SIZE_PRECOMPILES_PRE_CANCUN,
    },
    utils::*,
    vm::VM,
    AccountInfo, TransientStorage,
};

pub trait Hook {
    fn prepare_execution(
        &self,
        vm: &mut VM,
        initial_call_frame: &mut CallFrame,
    ) -> Result<(), VMError>;

    fn finalize_execution(&self) -> Result<(), VMError>;
}

pub struct DefaultHook {}

impl DefaultHook {
    pub fn new() -> DefaultHook {
        DefaultHook {}
    }
}

impl Hook for DefaultHook {
    fn prepare_execution(
        &self,
        vm: &mut VM,
        initial_call_frame: &mut CallFrame,
    ) -> Result<(), VMError> {
        todo!();
        // let sender_address = self.env.origin;
        // let sender_account = get_account(&mut self.cache, &self.db, sender_address);

        // if self.env.config.fork >= Fork::Prague {
        //     // check for gas limit is grater or equal than the minimum required
        //     let intrinsic_gas: u64 = get_intrinsic_gas(
        //         self.is_create(),
        //         self.env.config.fork,
        //         &self.access_list,
        //         &self.authorization_list,
        //         initial_call_frame,
        //     )?;

        //     // calldata_cost = tokens_in_calldata * 4
        //     let calldata_cost: u64 =
        //         gas_cost::tx_calldata(&initial_call_frame.calldata, self.env.config.fork)
        //             .map_err(VMError::OutOfGas)?;

        //     // same as calculated in gas_used()
        //     let tokens_in_calldata: u64 = calldata_cost
        //         .checked_div(STANDARD_TOKEN_COST)
        //         .ok_or(VMError::Internal(InternalError::DivisionError))?;

        //     // floor_cost_by_tokens = TX_BASE_COST + TOTAL_COST_FLOOR_PER_TOKEN * tokens_in_calldata
        //     let floor_cost_by_tokens = tokens_in_calldata
        //         .checked_mul(TOTAL_COST_FLOOR_PER_TOKEN)
        //         .ok_or(VMError::Internal(InternalError::GasOverflow))?
        //         .checked_add(TX_BASE_COST)
        //         .ok_or(VMError::Internal(InternalError::GasOverflow))?;

        //     let min_gas_limit = max(intrinsic_gas, floor_cost_by_tokens);

        //     if initial_call_frame.gas_limit < min_gas_limit {
        //         return Err(VMError::TxValidation(TxValidationError::IntrinsicGasTooLow));
        //     }
        // }

        // // (1) GASLIMIT_PRICE_PRODUCT_OVERFLOW
        // let gaslimit_price_product = self
        //     .env
        //     .gas_price
        //     .checked_mul(self.env.gas_limit.into())
        //     .ok_or(VMError::TxValidation(
        //         TxValidationError::GasLimitPriceProductOverflow,
        //     ))?;

        // // Up front cost is the maximum amount of wei that a user is willing to pay for. Gaslimit * gasprice + value + blob_gas_cost
        // let value = initial_call_frame.msg_value;

        // // blob gas cost = max fee per blob gas * blob gas used
        // // https://eips.ethereum.org/EIPS/eip-4844
        // let max_blob_gas_cost = get_max_blob_gas_price(
        //     self.env.tx_blob_hashes.clone(),
        //     self.env.tx_max_fee_per_blob_gas,
        // )?;

        // // For the transaction to be valid the sender account has to have a balance >= gas_price * gas_limit + value if tx is type 0 and 1
        // // balance >= max_fee_per_gas * gas_limit + value + blob_gas_cost if tx is type 2 or 3
        // let gas_fee_for_valid_tx = self
        //     .env
        //     .tx_max_fee_per_gas
        //     .unwrap_or(self.env.gas_price)
        //     .checked_mul(self.env.gas_limit.into())
        //     .ok_or(VMError::TxValidation(
        //         TxValidationError::GasLimitPriceProductOverflow,
        //     ))?;

        // let balance_for_valid_tx = gas_fee_for_valid_tx
        //     .checked_add(value)
        //     .ok_or(VMError::TxValidation(
        //         TxValidationError::InsufficientAccountFunds,
        //     ))?
        //     .checked_add(max_blob_gas_cost)
        //     .ok_or(VMError::TxValidation(
        //         TxValidationError::InsufficientAccountFunds,
        //     ))?;
        // if sender_account.info.balance < balance_for_valid_tx {
        //     return Err(VMError::TxValidation(
        //         TxValidationError::InsufficientAccountFunds,
        //     ));
        // }

        // let blob_gas_cost = get_blob_gas_price(
        //     self.env.tx_blob_hashes.clone(),
        //     self.env.block_excess_blob_gas,
        //     &self.env.config,
        // )?;

        // // (2) INSUFFICIENT_MAX_FEE_PER_BLOB_GAS
        // if let Some(tx_max_fee_per_blob_gas) = self.env.tx_max_fee_per_blob_gas {
        //     if tx_max_fee_per_blob_gas
        //         < get_base_fee_per_blob_gas(self.env.block_excess_blob_gas, &self.env.config)?
        //     {
        //         return Err(VMError::TxValidation(
        //             TxValidationError::InsufficientMaxFeePerBlobGas,
        //         ));
        //     }
        // }

        // // The real cost to deduct is calculated as effective_gas_price * gas_limit + value + blob_gas_cost
        // let up_front_cost = gaslimit_price_product
        //     .checked_add(value)
        //     .ok_or(VMError::TxValidation(
        //         TxValidationError::InsufficientAccountFunds,
        //     ))?
        //     .checked_add(blob_gas_cost)
        //     .ok_or(VMError::TxValidation(
        //         TxValidationError::InsufficientAccountFunds,
        //     ))?;
        // // There is no error specified for overflow in up_front_cost
        // // in ef_tests. We went for "InsufficientAccountFunds" simply
        // // because if the upfront cost is bigger than U256, then,
        // // technically, the sender will not be able to pay it.

        // // (3) INSUFFICIENT_ACCOUNT_FUNDS
        // decrease_account_balance(&mut self.cache, &mut self.db, sender_address, up_front_cost)
        //     .map_err(|_| TxValidationError::InsufficientAccountFunds)?;

        // // (4) INSUFFICIENT_MAX_FEE_PER_GAS
        // if self.env.tx_max_fee_per_gas.unwrap_or(self.env.gas_price) < self.env.base_fee_per_gas {
        //     return Err(VMError::TxValidation(
        //         TxValidationError::InsufficientMaxFeePerGas,
        //     ));
        // }

        // // (5) INITCODE_SIZE_EXCEEDED
        // if self.is_create() {
        //     // [EIP-3860] - INITCODE_SIZE_EXCEEDED
        //     if initial_call_frame.calldata.len() > INIT_CODE_MAX_SIZE
        //         && self.env.config.fork >= Fork::Shanghai
        //     {
        //         return Err(VMError::TxValidation(
        //             TxValidationError::InitcodeSizeExceeded,
        //         ));
        //     }
        // }

        // // (6) INTRINSIC_GAS_TOO_LOW
        // add_intrinsic_gas(
        //     self.is_create(),
        //     self.env.config.fork,
        //     initial_call_frame,
        //     &self.access_list,
        //     &self.authorization_list,
        // )?;

        // // (7) NONCE_IS_MAX
        // increment_account_nonce(&mut self.cache, &self.db, sender_address)
        //     .map_err(|_| VMError::TxValidation(TxValidationError::NonceIsMax))?;

        // // (8) PRIORITY_GREATER_THAN_MAX_FEE_PER_GAS
        // if let (Some(tx_max_priority_fee), Some(tx_max_fee_per_gas)) = (
        //     self.env.tx_max_priority_fee_per_gas,
        //     self.env.tx_max_fee_per_gas,
        // ) {
        //     if tx_max_priority_fee > tx_max_fee_per_gas {
        //         return Err(VMError::TxValidation(
        //             TxValidationError::PriorityGreaterThanMaxFeePerGas,
        //         ));
        //     }
        // }

        // // (9) SENDER_NOT_EOA
        // if sender_account.has_code() && !has_delegation(&sender_account.info)? {
        //     return Err(VMError::TxValidation(TxValidationError::SenderNotEOA));
        // }

        // // (10) GAS_ALLOWANCE_EXCEEDED
        // if self.env.gas_limit > self.env.block_gas_limit {
        //     return Err(VMError::TxValidation(
        //         TxValidationError::GasAllowanceExceeded,
        //     ));
        // }

        // // Transaction is type 3 if tx_max_fee_per_blob_gas is Some
        // if self.env.tx_max_fee_per_blob_gas.is_some() {
        //     // (11) TYPE_3_TX_PRE_FORK
        //     if self.env.config.fork < Fork::Cancun {
        //         return Err(VMError::TxValidation(TxValidationError::Type3TxPreFork));
        //     }

        //     let blob_hashes = &self.env.tx_blob_hashes;

        //     // (12) TYPE_3_TX_ZERO_BLOBS
        //     if blob_hashes.is_empty() {
        //         return Err(VMError::TxValidation(TxValidationError::Type3TxZeroBlobs));
        //     }

        //     // (13) TYPE_3_TX_INVALID_BLOB_VERSIONED_HASH
        //     for blob_hash in blob_hashes {
        //         let blob_hash = blob_hash.as_bytes();
        //         if let Some(first_byte) = blob_hash.first() {
        //             if !VALID_BLOB_PREFIXES.contains(first_byte) {
        //                 return Err(VMError::TxValidation(
        //                     TxValidationError::Type3TxInvalidBlobVersionedHash,
        //                 ));
        //             }
        //         }
        //     }

        //     // (14) TYPE_3_TX_BLOB_COUNT_EXCEEDED
        //     if blob_hashes.len()
        //         > self
        //             .env
        //             .config
        //             .blob_schedule
        //             .max
        //             .try_into()
        //             .map_err(|_| VMError::Internal(InternalError::ConversionError))?
        //     {
        //         return Err(VMError::TxValidation(
        //             TxValidationError::Type3TxBlobCountExceeded,
        //         ));
        //     }

        //     // (15) TYPE_3_TX_CONTRACT_CREATION
        //     if self.is_create() {
        //         return Err(VMError::TxValidation(
        //             TxValidationError::Type3TxContractCreation,
        //         ));
        //     }
        // }

        // // [EIP-7702]: https://eips.ethereum.org/EIPS/eip-7702
        // // Transaction is type 4 if authorization_list is Some
        // if let Some(auth_list) = &self.authorization_list {
        //     // (16) TYPE_4_TX_PRE_FORK
        //     if self.env.config.fork < Fork::Prague {
        //         return Err(VMError::TxValidation(TxValidationError::Type4TxPreFork));
        //     }

        //     // (17) TYPE_4_TX_CONTRACT_CREATION
        //     // From the EIP docs: a null destination is not valid.
        //     if self.is_create() {
        //         return Err(VMError::TxValidation(
        //             TxValidationError::Type4TxContractCreation,
        //         ));
        //     }

        //     // (18) TYPE_4_TX_LIST_EMPTY
        //     // From the EIP docs: The transaction is considered invalid if the length of authorization_list is zero.
        //     if auth_list.is_empty() {
        //         return Err(VMError::TxValidation(
        //             TxValidationError::Type4TxAuthorizationListIsEmpty,
        //         ));
        //     }

        //     self.env.refunded_gas = eip7702_set_access_code(
        //         &mut self.cache,
        //         &mut self.db,
        //         self.env.chain_id,
        //         &mut self.accrued_substate,
        //         // TODO: avoid clone()
        //         self.authorization_list.clone(),
        //         initial_call_frame,
        //     )?;
        // }

        // if self.is_create() {
        //     // Assign bytecode to context and empty calldata
        //     initial_call_frame.bytecode = std::mem::take(&mut initial_call_frame.calldata);
        //     initial_call_frame.valid_jump_destinations =
        //         get_valid_jump_destinations(&initial_call_frame.bytecode).unwrap_or_default();
        // } else {
        //     // Transfer value to receiver
        //     // It's here to avoid storing the "to" address in the cache before eip7702_set_access_code() step 7).
        //     increase_account_balance(
        //         &mut self.cache,
        //         &mut self.db,
        //         initial_call_frame.to,
        //         initial_call_frame.msg_value,
        //     )?;
        // }
        // Ok(())
    }

    fn finalize_execution(&self) -> Result<(), VMError> {
        todo!();
    }
}
