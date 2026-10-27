use crate::{H160, H256};
use ethrex_crypto::keccak::keccak_hash;
use ethrex_rlp::constants::RLP_NULL;
use std::{str::FromStr, sync::LazyLock};

/// SYSTEM_ADDRESS used for system contract calls and BAL filtering.
/// 0xfffffffffffffffffffffffffffffffffffffffe
pub const SYSTEM_ADDRESS: H160 = H160([
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFE,
]);

/// EIP-6110 beacon deposit contract on Ethereum mainnet.
/// 0x00000000219ab540356cbb839cbe05303d7705fa
///
/// Lives here rather than only in `ethrex_vm::system_contracts` so that
/// `ethrex-common` can reach it: `amsterdam_chain_config` needs it to derive a
/// `ChainConfig` for the stateless guest, and `ethrex-common` cannot depend on
/// `ethrex-vm`. `system_contracts` re-uses this value so there is one copy.
pub const MAINNET_DEPOSIT_CONTRACT_ADDRESS: H160 = H160([
    0x00, 0x00, 0x00, 0x00, 0x21, 0x9A, 0xB5, 0x40, 0x35, 0x6C, 0xBB, 0x83, 0x9C, 0xBE, 0x05, 0x30,
    0x3D, 0x77, 0x05, 0xFA,
]);

// The contracts the protocol calls from `SYSTEM_ADDRESS` while processing a block.
// They live here rather than only in `ethrex_vm::system_contracts` so that the EVM
// can reach them (see `SYSTEM_CALL_CONTRACTS`); `system_contracts` re-uses these
// values so there is one copy.

/// EIP-4788 beacon roots contract. 0x000F3df6D732807Ef1319fB7B8bB8522d0Beac02
pub const BEACON_ROOTS_ADDRESS: H160 = H160([
    0x00, 0x0F, 0x3D, 0xF6, 0xD7, 0x32, 0x80, 0x7E, 0xF1, 0x31, 0x9F, 0xB7, 0xB8, 0xBB, 0x85, 0x22,
    0xD0, 0xBE, 0xAC, 0x02,
]);

/// EIP-2935 block hash history contract. 0x0000F90827F1C53a10cb7A02335B175320002935
pub const HISTORY_STORAGE_ADDRESS: H160 = H160([
    0x00, 0x00, 0xF9, 0x08, 0x27, 0xF1, 0xC5, 0x3A, 0x10, 0xCB, 0x7A, 0x02, 0x33, 0x5B, 0x17, 0x53,
    0x20, 0x00, 0x29, 0x35,
]);

/// EIP-7002 withdrawal request predeploy. 0x00000961Ef480Eb55e80D19ad83579A64c007002
pub const WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS: H160 = H160([
    0x00, 0x00, 0x09, 0x61, 0xEF, 0x48, 0x0E, 0xB5, 0x5E, 0x80, 0xD1, 0x9A, 0xD8, 0x35, 0x79, 0xA6,
    0x4C, 0x00, 0x70, 0x02,
]);

/// EIP-7251 consolidation request predeploy. 0x0000BBdDc7CE488642fb579F8B00f3a590007251
pub const CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS: H160 = H160([
    0x00, 0x00, 0xBB, 0xDD, 0xC7, 0xCE, 0x48, 0x86, 0x42, 0xFB, 0x57, 0x9F, 0x8B, 0x00, 0xF3, 0xA5,
    0x90, 0x00, 0x72, 0x51,
]);

/// EIP-8282 builder deposit predeploy. 0x0000BFF46984E3725691FA540A8C7589300D8282
pub const BUILDER_DEPOSIT_CONTRACT_ADDRESS: H160 = H160([
    0x00, 0x00, 0xBF, 0xF4, 0x69, 0x84, 0xE3, 0x72, 0x56, 0x91, 0xFA, 0x54, 0x0A, 0x8C, 0x75, 0x89,
    0x30, 0x0D, 0x82, 0x82,
]);

/// EIP-8282 builder exit predeploy. 0x000064D678505AD48F8CCB093BC65613800E8282
pub const BUILDER_EXIT_CONTRACT_ADDRESS: H160 = H160([
    0x00, 0x00, 0x64, 0xD6, 0x78, 0x50, 0x5A, 0xD4, 0x8F, 0x8C, 0xCB, 0x09, 0x3B, 0xC6, 0x56, 0x13,
    0x80, 0x0E, 0x82, 0x82,
]);

/// Every contract the protocol calls from `SYSTEM_ADDRESS` while processing a block,
/// at the forks that have `SETCODEFROM` (EIP-8298), which halts when one of them is
/// the current account: block processing depends on their code. The EIP-6110 deposit
/// contract is not one of them, as its logs are read but it is never called.
pub const SYSTEM_CALL_CONTRACTS: [H160; 6] = [
    BEACON_ROOTS_ADDRESS,
    HISTORY_STORAGE_ADDRESS,
    WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
    CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS,
    BUILDER_DEPOSIT_CONTRACT_ADDRESS,
    BUILDER_EXIT_CONTRACT_ADDRESS,
];

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
pub static EMPTY_KECCAK_HASH: LazyLock<H256> = LazyLock::new(|| {
    H256::from_slice(
        &hex::decode("c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470")
            .expect("Failed to decode hex from string"),
    )
});

pub static EMPTY_TRIE_HASH: LazyLock<H256> = LazyLock::new(|| H256(keccak_hash([RLP_NULL])));

// Request related
pub static DEPOSIT_TOPIC: LazyLock<H256> = LazyLock::new(|| {
    H256::from_str("649bbc62d0e31342afea4e5cd82d4049e7e1ee912fc0889aa790803be39038c5")
        .expect("Failed to decode hex from string")
});

// = Keccak256(RLP([])) as of EIP-7928
pub static EMPTY_BLOCK_ACCESS_LIST_HASH: LazyLock<H256> = LazyLock::new(|| {
    H256::from_slice(
        &hex::decode("1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347")
            .expect("Failed to decode hex from string"),
    )
});

// === EIP-4844 constants ===

/// Gas consumption of a single data blob (== blob byte size).
pub const GAS_PER_BLOB: u32 = 1 << 17;

// Minimum base fee per blob
pub const MIN_BASE_FEE_PER_BLOB_GAS: u64 = 1;

// === EIP-7934 constants ===

pub const MAX_BLOCK_SIZE: u64 = 10_485_760;
pub const RLP_BLOCK_SIZE_SAFETY_MARGIN: u64 = 2_097_152;
pub const MAX_RLP_BLOCK_SIZE: u64 = MAX_BLOCK_SIZE - RLP_BLOCK_SIZE_SAFETY_MARGIN;
// Blob base cost defined in EIP-7918
pub const BLOB_BASE_COST: u64 = 8192;

// === EIP-7825 constants ===
// https://eips.ethereum.org/EIPS/eip-7825
pub const POST_OSAKA_GAS_LIMIT_CAP: u64 = 16777216;
pub const TX_MAX_GAS_LIMIT_AMSTERDAM: u64 = 1 << 24; // 16,777,216

/// EIP-7954 (Amsterdam): maximum size of deployed bytecode, 64 KiB.
pub const AMSTERDAM_MAX_CODE_SIZE: u64 = 0x10000;

// === EIP-8037 constant ===
// https://eips.ethereum.org/EIPS/eip-8037
/// Ceiling on a transaction's whole `gas` field from Amsterdam on, reservoir
/// included. EIP-7825's `TX_MAX_GAS_LIMIT` above bounds only the execution-gas
/// dimension, so it is checked against the intrinsic execution cost rather than
/// against `tx.gas`; this one bounds `tx.gas` itself, keeping the field inside
/// the range a `u32` can carry.
pub const TX_MAX_TOTAL_GAS_LIMIT_AMSTERDAM: u64 = u32::MAX as u64; // 4,294,967,295

// === EIP-7928 BAL size cap constants ===
/// GAS_BLOCK_ACCESS_LIST_ITEM: a flat per-item charge, independent of the
/// EIP-8038 access-list repricing.
pub const BAL_ITEM_COST: u64 = 2000;
