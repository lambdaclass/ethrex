use crate::{
    EVMConfig, Environment,
    account::{AccountStatus, LevmAccount},
    call_frame::CallFrameBackup,
    constants::*,
    db::gen_db::GeneralizedDatabase,
    errors::{ExceptionalHalt, InternalError, TxValidationError, VMError},
    gas_cost::{
        self, ACCESS_LIST_ADDRESS_COST, ACCESS_LIST_STORAGE_KEY_COST, BLOB_GAS_PER_BLOB,
        COLD_ADDRESS_ACCESS_COST, CREATE_BASE_COST, STANDARD_TOKEN_COST,
        TOTAL_COST_FLOOR_PER_TOKEN, WARM_ADDRESS_ACCESS_COST, fake_exponential,
    },
    opcodes::Opcode,
    vm::{Substate, VM},
};
use ExceptionalHalt::OutOfGas;
use bytes::{Bytes, buf::IntoIter};
use ethrex_common::{
    Address, H256, U256,
    evm::calculate_create_address,
    types::{Account, Fork, Transaction, tx_fields::*},
    utils::u256_to_big_endian,
};
use ethrex_common::{types::TxKind, utils::u256_from_big_endian_const};
use ethrex_rlp;
use ethrex_rlp::encode::RLPEncode;
use keccak_hash::keccak;
use secp256k1::{
    Message,
    ecdsa::{RecoverableSignature, RecoveryId},
};
use sha3::{Digest, Keccak256};
use std::{collections::HashMap, iter::Enumerate};
pub type Storage = HashMap<U256, H256>;

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

/// # Filter for jump target offsets.
///
/// Used to filter which program offsets are not valid jump targets. Implemented as a sorted list of
/// offsets of bytes `0x5B` (`JUMPDEST`) within push constants.
#[derive(Debug)]
pub struct JumpTargetFilter {
    /// The list of invalid jump target offsets.
    filter: Vec<usize>,
    /// The last processed offset, plus one.
    offset: usize,

    /// Program bytecode iterator.
    iter: Enumerate<IntoIter<Bytes>>,
    /// Number of bytes remaining to process from the last push instruction.
    partial: usize,
}

impl JumpTargetFilter {
    /// Create an empty `JumpTargetFilter`.
    pub fn new(bytecode: Bytes) -> Self {
        Self {
            filter: Vec::new(),
            offset: 0,

            iter: bytecode.into_iter().enumerate(),
            partial: 0,
        }
    }

    /// Check whether a target jump address is blacklisted or not.
    ///
    /// This method may potentially grow the filter if the requested address is out of range.
    pub fn is_blacklisted(&mut self, address: usize) -> bool {
        if let Some(delta) = address.checked_sub(self.offset) {
            // It is not realistic to expect a bytecode offset to overflow an `usize`.
            #[expect(clippy::arithmetic_side_effects)]
            for (offset, value) in (&mut self.iter).take(delta + 1) {
                match self.partial.checked_sub(1) {
                    None => {
                        // Neither the `as` conversions nor the subtraction can fail here.
                        #[expect(clippy::as_conversions)]
                        if (Opcode::PUSH1..=Opcode::PUSH32).contains(&Opcode::from(value)) {
                            self.partial = value as usize - Opcode::PUSH0 as usize;
                        }
                    }
                    Some(partial) => {
                        self.partial = partial;

                        #[expect(clippy::as_conversions)]
                        if value == Opcode::JUMPDEST as u8 {
                            self.filter.push(offset);
                        }
                    }
                }
            }

            self.filter.last() == Some(&address)
        } else {
            self.filter.binary_search(&address).is_ok()
        }
    }
}

// ================== Backup related functions =======================

/// Restore the state of the cache to the state it in the callframe backup.
pub fn restore_cache_state(
    db: &mut GeneralizedDatabase,
    callframe_backup: CallFrameBackup,
) -> Result<(), VMError> {
    for (address, account) in callframe_backup.original_accounts_info {
        if let Some(current_account) = db.current_accounts_state.get_mut(&address) {
            current_account.info = account.info;
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

    Ok(())
}

// ================= Blob hash related functions =====================
pub fn get_base_fee_per_blob_gas(
    block_excess_blob_gas: Option<U256>,
    evm_config: &EVMConfig,
) -> Result<U256, VMError> {
    let base_fee_update_fraction = evm_config.blob_schedule.base_fee_update_fraction;
    fake_exponential(
        MIN_BASE_FEE_PER_BLOB_GAS,
        block_excess_blob_gas.unwrap_or_default(),
        base_fee_update_fraction,
    )
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
/// Gets the actual blob gas cost.
pub fn get_blob_gas_price(
    tx_blob_hashes: &[H256],
    block_excess_blob_gas: Option<U256>,
    evm_config: &EVMConfig,
) -> Result<U256, VMError> {
    let blobhash_amount: u64 = tx_blob_hashes
        .len()
        .try_into()
        .map_err(|_| InternalError::TypeConversion)?;

    let blob_gas_price: u64 = blobhash_amount
        .checked_mul(BLOB_GAS_PER_BLOB)
        .unwrap_or_default();

    let base_fee_per_blob_gas = get_base_fee_per_blob_gas(block_excess_blob_gas, evm_config)?;

    let blob_gas_price: U256 = blob_gas_price.into();
    let blob_fee: U256 = blob_gas_price
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
) -> Result<Option<Address>, VMError> {
    if auth_tuple.s_signature > *SECP256K1_ORDER_OVER2 || U256::zero() >= auth_tuple.s_signature {
        return Ok(None);
    }
    if auth_tuple.r_signature > *SECP256K1_ORDER || U256::zero() >= auth_tuple.r_signature {
        return Ok(None);
    }
    if auth_tuple.y_parity != U256::one() && auth_tuple.y_parity != U256::zero() {
        return Ok(None);
    }

    let rlp_buf = (auth_tuple.chain_id, auth_tuple.address, auth_tuple.nonce).encode_to_vec();

    let mut hasher = Keccak256::new();
    hasher.update([MAGIC]);
    hasher.update(rlp_buf);
    let bytes = &mut hasher.finalize();

    let Ok(message) = Message::from_digest_slice(bytes) else {
        return Ok(None);
    };

    let bytes = [
        auth_tuple.r_signature.to_big_endian(),
        auth_tuple.s_signature.to_big_endian(),
    ]
    .concat();

    let Ok(recovery_id) = RecoveryId::try_from(
        TryInto::<i32>::try_into(auth_tuple.y_parity).map_err(|_| InternalError::TypeConversion)?,
    ) else {
        return Ok(None);
    };

    let Ok(signature) = RecoverableSignature::from_compact(&bytes, recovery_id) else {
        return Ok(None);
    };

    //recover
    let Ok(authority) = signature.recover(&message) else {
        return Ok(None);
    };

    let public_key = authority.serialize_uncompressed();
    let mut hasher = Keccak256::new();
    hasher.update(public_key.get(1..).ok_or(InternalError::Slicing)?);
    let address_hash = hasher.finalize();

    // Get the last 20 bytes of the hash -> Address
    let authority_address_bytes: [u8; 20] = address_hash
        .get(12..32)
        .ok_or(InternalError::Slicing)?
        .try_into()
        .map_err(|_| InternalError::TypeConversion)?;
    Ok(Some(Address::from_slice(&authority_address_bytes)))
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
) -> Result<(bool, u64, Address, Bytes), VMError> {
    // Address is the delgated address
    let bytecode = db.get_account_code(address)?;

    // If the Address doesn't have a delegation code
    // return false meaning that is not a delegation
    // return the same address given
    // return the bytecode of the given address
    if !code_has_delegation(bytecode)? {
        return Ok((false, 0, address, bytecode.clone()));
    }

    // Here the address has a delegation code
    // The delegation code has the authorized address
    let auth_address = get_authorized_address_from_code(bytecode)?;

    let access_cost = if accrued_substate.add_accessed_address(auth_address) {
        WARM_ADDRESS_ACCESS_COST
    } else {
        COLD_ADDRESS_ACCESS_COST
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
            let Some(authority_address) = eip7702_recover_address(&auth_tuple)? else {
                continue;
            };

            // 4. Add authority to accessed_addresses (as defined in EIP-2929).
            let authority_info = self.db.get_account(authority_address)?.info.clone();
            let authority_code = self.db.get_code(authority_info.code_hash)?;
            self.substate.add_accessed_address(authority_address);

            // 5. Verify the code of authority is either empty or already delegated.
            let empty_or_delegated =
                authority_code.is_empty() || code_has_delegation(authority_code)?;
            if !empty_or_delegated {
                continue;
            }

            // 6. Verify the nonce of authority is equal to nonce. In case authority does not exist in the trie, verify that nonce is equal to 0.
            // If it doesn't exist, it means the nonce is zero. The get_account() function will return Account::default()
            // If it has nonce, the account.info.nonce should equal auth_tuple.nonce
            if authority_info.nonce != auth_tuple.nonce {
                continue;
            }

            // 7. Add PER_EMPTY_ACCOUNT_COST - PER_AUTH_BASE_COST gas to the global refund counter if authority exists in the trie.
            if !authority_info.is_empty() {
                let refunded_gas_if_exists = PER_EMPTY_ACCOUNT_COST - PER_AUTH_BASE_COST;
                refunded_gas = refunded_gas
                    .checked_add(refunded_gas_if_exists)
                    .ok_or(InternalError::Overflow)?;
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
            self.update_account_bytecode(authority_address, code)?;

            // 9. Increase the nonce of authority by one.
            self.increment_account_nonce(authority_address)
                .map_err(|_| TxValidationError::NonceIsMax)?;
        }

        self.substate.refunded_gas = refunded_gas;

        Ok(())
    }

    pub fn add_intrinsic_gas(&mut self) -> Result<(), VMError> {
        // Intrinsic gas is the gas consumed by the transaction before the execution of the opcodes. Section 6.2 in the Yellow Paper.

        let intrinsic_gas = self.get_intrinsic_gas()?;

        self.current_call_frame
            .increase_consumed_gas(intrinsic_gas)
            .map_err(|_| TxValidationError::IntrinsicGasTooLow)?;

        Ok(())
    }

    // ==================== Gas related functions =======================
    pub fn get_intrinsic_gas(&self) -> Result<u64, VMError> {
        // Intrinsic Gas = Calldata cost + Create cost + Base cost + Access list cost
        let mut intrinsic_gas: u64 = 0;

        // Calldata Cost
        // 4 gas for each zero byte in the transaction data 16 gas for each non-zero byte in the transaction.
        let calldata_cost = gas_cost::tx_calldata(&self.current_call_frame.calldata)?;

        intrinsic_gas = intrinsic_gas.checked_add(calldata_cost).ok_or(OutOfGas)?;

        // Base Cost
        intrinsic_gas = intrinsic_gas.checked_add(TX_BASE_COST).ok_or(OutOfGas)?;

        // Create Cost
        if self.is_create()? {
            // https://eips.ethereum.org/EIPS/eip-2#specification
            intrinsic_gas = intrinsic_gas
                .checked_add(CREATE_BASE_COST)
                .ok_or(OutOfGas)?;

            // https://eips.ethereum.org/EIPS/eip-3860
            if self.env.config.fork >= Fork::Shanghai {
                let number_of_words = &self.current_call_frame.calldata.len().div_ceil(WORD_SIZE);
                let double_number_of_words: u64 = number_of_words
                    .checked_mul(2)
                    .ok_or(OutOfGas)?
                    .try_into()
                    .map_err(|_| InternalError::TypeConversion)?;

                intrinsic_gas = intrinsic_gas
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

        intrinsic_gas = intrinsic_gas
            .checked_add(access_lists_cost)
            .ok_or(OutOfGas)?;

        // Authorization List Cost
        // `unwrap_or_default` will return an empty vec when the `authorization_list` field is None.
        // If the vec is empty, the len will be 0, thus the authorization_list_cost is 0.
        let amount_of_auth_tuples = match self.tx.authorization_list() {
            None => 0,
            Some(list) => list
                .len()
                .try_into()
                .map_err(|_| InternalError::TypeConversion)?,
        };
        let authorization_list_cost = PER_EMPTY_ACCOUNT_COST
            .checked_mul(amount_of_auth_tuples)
            .ok_or(InternalError::Overflow)?;

        intrinsic_gas = intrinsic_gas
            .checked_add(authorization_list_cost)
            .ok_or(OutOfGas)?;

        Ok(intrinsic_gas)
    }

    /// Calculates the minimum gas to be consumed in the transaction.
    pub fn get_min_gas_used(&self) -> Result<u64, VMError> {
        // If the transaction is a CREATE transaction, the calldata is emptied and the bytecode is assigned.
        let calldata = if self.is_create()? {
            &self.current_call_frame.bytecode
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
pub fn account_to_levm_account(account: Account) -> (LevmAccount, Bytes) {
    (
        LevmAccount {
            info: account.info,
            storage: account.storage,
            status: AccountStatus::Unmodified,
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
