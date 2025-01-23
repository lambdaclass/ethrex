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

// ==================== Word related functions =======================
pub fn word_to_address(word: U256) -> Address {
    Address::from_slice(&word.to_big_endian()[12..])
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
