use crate::{
    account::{Account, StorageSlot},
    call_frame::CallFrame,
    constants::*,
    db::{
        cache::{self, get_account_mut, remove_account},
        CacheDB, Database,
    },
    environment::Environment,
    errors::{
        InternalError, OpcodeSuccess, OutOfGasError, ResultReason, TransactionReport, TxResult,
        TxValidationError, VMError,
    },
    gas_cost::{
        self, fake_exponential, ACCESS_LIST_ADDRESS_COST, ACCESS_LIST_STORAGE_KEY_COST,
        BLOB_GAS_PER_BLOB, CODE_DEPOSIT_COST, COLD_ADDRESS_ACCESS_COST, CREATE_BASE_COST,
        STANDARD_TOKEN_COST, TOTAL_COST_FLOOR_PER_TOKEN, WARM_ADDRESS_ACCESS_COST,
    },
    opcodes::Opcode,
    precompiles::{
        execute_precompile, is_precompile, SIZE_PRECOMPILES_CANCUN, SIZE_PRECOMPILES_PRAGUE,
        SIZE_PRECOMPILES_PRE_CANCUN,
    },
    vm::{AccessList, AuthorizationList},
    AccountInfo, TransientStorage,
};
use bytes::Bytes;
use ethrex_core::{types::TxKind, Address, H256, U256};
use ethrex_rlp;
use ethrex_rlp::encode::RLPEncode;
use keccak_hash::keccak;
use libsecp256k1::{Message, RecoveryId, Signature};
use revm_primitives::SpecId;
use sha3::{Digest, Keccak256};
use std::{
    cmp::max,
    collections::{HashMap, HashSet},
    fmt::Debug,
    sync::Arc,
};
pub type Storage = HashMap<U256, H256>;

// ================== Address related functions ======================
pub fn address_to_word(address: Address) -> U256 {
    // This unwrap can't panic, as Address are 20 bytes long and U256 use 32 bytes
    let mut word = [0u8; 32];

    for (word_byte, address_byte) in word.iter_mut().skip(12).zip(address.as_bytes().iter()) {
        *word_byte = *address_byte;
    }

    U256::from_big_endian(&word)
}

/// Calculates the address of a new conctract using the CREATE
/// opcode as follows:
///
/// address = keccak256(rlp([sender_address,sender_nonce]))[12:]
pub fn calculate_create_address(
    sender_address: Address,
    sender_nonce: u64,
) -> Result<Address, VMError> {
    let mut encoded = Vec::new();
    (sender_address, sender_nonce).encode(&mut encoded);
    let mut hasher = Keccak256::new();
    hasher.update(encoded);
    Ok(Address::from_slice(hasher.finalize().get(12..).ok_or(
        VMError::Internal(InternalError::CouldNotComputeCreateAddress),
    )?))
}

/// Calculates the address of a new contract using the CREATE2 opcode as follow
///
/// initialization_code = memory[offset:offset+size]
///
/// address = keccak256(0xff + sender_address + salt + keccak256(initialization_code))[12:]
///
pub fn calculate_create2_address(
    sender_address: Address,
    initialization_code: &Bytes,
    salt: U256,
) -> Result<Address, VMError> {
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
        .ok_or(VMError::Internal(
            InternalError::CouldNotComputeCreate2Address,
        ))?,
    );
    Ok(generated_address)
}

// ==================== Word related functions =======================
pub fn word_to_address(word: U256) -> Address {
    Address::from_slice(&word.to_big_endian()[12..])
}

// ==================== Gas related functions =======================

pub fn get_intrinsic_gas(
    is_create: bool,
    spec_id: SpecId,
    access_list: &AccessList,
    authorization_list: &Option<AuthorizationList>,
    initial_call_frame: &CallFrame,
) -> Result<u64, VMError> {
    // Intrinsic Gas = Calldata cost + Create cost + Base cost + Access list cost
    let mut intrinsic_gas: u64 = 0;

    // Calldata Cost
    // 4 gas for each zero byte in the transaction data 16 gas for each non-zero byte in the transaction.
    let calldata_cost =
        gas_cost::tx_calldata(&initial_call_frame.calldata, spec_id).map_err(VMError::OutOfGas)?;

    intrinsic_gas = intrinsic_gas
        .checked_add(calldata_cost)
        .ok_or(OutOfGasError::ConsumedGasOverflow)?;

    // Base Cost
    intrinsic_gas = intrinsic_gas
        .checked_add(TX_BASE_COST)
        .ok_or(OutOfGasError::ConsumedGasOverflow)?;

    // Create Cost
    if is_create {
        intrinsic_gas = intrinsic_gas
            .checked_add(CREATE_BASE_COST)
            .ok_or(OutOfGasError::ConsumedGasOverflow)?;

        let number_of_words = initial_call_frame.calldata.len().div_ceil(WORD_SIZE);
        let double_number_of_words: u64 = number_of_words
            .checked_mul(2)
            .ok_or(OutOfGasError::ConsumedGasOverflow)?
            .try_into()
            .map_err(|_| VMError::Internal(InternalError::ConversionError))?;

        intrinsic_gas = intrinsic_gas
            .checked_add(double_number_of_words)
            .ok_or(OutOfGasError::ConsumedGasOverflow)?;
    }

    // Access List Cost
    let mut access_lists_cost: u64 = 0;
    for (_, keys) in access_list {
        access_lists_cost = access_lists_cost
            .checked_add(ACCESS_LIST_ADDRESS_COST)
            .ok_or(OutOfGasError::ConsumedGasOverflow)?;
        for _ in keys {
            access_lists_cost = access_lists_cost
                .checked_add(ACCESS_LIST_STORAGE_KEY_COST)
                .ok_or(OutOfGasError::ConsumedGasOverflow)?;
        }
    }

    intrinsic_gas = intrinsic_gas
        .checked_add(access_lists_cost)
        .ok_or(OutOfGasError::ConsumedGasOverflow)?;

    // Authorization List Cost
    // `unwrap_or_default` will return an empty vec when the `authorization_list` field is None.
    // If the vec is empty, the len will be 0, thus the authorization_list_cost is 0.
    let amount_of_auth_tuples: u64 = authorization_list
        .clone()
        .unwrap_or_default()
        .len()
        .try_into()
        .map_err(|_| VMError::Internal(InternalError::ConversionError))?;
    let authorization_list_cost = PER_EMPTY_ACCOUNT_COST
        .checked_mul(amount_of_auth_tuples)
        .ok_or(VMError::Internal(InternalError::GasOverflow))?;

    intrinsic_gas = intrinsic_gas
        .checked_add(authorization_list_cost)
        .ok_or(OutOfGasError::ConsumedGasOverflow)?;

    Ok(intrinsic_gas)
}

// ================= Blob hash related functions =====================
/// After EIP-7691 the maximum number of blob hashes changes. For more
/// information see
/// [EIP-7691](https://eips.ethereum.org/EIPS/eip-7691#specification).
pub const fn max_blobs_per_block(specid: SpecId) -> usize {
    match specid {
        SpecId::PRAGUE => MAX_BLOB_COUNT_ELECTRA,
        SpecId::PRAGUE_EOF => MAX_BLOB_COUNT_ELECTRA,
        _ => MAX_BLOB_COUNT,
    }
}

/// According to EIP-7691
/// (https://eips.ethereum.org/EIPS/eip-7691#specification):
///
/// "These changes imply that get_base_fee_per_blob_gas and
/// calc_excess_blob_gas functions defined in EIP-4844 use the new
/// values for the first block of the fork (and for all subsequent
/// blocks)."
pub const fn get_blob_base_fee_update_fraction_value(specid: SpecId) -> U256 {
    match specid {
        SpecId::PRAGUE => BLOB_BASE_FEE_UPDATE_FRACTION_PRAGUE,
        SpecId::PRAGUE_EOF => BLOB_BASE_FEE_UPDATE_FRACTION_PRAGUE,
        _ => BLOB_BASE_FEE_UPDATE_FRACTION,
    }
}

pub fn get_base_fee_per_blob_gas(
    block_excess_blob_gas: Option<U256>,
    spec_id: SpecId,
) -> Result<U256, VMError> {
    fake_exponential(
        MIN_BASE_FEE_PER_BLOB_GAS,
        block_excess_blob_gas.unwrap_or_default(),
        get_blob_base_fee_update_fraction_value(spec_id),
    )
}

/// Gets the max blob gas cost for a transaction that a user is
/// willing to pay.
pub fn get_max_blob_gas_price(
    tx_blob_hashes: Vec<H256>,
    tx_max_fee_per_blob_gas: Option<U256>,
) -> Result<U256, VMError> {
    let blobhash_amount: u64 = tx_blob_hashes
        .len()
        .try_into()
        .map_err(|_| VMError::Internal(InternalError::ConversionError))?;

    let blob_gas_used: u64 = blobhash_amount
        .checked_mul(BLOB_GAS_PER_BLOB)
        .unwrap_or_default();

    let max_blob_gas_cost = tx_max_fee_per_blob_gas
        .unwrap_or_default()
        .checked_mul(blob_gas_used.into())
        .ok_or(InternalError::UndefinedState(1))?;

    Ok(max_blob_gas_cost)
}
