use ethrex_rlp::constants::RLP_NULL;
use keccak_hash::H256;
use sha3::{Digest as _, Keccak256};
use std::{str::FromStr, sync::LazyLock};

// = Keccak256(RLP([])) as of EIP-3675
pub static DEFAULT_OMMERS_HASH: LazyLock<H256> = LazyLock::new(|| {
    H256::from_slice(
        &hex::decode("1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347")
            .expect("Failed to decode hex from string"),
    )
});

// = Sha256([])) as of EIP-7685
pub static DEFAULT_REQUESTS_HASH: LazyLock<H256> = LazyLock::new(|| {
    H256::from_slice(
        &hex::decode("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
            .expect("Failed to decode hex from string"),
    )
});

// = Root of empty Trie as of EIP-4895
pub static EMPTY_WITHDRAWALS_HASH: LazyLock<H256> = LazyLock::new(|| {
    H256::from_slice(
        &hex::decode("56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421")
            .expect("Failed to decode hex from string"),
    )
});

// Keccak256(""), represents the code hash for an account without code
pub static EMPTY_KECCACK_HASH: LazyLock<H256> = LazyLock::new(|| {
    H256::from_slice(
        &hex::decode("c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470")
            .expect("Failed to decode hex from string"),
    )
});

pub static EMPTY_TRIE_HASH: LazyLock<H256> = LazyLock::new(|| {
    H256::from_slice(
        Keccak256::new()
            .chain_update([RLP_NULL])
            .finalize()
            .as_slice(),
    )
});

// Request related
pub static DEPOSIT_TOPIC: LazyLock<H256> = LazyLock::new(|| {
    H256::from_str("649bbc62d0e31342afea4e5cd82d4049e7e1ee912fc0889aa790803be39038c5")
        .expect("Failed to decode hex from string")
});

// === EIP-4844 constants ===

/// Gas consumption of a single data blob (== blob byte size).
pub const GAS_PER_BLOB: u32 = 1 << 17;

// Minimum base fee per blob
pub const MIN_BASE_FEE_PER_BLOB_GAS: u64 = 1;
