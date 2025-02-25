use crate::{
    deserialize::{
        deserialize_access_lists, deserialize_authorization_lists,
        deserialize_ef_post_value_indexes, deserialize_h256_vec_optional_safe,
        deserialize_hex_bytes, deserialize_hex_bytes_vec, deserialize_post,
        deserialize_transaction_expected_exception, deserialize_u256_optional_safe,
        deserialize_u256_safe, deserialize_u256_valued_hashmap_safe, deserialize_u256_vec_safe,
        deserialize_u64_safe, deserialize_u64_vec_safe,
    },
    report::TestVector,
};
use bytes::Bytes;
use ethrex_common::{
    types::{Fork, Genesis, GenesisAccount, TxKind},
    Address, H256, U256,
};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};

#[derive(Debug)]
pub struct EFTests(pub Vec<EFTest>);

#[derive(Debug)]
pub struct EFTest {
    pub name: String,
    pub dir: String,
    pub _info: EFTestInfo,
    pub env: EFTestEnv,
    pub post: EFTestPost,
    pub pre: EFTestPre,
    pub transactions: HashMap<TestVector, EFTestTransaction>,
}

impl From<&EFTest> for Genesis {
    fn from(test: &EFTest) -> Self {
        Genesis {
            alloc: {
                let mut alloc = BTreeMap::new();
                for (account, ef_test_pre_value) in test.pre.0.iter() {
                    alloc.insert(*account, ef_test_pre_value.into());
                }
                alloc
            },
            coinbase: test.env.current_coinbase,
            difficulty: test.env.current_difficulty,
            gas_limit: test.env.current_gas_limit,
            mix_hash: test.env.current_random.unwrap_or_default(),
            timestamp: test.env.current_timestamp.as_u64(),
            base_fee_per_gas: test.env.current_base_fee.map(|v| v.as_u64()),
            excess_blob_gas: test.env.current_excess_blob_gas.map(|v| v.as_u64()),
            ..Default::default()
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct EFTestInfo {
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(rename = "filling-rpc-server", default)]
    pub filling_rpc_server: Option<String>,
    #[serde(rename = "filling-tool-version", default)]
    pub filling_tool_version: Option<String>,
    #[serde(rename = "generatedTestHash", default)]
    pub generated_test_hash: Option<H256>,
    #[serde(default)]
    pub labels: Option<HashMap<u64, String>>,
    #[serde(default)]
    pub lllcversion: Option<String>,
    #[serde(default)]
    pub solidity: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(rename = "sourceHash", default)]
    pub source_hash: Option<H256>,

    // These fields are implemented in the new version of the test vectors (Prague).
    #[serde(rename = "hash", default)]
    pub hash: Option<H256>,
    #[serde(rename = "filling-transition-tool", default)]
    pub filling_transition_tool: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(rename = "fixture_format", default)]
    pub fixture_format: Option<String>,
    #[serde(rename = "reference-spec", default)]
    pub reference_spec: Option<String>,
    #[serde(rename = "reference-spec-version", default)]
    pub reference_spec_version: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EFTestEnv {
    #[serde(default, deserialize_with = "deserialize_u256_optional_safe")]
    pub current_base_fee: Option<U256>,
    pub current_coinbase: Address,
    #[serde(deserialize_with = "deserialize_u256_safe")]
    pub current_difficulty: U256,
    #[serde(default, deserialize_with = "deserialize_u256_optional_safe")]
    pub current_excess_blob_gas: Option<U256>,
    #[serde(deserialize_with = "deserialize_u64_safe")]
    pub current_gas_limit: u64,
    #[serde(deserialize_with = "deserialize_u256_safe")]
    pub current_number: U256,
    pub current_random: Option<H256>,
    #[serde(deserialize_with = "deserialize_u256_safe")]
    pub current_timestamp: U256,
}

#[derive(Debug, Deserialize)]
pub struct EFTestPost {
    #[serde(flatten)]
    #[serde(deserialize_with = "deserialize_post")]
    pub forks: HashMap<Fork, Vec<EFTestPostValue>>,
}

impl EFTestPost {
    pub fn vector_post_value(&self, vector: &TestVector, fork: Fork) -> EFTestPostValue {
        let post_values = self.forks.get(&fork).unwrap();
        Self::find_vector_post_value(post_values, vector).unwrap()
    }

    fn find_vector_post_value(
        values: &[EFTestPostValue],
        vector: &TestVector,
    ) -> Option<EFTestPostValue> {
        values
            .iter()
            .find(|v| {
                let data_index = v.indexes.get("data").unwrap().as_usize();
                let gas_limit_index = v.indexes.get("gas").unwrap().as_usize();
                let value_index = v.indexes.get("value").unwrap().as_usize();
                vector == &(data_index, gas_limit_index, value_index)
            })
            .cloned()
    }

    pub fn has_vector_for_fork(&self, vector: &TestVector, fork: Fork) -> bool {
        self.forks
            .get(&fork)
            .and_then(|values| Self::find_vector_post_value(values, vector))
            .is_some()
    }
}

#[derive(Debug, Deserialize, Clone)]
pub enum TransactionExpectedException {
    InitcodeSizeExceeded,
    NonceIsMax,
    Type3TxBlobCountExceeded,
    Type3TxZeroBlobs,
    Type3TxContractCreation,
    Type3TxInvalidBlobVersionedHash,
    Type4TxContractCreation,
    IntrinsicGasTooLow,
    InsufficientAccountFunds,
    SenderNotEoa,
    PriorityGreaterThanMaxFeePerGas,
    GasAllowanceExceeded,
    InsufficientMaxFeePerGas,
    RlpInvalidValue,
    GasLimitPriceProductOverflow,
    Type3TxPreFork,
    InsufficientMaxFeePerBlobGas,
}

#[derive(Debug, Deserialize, Clone)]
pub struct EFTestPostValue {
    #[serde(
        rename = "expectException",
        default,
        deserialize_with = "deserialize_transaction_expected_exception"
    )]
    pub expect_exception: Option<Vec<TransactionExpectedException>>,
    pub hash: H256,
    #[serde(deserialize_with = "deserialize_ef_post_value_indexes")]
    pub indexes: HashMap<String, U256>,
    pub logs: H256,
    // we add the default because some tests don't have this field
    #[serde(default, deserialize_with = "deserialize_hex_bytes")]
    pub txbytes: Bytes,
}

#[derive(Debug, Deserialize)]
pub struct EFTestPre(pub HashMap<Address, EFTestPreValue>);

#[derive(Debug, Deserialize)]
pub struct EFTestPreValue {
    #[serde(deserialize_with = "deserialize_u256_safe")]
    pub balance: U256,
    #[serde(deserialize_with = "deserialize_hex_bytes")]
    pub code: Bytes,
    #[serde(deserialize_with = "deserialize_u256_safe")]
    pub nonce: U256,
    #[serde(deserialize_with = "deserialize_u256_valued_hashmap_safe")]
    pub storage: HashMap<U256, U256>,
}

impl From<&EFTestPreValue> for GenesisAccount {
    fn from(value: &EFTestPreValue) -> Self {
        Self {
            code: value.code.clone(),
            storage: value
                .storage
                .iter()
                .map(|(k, v)| (H256::from_slice(&k.to_big_endian()), *v))
                .collect(),
            balance: value.balance,
            nonce: value.nonce.as_u64(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct EFTestAccessListItem {
    pub address: Address,
    pub storage_keys: Vec<H256>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct EFTestAuthorizationListTuple {
    #[serde(deserialize_with = "deserialize_u256_safe")]
    pub chain_id: U256,
    pub address: Address,
    #[serde(deserialize_with = "deserialize_u64_safe")]
    pub nonce: u64,
    #[serde(deserialize_with = "deserialize_u256_safe")]
    pub v: U256,
    #[serde(deserialize_with = "deserialize_u256_safe")]
    pub r: U256,
    #[serde(deserialize_with = "deserialize_u256_safe")]
    pub s: U256,
    pub signer: Option<Address>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EFTestRawTransaction {
    #[serde(deserialize_with = "deserialize_hex_bytes_vec")]
    pub data: Vec<Bytes>,
    #[serde(deserialize_with = "deserialize_u64_vec_safe")]
    pub gas_limit: Vec<u64>,
    #[serde(default, deserialize_with = "deserialize_u256_optional_safe")]
    pub gas_price: Option<U256>,
    #[serde(deserialize_with = "deserialize_u256_safe")]
    pub nonce: U256,
    pub secret_key: H256,
    pub sender: Address,
    pub to: TxKind,
    #[serde(deserialize_with = "deserialize_u256_vec_safe")]
    pub value: Vec<U256>,
    #[serde(default, deserialize_with = "deserialize_u256_optional_safe")]
    pub max_fee_per_gas: Option<U256>,
    #[serde(default, deserialize_with = "deserialize_u256_optional_safe")]
    pub max_priority_fee_per_gas: Option<U256>,
    #[serde(default, deserialize_with = "deserialize_u256_optional_safe")]
    pub max_fee_per_blob_gas: Option<U256>,
    #[serde(default, deserialize_with = "deserialize_h256_vec_optional_safe")]
    pub blob_versioned_hashes: Option<Vec<H256>>,
    #[serde(default, deserialize_with = "deserialize_access_lists")]
    pub access_lists: Option<Vec<Vec<EFTestAccessListItem>>>,
    #[serde(default, deserialize_with = "deserialize_authorization_lists")]
    pub authorization_list: Option<Vec<EFTestAuthorizationListTuple>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EFTestTransaction {
    pub data: Bytes,
    pub gas_limit: u64,
    pub gas_price: Option<U256>,
    #[serde(deserialize_with = "deserialize_u256_safe")]
    pub nonce: U256,
    pub secret_key: H256,
    pub sender: Address,
    pub to: TxKind,
    pub value: U256,
    pub max_fee_per_gas: Option<U256>,
    pub max_priority_fee_per_gas: Option<U256>,
    pub max_fee_per_blob_gas: Option<U256>,
    pub blob_versioned_hashes: Vec<H256>,
    pub access_list: Vec<EFTestAccessListItem>,
    pub authorization_list: Option<Vec<EFTestAuthorizationListTuple>>,
}
