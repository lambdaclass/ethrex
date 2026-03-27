use crate::{
    EVMConfig, Environment,
    account::{AccountStatus, LevmAccount},
    call_frame::CallFrameBackup,
    constants::*,
    db::gen_db::GeneralizedDatabase,
    errors::{ExceptionalHalt, InternalError, TxValidationError, VMError},
    gas_cost::{
        self, ACCESS_LIST_ADDRESS_COST, ACCESS_LIST_STORAGE_KEY_COST, BLOB_GAS_PER_BLOB,
        COLD_ADDRESS_ACCESS_COST, CREATE_BASE_COST, REGULAR_GAS_CREATE, STANDARD_TOKEN_COST,
        STATE_GAS_AUTH_TOTAL, STATE_GAS_NEW_ACCOUNT, TOTAL_COST_FLOOR_PER_TOKEN,
        WARM_ADDRESS_ACCESS_COST,
    },
    vm::{Substate, VM},
};
use ExceptionalHalt::OutOfGas;
use bytes::Bytes;
use ethrex_common::constants::SYSTEM_ADDRESS;
use ethrex_common::types::Log;
use ethrex_common::{
    Address, H256, U256,
    evm::calculate_create_address,
    types::{Account, Code, Fork, Transaction, fake_exponential, tx_fields::*},
    utils::{keccak, u256_to_big_endian},
};
use ethrex_common::{types::TxKind, utils::u256_from_big_endian_const};
use ethrex_rlp;
use rustc_hash::FxHashMap;
pub type Storage = FxHashMap<U256, H256>;

// ================== Address related functions ======================
/// Converts address (H160) to word (U256)
pub fn address_to_word(address: Address) -> U256 {
    let mut word = [0u8; 32];

    for (word_byte, address_byte) in word.iter_mut().skip(12).zip(address.as_bytes().iter()) {
        *word_byte = *address_byte;
    }

    u256_from_big_endian_const(word)
}

/// Calculates the address of a new contract using the CREATE2 opcode as follows
///
/// initialization_code = memory[offset:offset+size]
///
/// address = keccak256(0xff || sender_address || salt || keccak256(initialization_code))[12:]
pub fn calculate_create2_address(
    sender_address: Address,
    initialization_code: &Bytes,
    salt: U256,
) -> Result<Address, InternalError> {
    let init_code_hash = keccak(initialization_code);

    let generated_address = Address::from_slice(
        keccak(
            [
                &[0xff],
                sender_address.as_bytes(),
                &salt.to_big_endian(),
                init_code_hash.as_bytes(),
            ]
            .concat(),
        )
        .as_bytes()
        .get(12..)
        .ok_or(InternalError::Slicing)?,
    );
    Ok(generated_address)
}

// ================== Backup related functions =======================

/// Restore the state of the cache to the state it in the callframe backup.
/// Also restores BAL recorder state changes (but not touched_addresses) per EIP-7928.
pub fn restore_cache_state(
    db: &mut GeneralizedDatabase,
    callframe_backup: CallFrameBackup,
) -> Result<(), VMError> {
    for (address, account) in callframe_backup.original_accounts_info {
        if let Some(current_account) = db.current_accounts_state.get_mut(&address) {
            current_account.info = account.info;
            current_account.status = account.status;
            current_account.has_storage = account.has_storage;
        }
    }

    for (address, storage) in callframe_backup.original_account_storage_slots {
        // This call to `get_account_mut` should never return None, because we are looking up accounts
        // that had their storage modified, which means they should be in the cache. That's why
        // we return an internal error in case we haven't found it.
        let account = db
            .current_accounts_state
            .get_mut(&address)
            .ok_or(InternalError::AccountNotFound)?;

        for (key, value) in storage {
            account.storage.insert(key, value);
        }
    }

    // Restore BAL recorder to checkpoint (but keep touched_addresses per EIP-7928)
    if let Some(checkpoint) = callframe_backup.bal_checkpoint
        && let Some(recorder) = db.bal_recorder.as_mut()
    {
        recorder.restore(checkpoint);
    }

    Ok(())
}

// ================= Blob hash related functions =====================
pub fn get_base_fee_per_blob_gas(
    block_excess_blob_gas: Option<u64>,
    evm_config: &EVMConfig,
) -> Result<U256, VMError> {
    let base_fee_update_fraction = evm_config.blob_schedule.base_fee_update_fraction;
    fake_exponential(
        MIN_BASE_FEE_PER_BLOB_GAS.into(),
        block_excess_blob_gas.unwrap_or_default().into(),
        base_fee_update_fraction,
    )
    .map_err(|err| VMError::Internal(InternalError::FakeExponentialError(err)))
}

/// Gets the max blob gas cost for a transaction that a user is
/// willing to pay.
pub fn get_max_blob_gas_price(
    tx_blob_hashes: &[H256],
    tx_max_fee_per_blob_gas: Option<U256>,
) -> Result<U256, VMError> {
    let blobhash_amount: u64 = tx_blob_hashes
        .len()
        .try_into()
        .map_err(|_| InternalError::TypeConversion)?;

    let blob_gas_used: u64 = blobhash_amount
        .checked_mul(BLOB_GAS_PER_BLOB)
        .unwrap_or_default();

    let max_blob_gas_cost = tx_max_fee_per_blob_gas
        .unwrap_or_default()
        .checked_mul(blob_gas_used.into())
        .ok_or(InternalError::Overflow)?;

    Ok(max_blob_gas_cost)
}
/// Calculate the actual blob gas cost.
pub fn calculate_blob_gas_cost(
    tx_blob_hashes: &[H256],
    block_excess_blob_gas: Option<u64>,
    evm_config: &EVMConfig,
) -> Result<U256, VMError> {
    let blobhash_amount: u64 = tx_blob_hashes
        .len()
        .try_into()
        .map_err(|_| InternalError::TypeConversion)?;

    let blob_gas_used: u64 = blobhash_amount
        .checked_mul(BLOB_GAS_PER_BLOB)
        .unwrap_or_default();

    let base_fee_per_blob_gas = get_base_fee_per_blob_gas(block_excess_blob_gas, evm_config)?;

    let blob_gas_used: U256 = blob_gas_used.into();
    let blob_fee: U256 = blob_gas_used
        .checked_mul(base_fee_per_blob_gas)
        .ok_or(InternalError::Overflow)?;

    Ok(blob_fee)
}

// ==================== Word related functions =======================
pub fn word_to_address(word: U256) -> Address {
    Address::from_slice(&u256_to_big_endian(word)[12..])
}

// ================== EIP-7702 related functions =====================

pub fn code_has_delegation(code: &Bytes) -> Result<bool, VMError> {
    if code.len() == EIP7702_DELEGATED_CODE_LEN {
        let first_3_bytes = &code.get(..3).ok_or(InternalError::Slicing)?;
        return Ok(*first_3_bytes == SET_CODE_DELEGATION_BYTES);
    }
    Ok(false)
}

/// Gets the address inside the bytecode if it has been
/// delegated as the EIP7702 determines.
pub fn get_authorized_address_from_code(code: &Bytes) -> Result<Address, VMError> {
    if code_has_delegation(code)? {
        let address_bytes = &code
            .get(SET_CODE_DELEGATION_BYTES.len()..)
            .ok_or(InternalError::Slicing)?;
        // It shouldn't panic when doing Address::from_slice()
        // because the length is checked inside the code_has_delegation() function
        let address = Address::from_slice(address_bytes);
        Ok(address)
    } else {
        // if we end up here, it means that the address wasn't previously delegated.
        Err(InternalError::AccountNotDelegated.into())
    }
}

pub fn eip7702_recover_address(
    auth_tuple: &AuthorizationTuple,
    crypto: &dyn ethrex_crypto::Crypto,
) -> Result<Option<Address>, VMError> {
    use ethrex_rlp::encode::RLPEncode;

    if auth_tuple.s_signature > *SECP256K1_ORDER_OVER2 || U256::zero() >= auth_tuple.s_signature {
        return Ok(None);
    }
    if auth_tuple.r_signature > *SECP256K1_ORDER || U256::zero() >= auth_tuple.r_signature {
        return Ok(None);
    }
    if auth_tuple.y_parity != U256::one() && auth_tuple.y_parity != U256::zero() {
        return Ok(None);
    }

    let mut rlp_buf = Vec::with_capacity(128);
    rlp_buf.push(MAGIC);
    (auth_tuple.chain_id, auth_tuple.address, auth_tuple.nonce).encode(&mut rlp_buf);
    let msg = crypto.keccak256(&rlp_buf);

    let y_parity: u8 =
        TryInto::<u8>::try_into(auth_tuple.y_parity).map_err(|_| InternalError::TypeConversion)?;

    let mut sig = [0u8; 65];
    sig[..32].copy_from_slice(&auth_tuple.r_signature.to_big_endian());
    sig[32..64].copy_from_slice(&auth_tuple.s_signature.to_big_endian());
    sig[64] = y_parity;

    match crypto.recover_signer(&sig, &msg) {
        Ok(address) => Ok(Some(address)),
        Err(_) => Ok(None),
    }
}

/// Gets code of an account, returning early if it's not a delegated account, otherwise
/// Returns tuple (is_delegated, eip7702_cost, code_address, code).
/// Notice that it also inserts the delegated account to the "accessed accounts" set.
///
/// Where:
/// - `is_delegated`: True if account is a delegated account.
/// - `eip7702_cost`: Cost of accessing the delegated account (if any)
/// - `code_address`: Code address (if delegated, returns the delegated address)
/// - `code`: Bytecode of the code_address, what the EVM will execute.
pub fn eip7702_get_code(
    db: &mut GeneralizedDatabase,
    accrued_substate: &mut Substate,
    address: Address,
) -> Result<(bool, u64, Address, Code), VMError> {
    // Address is the delgated address
    let bytecode = db.get_account_code(address)?;

    // If the Address doesn't have a delegation code
    // return false meaning that is not a delegation
    // return the same address given
    // return the bytecode of the given address
    if !code_has_delegation(&bytecode.bytecode)? {
        return Ok((false, 0, address, bytecode.clone()));
    }

    // Here the address has a delegation code
    // The delegation code has the authorized address
    let auth_address = get_authorized_address_from_code(&bytecode.bytecode)?;

    let access_cost = if accrued_substate.add_accessed_address(auth_address) {
        COLD_ADDRESS_ACCESS_COST
    } else {
        WARM_ADDRESS_ACCESS_COST
    };

    let authorized_bytecode = db.get_account_code(auth_address)?.clone();

    Ok((true, access_cost, auth_address, authorized_bytecode))
}

impl<'a> VM<'a> {
    /// Sets the account code as the EIP7702 determines.
    pub fn eip7702_set_access_code(&mut self) -> Result<(), VMError> {
        let mut refunded_gas: u64 = 0;
        // IMPORTANT:
        // If any of the below steps fail, immediately stop processing that tuple and continue to the next tuple in the list. It will in the case of multiple tuples for the same authority, set the code using the address in the last valid occurrence.
        // If transaction execution results in failure (any exceptional condition or code reverting), setting delegation designations is not rolled back.
        for auth_tuple in self.tx.authorization_list().cloned().unwrap_or_default() {
            let chain_id_not_equals_this_chain_id = auth_tuple.chain_id != self.env.chain_id;
            let chain_id_not_zero = !auth_tuple.chain_id.is_zero();

            // 1. Verify the chain id is either 0 or the chain’s current ID.
            if chain_id_not_zero && chain_id_not_equals_this_chain_id {
                continue;
            }

            // 2. Verify the nonce is less than 2**64 - 1.
            // NOTE: nonce is a u64, it's always less than or equal to u64::MAX
            if auth_tuple.nonce == u64::MAX {
                continue;
            }

            // 3. authority = ecrecover(keccak(MAGIC || rlp([chain_id, address, nonce])), y_parity, r, s)
            //      s value must be less than or equal to secp256k1n/2, as specified in EIP-2.
            let Some(authority_address) = eip7702_recover_address(&auth_tuple, self.crypto)? else {
                continue;
            };

            // 4. Add authority to accessed_addresses (as defined in EIP-2929).
            let authority_account = self.db.get_account(authority_address)?;
            let authority_exists = authority_account.exists;
            let authority_info = authority_account.info.clone();
            let authority_code = self.db.get_code(authority_info.code_hash)?;
            self.substate.add_accessed_address(authority_address);

            // 5. Verify the code of authority is either empty or already delegated.
            // Check this BEFORE recording to BAL so we can release the borrow on authority_code.
            let empty_or_delegated = authority_code.bytecode.is_empty()
                || code_has_delegation(&authority_code.bytecode)?;

            // Record authority as touched for BAL per EIP-7928, even if validation fails later.
            // This ensures authority appears in BAL with empty change set when:
            // - Authority was loaded (above)
            // - But validation fails (checks below)
            if let Some(recorder) = self.db.bal_recorder.as_mut() {
                recorder.record_touched_address(authority_address);
            }

            if !empty_or_delegated {
                continue;
            }

            // 6. Verify the nonce of authority is equal to nonce. In case authority does not exist in the trie, verify that nonce is equal to 0.
            // If it doesn't exist, it means the nonce is zero. The get_account() function will return Account::default()
            // If it has nonce, the account.info.nonce should equal auth_tuple.nonce
            if authority_info.nonce != auth_tuple.nonce {
                continue;
            }

            // 7. Refund if authority exists in the trie.
            // EIP-8037 (Amsterdam+): return STATE_BYTES_PER_NEW_ACCOUNT * COST_PER_STATE_BYTE
            // to the state gas reservoir (the new-account portion of the auth state charge).
            // Pre-Amsterdam: add REFUND_AUTH_PER_EXISTING_ACCOUNT (12500) to global refund counter.
            // NOTE: Uses `exists` (account_exists in EELS / Exist in geth), NOT `!is_empty()`.
            // An account can exist in the trie but be empty (e.g., has non-empty storage root).
            if authority_exists {
                if self.env.config.fork >= Fork::Amsterdam {
                    let state_refund = STATE_GAS_NEW_ACCOUNT;
                    self.state_gas_reservoir = self
                        .state_gas_reservoir
                        .checked_add(state_refund)
                        .ok_or(InternalError::Overflow)?;
                    // Track as intrinsic state gas adjustment (matches EELS intrinsic_state_gas -= refund).
                    // Do NOT reduce state_gas_used here — that would inflate regular_gas in block accounting.
                    self.intrinsic_state_gas_refund = self
                        .intrinsic_state_gas_refund
                        .checked_add(state_refund)
                        .ok_or(InternalError::Overflow)?;
                } else {
                    refunded_gas = refunded_gas
                        .checked_add(REFUND_AUTH_PER_EXISTING_ACCOUNT)
                        .ok_or(InternalError::Overflow)?;
                }
            }

            // 8. Set the code of authority to be 0xef0100 || address. This is a delegation designation.
            let delegation_bytes = [
                &SET_CODE_DELEGATION_BYTES[..],
                auth_tuple.address.as_bytes(),
            ]
            .concat();

            // As a special case, if address is 0x0000000000000000000000000000000000000000 do not write the designation.
            // Clear the account’s code and reset the account’s code hash to the empty hash.
            let code = if auth_tuple.address != Address::zero() {
                delegation_bytes.into()
            } else {
                Bytes::new()
            };
            self.update_account_bytecode(
                authority_address,
                Code::from_bytecode(code, self.crypto),
            )?;

            // 9. Increase the nonce of authority by one.
            self.increment_account_nonce(authority_address)
                .map_err(|_| TxValidationError::NonceIsMax)?;
        }

        self.substate.refunded_gas = self
            .substate
            .refunded_gas
            .checked_add(refunded_gas)
            .ok_or(InternalError::Overflow)?;

        Ok(())
    }

    pub fn add_intrinsic_gas(&mut self) -> Result<(), VMError> {
        // Intrinsic gas is the gas consumed by the transaction before the execution of the opcodes. Section 6.2 in the Yellow Paper.

        let (regular_gas, state_gas) = self.get_intrinsic_gas()?;

        let total_gas = regular_gas.checked_add(state_gas).ok_or(OutOfGas)?;

        self.current_call_frame
            .increase_consumed_gas(total_gas)
            .map_err(|_| TxValidationError::IntrinsicGasTooLow)?;

        self.state_gas_used = self
            .state_gas_used
            .checked_add(state_gas)
            .ok_or(InternalError::Overflow)?;

        // EIP-8037 (Amsterdam+): compute state gas reservoir from excess gas_limit.
        // execution_gas = what remains after all intrinsic gas; regular_gas_budget = how much
        // regular execution gas is allowed (capped at TX_MAX_GAS_LIMIT_AMSTERDAM); the difference becomes
        // the reservoir for drawing state gas without consuming regular gas_remaining.
        if self.env.config.fork >= Fork::Amsterdam {
            let gas_limit = self.tx.gas_limit();
            let execution_gas = gas_limit.saturating_sub(total_gas);
            let regular_gas_budget = TX_MAX_GAS_LIMIT_AMSTERDAM.saturating_sub(regular_gas);
            let gas_left = regular_gas_budget.min(execution_gas);
            let reservoir = execution_gas.saturating_sub(gas_left);
            if reservoir > 0 {
                // Pre-consume reservoir from gas_remaining so GAS opcode returns <= TX_MAX_GAS_LIMIT_AMSTERDAM
                let reservoir_i64 =
                    i64::try_from(reservoir).map_err(|_| InternalError::Overflow)?;
                self.current_call_frame.gas_remaining = self
                    .current_call_frame
                    .gas_remaining
                    .checked_sub(reservoir_i64)
                    .ok_or(InternalError::Overflow)?;
                self.state_gas_reservoir = reservoir;
            }
        }

        Ok(())
    }

    // ==================== Gas related functions =======================
    /// Returns `(regular_gas, state_gas)` intrinsic gas for the transaction.
    /// For Amsterdam+, state_gas is the EIP-8037 state portion.
    /// For pre-Amsterdam, state_gas is always 0.
    pub fn get_intrinsic_gas(&self) -> Result<(u64, u64), VMError> {
        // Intrinsic Gas = Calldata cost + Create cost + Base cost + Access list cost
        let mut regular_gas: u64 = 0;
        let mut state_gas: u64 = 0;
        let fork = self.env.config.fork;

        // Calldata Cost
        // 4 gas for each zero byte in the transaction data 16 gas for each non-zero byte in the transaction.
        let calldata_cost = gas_cost::tx_calldata(&self.current_call_frame.calldata)?;

        regular_gas = regular_gas.checked_add(calldata_cost).ok_or(OutOfGas)?;

        // Base Cost
        regular_gas = regular_gas.checked_add(TX_BASE_COST).ok_or(OutOfGas)?;

        // Create Cost
        if self.is_create()? {
            if fork >= Fork::Amsterdam {
                // EIP-8037: reduced regular cost + state gas for new account
                regular_gas = regular_gas
                    .checked_add(REGULAR_GAS_CREATE)
                    .ok_or(OutOfGas)?;
                state_gas = state_gas
                    .checked_add(STATE_GAS_NEW_ACCOUNT)
                    .ok_or(OutOfGas)?;
            } else {
                // https://eips.ethereum.org/EIPS/eip-2#specification
                regular_gas = regular_gas.checked_add(CREATE_BASE_COST).ok_or(OutOfGas)?;
            }

            // https://eips.ethereum.org/EIPS/eip-3860
            if fork >= Fork::Shanghai {
                let number_of_words = &self.current_call_frame.calldata.len().div_ceil(WORD_SIZE);
                let double_number_of_words: u64 = number_of_words
                    .checked_mul(2)
                    .ok_or(OutOfGas)?
                    .try_into()
                    .map_err(|_| InternalError::TypeConversion)?;

                regular_gas = regular_gas
                    .checked_add(double_number_of_words)
                    .ok_or(OutOfGas)?;
            }
        }

        // Access List Cost
        let mut access_lists_cost: u64 = 0;
        for (_, keys) in self.tx.access_list() {
            access_lists_cost = access_lists_cost
                .checked_add(ACCESS_LIST_ADDRESS_COST)
                .ok_or(OutOfGas)?;
            for _ in keys {
                access_lists_cost = access_lists_cost
                    .checked_add(ACCESS_LIST_STORAGE_KEY_COST)
                    .ok_or(OutOfGas)?;
            }
        }

        regular_gas = regular_gas.checked_add(access_lists_cost).ok_or(OutOfGas)?;

        // Authorization List Cost
        // `unwrap_or_default` will return an empty vec when the `authorization_list` field is None.
        // If the vec is empty, the len will be 0, thus the authorization_list_cost is 0.
        let amount_of_auth_tuples: u64 = match self.tx.authorization_list() {
            None => 0,
            Some(list) => list
                .len()
                .try_into()
                .map_err(|_| InternalError::TypeConversion)?,
        };

        if fork >= Fork::Amsterdam {
            // EIP-8037: per-auth regular cost is PER_AUTH_BASE_COST, state is 135 * COST_PER_STATE_BYTE
            let regular_auth_cost = PER_AUTH_BASE_COST
                .checked_mul(amount_of_auth_tuples)
                .ok_or(InternalError::Overflow)?;
            regular_gas = regular_gas.checked_add(regular_auth_cost).ok_or(OutOfGas)?;
            let state_auth_cost = STATE_GAS_AUTH_TOTAL
                .checked_mul(amount_of_auth_tuples)
                .ok_or(InternalError::Overflow)?;
            state_gas = state_gas.checked_add(state_auth_cost).ok_or(OutOfGas)?;
        } else {
            let authorization_list_cost = PER_EMPTY_ACCOUNT_COST
                .checked_mul(amount_of_auth_tuples)
                .ok_or(InternalError::Overflow)?;
            regular_gas = regular_gas
                .checked_add(authorization_list_cost)
                .ok_or(OutOfGas)?;
        }

        Ok((regular_gas, state_gas))
    }

    /// Calculates the minimum gas to be consumed in the transaction.
    pub fn get_min_gas_used(&self) -> Result<u64, VMError> {
        // If the transaction is a CREATE transaction, the calldata is emptied and the bytecode is assigned.
        let calldata = if self.is_create()? {
            &self.current_call_frame.bytecode.bytecode
        } else {
            &self.current_call_frame.calldata
        };

        // tokens_in_calldata = nonzero_bytes_in_calldata * 4 + zero_bytes_in_calldata
        // tx_calldata = nonzero_bytes_in_calldata * 16 + zero_bytes_in_calldata * 4
        // this is actually tokens_in_calldata * STANDARD_TOKEN_COST
        // see it in https://eips.ethereum.org/EIPS/eip-7623
        let tokens_in_calldata: u64 = gas_cost::tx_calldata(calldata)? / STANDARD_TOKEN_COST;

        // min_gas_used = TX_BASE_COST + TOTAL_COST_FLOOR_PER_TOKEN * tokens_in_calldata
        let mut min_gas_used: u64 = tokens_in_calldata
            .checked_mul(TOTAL_COST_FLOOR_PER_TOKEN)
            .ok_or(InternalError::Overflow)?;

        min_gas_used = min_gas_used
            .checked_add(TX_BASE_COST)
            .ok_or(InternalError::Overflow)?;

        Ok(min_gas_used)
    }

    /// Gets transaction callee, calculating create address if it's a "Create" transaction.
    /// Bool indicates whether it is a `create` transaction or not.
    pub fn get_tx_callee(
        tx: &Transaction,
        db: &mut GeneralizedDatabase,
        env: &Environment,
        substate: &mut Substate,
    ) -> Result<(Address, bool), VMError> {
        match tx.to() {
            TxKind::Call(address_to) => {
                substate.add_accessed_address(address_to);

                Ok((address_to, false))
            }

            TxKind::Create => {
                let sender_nonce = db.get_account(env.origin)?.info.nonce;

                let created_address = calculate_create_address(env.origin, sender_nonce);

                substate.add_accessed_address(created_address);
                substate.add_created_account(created_address);

                Ok((created_address, true))
            }
        }
    }
}

/// Converts Account to LevmAccount
/// The problem with this is that we don't have the storage root.
pub fn account_to_levm_account(account: Account) -> (LevmAccount, Code) {
    (
        LevmAccount {
            info: account.info,
            has_storage: !account.storage.is_empty(), // This is used in scenarios in which the storage is already all in the account. For the Levm Runner
            storage: account.storage,
            status: AccountStatus::Unmodified,
            exists: true,
        },
        account.code,
    )
}

/// Converts a U256 value into usize, returning an error if the value is over 32 bits
/// This is generally used for memory offsets and sizes, 32 bits is more than enough for this purpose.
#[expect(clippy::as_conversions)]
pub fn u256_to_usize(val: U256) -> Result<usize, VMError> {
    if val.0[0] > u32::MAX as u64 || val.0[1] != 0 || val.0[2] != 0 || val.0[3] != 0 {
        return Err(VMError::ExceptionalHalt(ExceptionalHalt::VeryLargeNumber));
    }
    Ok(val.0[0] as usize)
}

/// Converts U256 size and offset to usize.
/// If the size is zero, the offset will be zero regardless of its original value as it is not relevant
pub fn size_offset_to_usize(size: U256, offset: U256) -> Result<(usize, usize), VMError> {
    if size.is_zero() {
        // Offset is irrelevant
        Ok((0, 0))
    } else {
        Ok((u256_to_usize(size)?, u256_to_usize(offset)?))
    }
}

// ==================== EIP-7708 Helper Functions ====================

/// Creates EIP-7708 Transfer log (LOG3) for ETH transfers.
/// Emitted from SYSTEM_ADDRESS when ETH is transferred.
#[inline]
pub fn create_eth_transfer_log(from: Address, to: Address, value: U256) -> Log {
    let mut from_topic = [0u8; 32];
    from_topic[12..].copy_from_slice(from.as_bytes());

    let mut to_topic = [0u8; 32];
    to_topic[12..].copy_from_slice(to.as_bytes());

    let data = value.to_big_endian();

    Log {
        address: SYSTEM_ADDRESS,
        topics: vec![
            TRANSFER_EVENT_TOPIC,
            H256::from(from_topic),
            H256::from(to_topic),
        ],
        data: Bytes::from(data.to_vec()),
    }
}

/// Creates EIP-7708 Burn log (LOG2) for ETH burns.
/// Emitted from SYSTEM_ADDRESS when ETH is burned (e.g. via SELFDESTRUCT).
#[inline]
pub fn create_burn_log(address: Address, amount: U256) -> Log {
    let mut address_topic = [0u8; 32];
    address_topic[12..].copy_from_slice(address.as_bytes());

    let data = amount.to_big_endian();

    Log {
        address: SYSTEM_ADDRESS,
        topics: vec![BURN_EVENT_TOPIC, H256::from(address_topic)],
        data: Bytes::from(data.to_vec()),
    }
}
