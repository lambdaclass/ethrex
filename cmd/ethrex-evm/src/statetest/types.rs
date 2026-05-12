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
    pub secret_key: H256,
    pub to: Option<Address>,
    pub value: Vec<U256>,
    pub max_fee_per_gas: Option<U256>,
    pub max_priority_fee_per_gas: Option<U256>,
    // Phase 4: replace with structured `Vec<Vec<AccessListItem>>` + a custom
    // deserializer once the statetest CLI starts constructing real txs.
    #[serde(default)]
    pub access_lists: Vec<serde_json::Value>,
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

/// The chain config used when constructing Genesis from a StateTest.
/// Currently unused by the Phase 3 helper but will be used in Phase 4.
#[allow(dead_code)]
pub fn default_statetest_chain_config() -> ChainConfig {
    crate::statetest::state_root::minimal_chain_config()
}
