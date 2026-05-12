//! Minimal inlined types for parsing EIP-3155 `statetest` JSON files.
//!
//! These are inlined here (Option B) rather than imported from `tooling/ef_tests/state`
//! because that crate lives in a separate Cargo workspace and pulls in `revm` and `simd-json`.
//! Only the fields required for Phase 4 execution are included; unknown JSON fields are
//! silently ignored (no `#[serde(deny_unknown_fields)]`).

use std::collections::BTreeMap;

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    types::{ChainConfig, GenesisAccount},
};
use serde::{Deserialize, Serialize};

/// Authorization tuple for EIP-7702 set-code transactions.
///
/// Mirrors `EFTestAuthorizationListTuple` from `tooling/ef_tests/state/types.rs`.
/// Accepts both `"v"` and `"yParity"` JSON keys for the y-parity field because
/// older EF vectors used `"v"` while newer Prague vectors use `"yParity"`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestAuthTuple {
    #[serde(deserialize_with = "ethrex_common::serde_utils::u64::deser_hex_or_dec_str")]
    pub chain_id: u64,
    pub address: Address,
    #[serde(deserialize_with = "ethrex_common::serde_utils::u64::deser_hex_or_dec_str")]
    pub nonce: u64,
    #[serde(alias = "yParity", alias = "y_parity")]
    pub v: U256,
    pub r: U256,
    pub s: U256,
}

/// A single `statetest` JSON file (the outer map keyed by test name).
pub type StateTestFile = BTreeMap<String, StateTest>;

/// One named state test containing pre-state, environment, transaction, and post-state vectors.
#[derive(Debug, Clone, Deserialize)]
pub struct StateTest {
    /// Pre-execution account states.
    pub pre: BTreeMap<Address, StateTestAccount>,
    /// Block environment fields.
    pub env: TestEnv,
    /// Transaction template (gas / value fields are indexed per subtest).
    pub transaction: TestTransaction,
    /// Post-state vectors keyed by fork name, then subtest index.
    pub post: BTreeMap<String, Vec<PostStateVector>>,
}

/// An account entry in the `pre` section.
#[derive(Debug, Clone, Deserialize)]
pub struct StateTestAccount {
    #[serde(default, with = "ethrex_common::serde_utils::bytes")]
    pub code: Bytes,
    #[serde(default)]
    pub storage: BTreeMap<U256, U256>,
    #[serde(deserialize_with = "ethrex_common::serde_utils::u256::deser_hex_or_dec_str")]
    pub balance: U256,
    #[serde(default, with = "ethrex_common::serde_utils::u64::hex_str")]
    pub nonce: u64,
}

impl From<&StateTestAccount> for GenesisAccount {
    fn from(a: &StateTestAccount) -> Self {
        GenesisAccount {
            code: a.code.clone(),
            storage: a.storage.clone(),
            balance: a.balance,
            nonce: a.nonce,
        }
    }
}

/// Block environment fields for a state test.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestEnv {
    pub current_coinbase: Address,
    #[serde(deserialize_with = "ethrex_common::serde_utils::u256::deser_hex_or_dec_str")]
    pub current_difficulty: U256,
    #[serde(deserialize_with = "ethrex_common::serde_utils::u64::deser_hex_or_dec_str")]
    pub current_gas_limit: u64,
    #[serde(deserialize_with = "ethrex_common::serde_utils::u64::deser_hex_or_dec_str")]
    pub current_number: u64,
    #[serde(deserialize_with = "ethrex_common::serde_utils::u64::deser_hex_or_dec_str")]
    pub current_timestamp: u64,
    #[serde(
        default,
        deserialize_with = "ethrex_common::serde_utils::u256::deser_hex_str_opt"
    )]
    pub current_base_fee: Option<U256>,
    #[serde(default)]
    pub current_random: Option<H256>,
    /// Excess blob gas for EIP-4844 blob fee computation. Present in Cancun+ vectors.
    #[serde(default, deserialize_with = "deser_u64_hex_or_dec_opt")]
    pub current_excess_blob_gas: Option<u64>,
}

/// The transaction template for a state test. Indexes are per-subtest.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestTransaction {
    /// Per-subtest calldata bodies, each a hex-encoded byte string. Without
    /// the custom deserializer, the default `Vec<Bytes>` parse would treat
    /// `"0x"` as the literal ASCII bytes `'0','x'` (intrinsic-gas accounting
    /// would then over-charge 32 gas for what should be empty calldata).
    #[serde(deserialize_with = "deser_vec_hex_bytes")]
    pub data: Vec<Bytes>,
    #[serde(deserialize_with = "deser_vec_u64_hex_dec")]
    pub gas_limit: Vec<u64>,
    pub gas_price: Option<U256>,
    #[serde(deserialize_with = "ethrex_common::serde_utils::u64::deser_hex_or_dec_str")]
    pub nonce: u64,
    /// Private key for sender derivation. Optional; some vectors supply `sender` directly.
    #[serde(default)]
    pub secret_key: Option<H256>,
    /// Pre-derived sender address. When present, used directly without key derivation.
    #[serde(default)]
    pub sender: Option<Address>,
    pub to: Option<Address>,
    pub value: Vec<U256>,
    pub max_fee_per_gas: Option<U256>,
    pub max_priority_fee_per_gas: Option<U256>,
    #[serde(default)]
    pub access_lists: Vec<serde_json::Value>,
    /// EIP-7702 authorization list. Each entry delegates to a target address.
    #[serde(default)]
    pub authorization_list: Option<Vec<TestAuthTuple>>,
    /// EIP-4844 blob versioned hashes for blob transactions.
    #[serde(default)]
    pub blob_versioned_hashes: Option<Vec<H256>>,
    /// EIP-4844 max fee per blob gas.
    #[serde(default)]
    pub max_fee_per_blob_gas: Option<U256>,
}

/// Deserializes a JSON array of hex strings (`["0x", "0xdeadbeef"]`) into a
/// `Vec<Bytes>`. `"0x"` decodes to an empty `Bytes`.
fn deser_vec_hex_bytes<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<Bytes>, D::Error> {
    use serde::de::Error;
    let raw: Vec<String> = Vec::deserialize(d)?;
    raw.into_iter()
        .map(|s| {
            let stripped = s.strip_prefix("0x").unwrap_or(&s);
            hex::decode(stripped)
                .map(Bytes::from)
                .map_err(D::Error::custom)
        })
        .collect()
}

/// Deserializes a JSON array of hex-or-decimal strings (`["0x5208", "21000"]`)
/// into a `Vec<u64>`. EF tests encode per-subtest gas limits this way.
fn deser_vec_u64_hex_dec<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<u64>, D::Error> {
    use serde::de::Error;
    let raw: Vec<String> = Vec::deserialize(d)?;
    raw.into_iter()
        .map(|s| {
            let trimmed = s.trim_start_matches("0x");
            if trimmed.len() != s.len() {
                u64::from_str_radix(trimmed, 16)
            } else {
                trimmed.parse::<u64>()
            }
            .map_err(D::Error::custom)
        })
        .collect()
}

/// A single post-state vector entry (one subtest).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PostStateVector {
    /// Expected post-state root hash.
    pub hash: H256,
    /// Keccak of the RLP-encoded logs.
    pub logs: H256,
    #[serde(default)]
    pub expect_exception: Option<serde_json::Value>,
    /// Indexes selecting which data/gas/value item from the transaction template to use.
    pub indexes: SubtestIndexes,
}

/// Indexes into the transaction template arrays.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SubtestIndexes {
    pub data: usize,
    pub gas: usize,
    pub value: usize,
}

/// Deserializes an optional `u64` from a JSON string that is either `"0x..."` hex
/// or a decimal integer. Used for `currentExcessBlobGas`.
fn deser_u64_hex_or_dec_opt<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<u64>, D::Error> {
    use serde::de::Error;
    let opt: Option<String> = Option::deserialize(d)?;
    match opt {
        None => Ok(None),
        Some(s) if s.is_empty() => Ok(None),
        Some(s) => {
            let trimmed = s.trim_start_matches("0x");
            let v = if trimmed.len() != s.len() {
                u64::from_str_radix(trimmed, 16)
            } else {
                trimmed.parse::<u64>()
            };
            v.map(Some).map_err(D::Error::custom)
        }
    }
}

/// The chain config used when constructing Genesis from a StateTest.
#[allow(dead_code)]
pub fn default_statetest_chain_config() -> ChainConfig {
    crate::statetest::state_root::minimal_chain_config()
}
