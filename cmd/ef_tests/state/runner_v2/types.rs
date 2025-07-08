use crate::{
    deserialize::{
        deserialize_access_lists, deserialize_authorization_lists,
        deserialize_ef_post_value_indexes, deserialize_h256_vec_optional_safe,
        deserialize_hex_bytes, deserialize_hex_bytes_vec,
        deserialize_transaction_expected_exception, deserialize_u64_safe, deserialize_u64_vec_safe,
        deserialize_u256_optional_safe, deserialize_u256_safe,
        deserialize_u256_valued_hashmap_safe, deserialize_u256_vec_safe,
    },
    runner_v2::deserializer::deserialize_post,
};

use crate::types::{
    EFTestAccessListItem, EFTestAuthorizationListTuple, TransactionExpectedException,
};
use bytes::Bytes;

use ethrex_common::{
    Address, H256, U256,
    types::{Fork, TxKind},
};

use serde::Deserialize;
use std::{collections::HashMap, fmt::Display};

#[derive(Debug)]
pub struct Tests(pub Vec<Test>);

impl Display for Tests {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for test in &self.0 {
            for test_case in &test.test_cases {
                writeln!(f, "{}", test_case)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct Test {
    pub name: String,
    pub dir: String,
    pub _info: Info,
    pub env: Env,
    pub pre: HashMap<Address, AccountState>,
    pub test_cases: Vec<TestCase>,
}

#[derive(Debug, Deserialize)]
pub struct TestPost {
    #[serde(flatten)]
    #[serde(deserialize_with = "deserialize_post")]
    pub forks: HashMap<Fork, Vec<TestPostValue>>,
}
#[derive(Debug, Deserialize, Clone)]
pub struct TestPostValue {
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
    pub state: HashMap<Address, AccountState>,
}

impl<'de> Deserialize<'de> for Tests {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut ef_tests = Vec::new();
        // This will get a HashMap where the first String key is the name of the test in the file
        // and the second String key is the name of the field.
        let aux: HashMap<String, HashMap<String, serde_json::Value>> =
            HashMap::deserialize(deserializer)?;

        for test_name in aux.keys() {
            let test_data = aux
                .get(test_name)
                .ok_or(serde::de::Error::missing_field("test data value"))?;

            let tx_field_value = test_data
                .get("transaction")
                .ok_or(serde::de::Error::missing_field("transaction"))?
                .clone();

            let raw_tx_field: EFTestRawTransaction = serde_json::from_value(tx_field_value)
                .map_err(|err| {
                    serde::de::Error::custom(format!(
                        "error deserializing test \"{test_name}\", \"transaction\" field: {err}"
                    ))
                })?;
            let possible_data = raw_tx_field.data;
            let possible_values = raw_tx_field.value;
            let possible_gas_limit = raw_tx_field.gas_limit;

            let post: TestPost = serde_json::from_value(
                test_data
                    .get("post")
                    .ok_or(serde::de::Error::missing_field("post"))?
                    .clone(),
            )
            .map_err(|err| {
                serde::de::Error::custom(format!(
                    "error deserializing test \"{test_name}\", \"transaction\" field: {err}"
                ))
            })?;
            let mut test_cases: Vec<TestCase> = Vec::new();
            for fork in post.forks.keys() {
                let fork_test_cases = post.forks.get(fork).unwrap();
                for case in fork_test_cases {
                    let test_case = TestCase {
                        data: possible_data[case.indexes.get("data").unwrap().as_usize()].clone(),
                        value: possible_values[case.indexes.get("value").unwrap().as_usize()],
                        gas: possible_gas_limit[case.indexes.get("gas").unwrap().as_usize()],
                        tx_bytes: case.txbytes.clone(),
                        gas_price: raw_tx_field.gas_price,
                        nonce: raw_tx_field.nonce,
                        secret_key: raw_tx_field.secret_key,
                        sender: raw_tx_field.sender,
                        to: raw_tx_field.to.clone(),
                        fork: *fork,
                        post: TestCasePost {
                            hash: case.hash,
                            logs: case.logs,
                            state: case.state.clone(),
                            expected_exception: case.expect_exception.clone(),
                        },
                    };
                    test_cases.push(test_case);
                }
            }
            let test = Test {
                name: test_name.to_string(),
                dir: "".to_string(),
                _info: serde_json::from_value(
                    test_data
                        .get("_info")
                        .ok_or(serde::de::Error::missing_field("_info"))?
                        .clone(),
                )
                .map_err(|err| {
                    serde::de::Error::custom(format!(
                        "error deserializing test \"{test_name}\", \"_info\" field: {err}"
                    ))
                })?,
                env: serde_json::from_value(
                    test_data
                        .get("env")
                        .ok_or(serde::de::Error::missing_field("env"))?
                        .clone(),
                )
                .map_err(|err| {
                    serde::de::Error::custom(format!(
                        "error deserializing test \"{test_name}\", \"env\" field: {err}"
                    ))
                })?,
                pre: serde_json::from_value(
                    test_data
                        .get("pre")
                        .ok_or(serde::de::Error::missing_field("pre"))?
                        .clone(),
                )
                .map_err(|err| {
                    serde::de::Error::custom(format!(
                        "error deserializing test \"{test_name}\", \"pre\" field: {err}"
                    ))
                })?,
                test_cases,
            };
            println!("A ver el prestate--");
            for key in test.pre.keys() {
                println!("Key: {}, state {}", key, test.pre.get(key).unwrap());
            }
            ef_tests.push(test);
        }
        Ok(Self(ef_tests))
    }
}

#[derive(Deserialize, Debug)]
pub struct TestCase {
    pub data: Bytes,
    pub gas: u64,
    pub value: U256,
    pub tx_bytes: Bytes,
    pub gas_price: Option<U256>,
    pub nonce: u64,
    pub secret_key: H256,
    pub sender: Address,
    pub to: TxKind,
    pub fork: Fork,
    pub post: TestCasePost,
}

impl Display for TestCase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "\nTest case:
                data: 0x{},
                gas: {},
                value: {},
                tx_bytes: 0x{},
                gas_price: {:?},
                nonce: {},
                secret_key: {},
                sender: {},
                to: {:#?},
                fork: {:#?},
                post: {}
                ",
            hex::encode(self.data.clone()),
            self.gas,
            self.value,
            hex::encode(self.tx_bytes.clone()),
            self.gas_price,
            self.nonce,
            self.secret_key,
            self.sender,
            self.to,
            self.fork,
            self.post
        )?;
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
pub struct TestCasePost {
    pub hash: H256,
    pub logs: H256,
    pub state: HashMap<Address, AccountState>,
    pub expected_exception: Option<Vec<TransactionExpectedException>>,
}

impl Display for TestCasePost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "testcasepost:
        hash: {},
        logs: {},
        expected_exception: {:?},
        ",
            self.hash, self.logs, self.expected_exception
        )?;
        for addr in self.state.keys() {
            writeln!(
                f,
                "Address: {}, state: {}",
                addr,
                self.state.get(addr).unwrap()
            )?;
        }
        Ok(())
    }
}

impl Display for AccountState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "
        balance: {},
        code: 0x{},
        nonce: {},
        storage: {:?}
        ",
            self.balance,
            hex::encode(self.code.clone()),
            self.nonce,
            self.storage
        )?;
        Ok(())
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct AccountState {
    #[serde(deserialize_with = "deserialize_u256_safe")]
    pub balance: U256,
    #[serde(deserialize_with = "deserialize_hex_bytes")]
    pub code: Bytes,
    #[serde(deserialize_with = "deserialize_u64_safe")]
    pub nonce: u64,
    #[serde(deserialize_with = "deserialize_u256_valued_hashmap_safe")]
    pub storage: HashMap<U256, U256>,
}

#[derive(Debug, Deserialize)]
pub struct Info {
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
pub struct Env {
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
#[serde(rename_all = "camelCase")]
pub struct EFTestRawTransaction {
    #[serde(deserialize_with = "deserialize_hex_bytes_vec")]
    pub data: Vec<Bytes>,
    #[serde(deserialize_with = "deserialize_u64_vec_safe")]
    pub gas_limit: Vec<u64>,
    #[serde(default, deserialize_with = "deserialize_u256_optional_safe")]
    pub gas_price: Option<U256>,
    #[serde(deserialize_with = "deserialize_u64_safe")]
    pub nonce: u64,
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
