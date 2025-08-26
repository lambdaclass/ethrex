use crate::modules::{
    deserialize::{
        deserialize_access_lists, deserialize_authorization_lists,
        deserialize_ef_post_value_indexes, deserialize_post,
        deserialize_transaction_expected_exception,
    },
    error::RunnerError,
};
use std::str::FromStr;

use ::bytes::Bytes;
use ethrex_common::{
    Address, H160, H256, U256,
    types::{AuthorizationTuple, Fork, Genesis, GenesisAccount, TxKind},
};
use ethrex_common::{
    serde_utils::{bytes, u64, u256},
    types::{BlobSchedule, ChainConfig},
};

use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
};

const DEFAULT_FORKS: [&str; 4] = ["Merge", "Shanghai", "Cancun", "Prague"];

/// `Tests` structure is the result of parsing a whole `.json` file from the EF tests. This file includes at
/// least one general test enviroment and different test cases inside each enviroment.
#[derive(Debug)]
pub struct Tests(pub Vec<Test>);

/// Custom deserialize function for a `.json file`.
impl<'de> Deserialize<'de> for Tests {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // A single .json file can contain more than one Test.
        let mut ef_tests = Vec::new();
        // This will get a HashMap where the first String key is the name of the test in the file
        // and the String key in the inner HashMap represents the name of a particular field inside
        // the test.
        let test_file: HashMap<String, HashMap<String, serde_json::Value>> =
            HashMap::deserialize(deserializer)?;

        // Every test object that appears (identified with a String key, its name) will end up represented
        // by a `Test`.
        for test_name in test_file.keys() {
            let test_data = test_file
                .get(test_name)
                .ok_or(serde::de::Error::missing_field("test data value"))?;

            // Obtain the value of the `transaction` field in the JSON.
            let tx_field = test_data
                .get("transaction")
                .ok_or(serde::de::Error::missing_field("transaction"))?
                .clone();
            // Parse the field value as a `RawTransaction`.
            let raw_tx: RawTransaction = serde_json::from_value(tx_field).map_err(|err| {
                serde::de::Error::custom(format!(
                    "Failed to deserialize `transaction` field in test {}. Serde error: {}",
                    test_name, err
                ))
            })?;
            // Obtain the value of the `post` field in the JSON.
            let post_field = test_data
                .get("post")
                .ok_or(serde::de::Error::missing_field("post"))?
                .clone();
            // Parse the field value as a `RawPost`.
            let post: RawPost = serde_json::from_value(post_field).map_err(|err| {
                serde::de::Error::custom(format!(
                    "Failed to deserialize `post` field in test {}. Serde error: {}",
                    test_name, err
                ))
            })?;

            let mut test_cases: Vec<TestCase> = Vec::new();
            // For every pair <fork, test_transaction> included in this test create a `TestCase`.
            // One fork can be used to execute more than one transaction, that will be given by the
            // different combinations of data, value and gas limit.
            for fork in post.forks.keys() {
                // We make sure we parse only the forks Ethrex supports (post Merge).
                if !DEFAULT_FORKS.contains(&(*fork).into()) {
                    continue;
                }
                let fork_test_cases = post.forks.get(fork).ok_or(serde::de::Error::custom(
                    "Failed to find fork in test post value",
                ))?;
                for case in fork_test_cases {
                    let test_case = Self::build_test_case(&raw_tx, fork, case)
                        .map_err(|e| serde::de::Error::custom(format!("{:?}", e)))?;
                    test_cases.push(test_case);
                }
            }
            // After we have obtained all the possible combinations of <fork, transaction> into test cases,
            // a `Test` is created, that includes the shared enviroment and all of the test cases that will be
            // executed under the same pre-conditions.
            let test = Self::build_test(test_name, test_data, test_cases)
                .map_err(|err| serde::de::Error::custom(err.to_string()))?;
            ef_tests.push(test);
        }
        Ok(Self(ef_tests))
    }
}

impl Tests {
    /// Returns a `Test` structure from the `.json` parsed data and the previously built
    /// test cases (<fork, transaction>).
    fn build_test(
        test_name: &str,
        test_data: &HashMap<String, Value>,
        test_cases: Vec<TestCase>,
    ) -> Result<Test, serde_json::Error> {
        // Obtain the value of the `info` field in the JSON.
        let info_field = test_data
            .get("_info")
            .ok_or(serde::de::Error::missing_field("_info"))?;
        // Parse the field value as `Info`.
        let test_info = serde_json::from_value(info_field.clone()).map_err(|err| {
            serde::de::Error::custom(format!(
                "Failed to deserialize `info` field in test {}. Serde error: {}",
                test_name, err
            ))
        })?;
        // Obtain the value of the `env` field in the JSON.
        let env_field = test_data
            .get("env")
            .ok_or(serde::de::Error::missing_field("env"))?;
        // Parse the field value as `Env`.
        let test_env = serde_json::from_value(env_field.clone()).map_err(|err| {
            serde::de::Error::custom(format!(
                "Failed to deserialize `env` field in test {}. Serde error: {}",
                test_name, err
            ))
        })?;
        // Obtain the value of the `pre` field in the JSON.
        let pre_field = test_data
            .get("pre")
            .ok_or(serde::de::Error::missing_field("pre"))?;
        // Parse the field value as a `HashMap<Address, AccountState>`.
        let test_pre = serde_json::from_value(pre_field.clone()).map_err(|err| {
            serde::de::Error::custom(format!(
                "Failed to deserialize `pre` field in test {}. Serde error: {}",
                test_name, err
            ))
        })?;

        let test = Test {
            name: test_name.to_string(),
            path: PathBuf::default(), // Test file path gets updated afterwards, cannot be known from here.
            _info: test_info,
            env: test_env,
            pre: test_pre,
            test_cases,
        };
        Ok(test)
    }

    /// Builds a `TestCase` struct from previously parsed `.json` data.
    fn build_test_case(
        raw_tx: &RawTransaction,
        fork: &Fork,
        raw_post: &RawPostValue,
    ) -> Result<TestCase, RunnerError> {
        let data_index = raw_post
            .indexes
            .get("data")
            .ok_or(RunnerError::FailedToGetIndexValue("value".to_string()))?
            .as_usize();
        let value_index = raw_post
            .indexes
            .get("value")
            .ok_or(RunnerError::FailedToGetIndexValue("value".to_string()))?
            .as_usize();
        let gas_index = raw_post
            .indexes
            .get("gas")
            .ok_or(RunnerError::FailedToGetIndexValue("value".to_string()))?
            .as_usize();
        let access_list_raw = raw_tx.access_lists.clone().unwrap_or_default();
        let mut access_list = Vec::new();
        if !access_list_raw.is_empty() {
            access_list = access_list_raw[data_index].clone();
        }
        let test_case = TestCase {
            vector: (data_index, value_index, gas_index),
            data: raw_tx.data[data_index].clone(),
            value: raw_tx.value[value_index],
            gas: raw_tx.gas_limit[gas_index],
            tx_bytes: raw_post.txbytes.clone(),
            gas_price: raw_tx.gas_price,
            nonce: raw_tx.nonce,
            secret_key: raw_tx.secret_key,
            sender: raw_tx.sender,
            max_fee_per_blob_gas: raw_tx.max_fee_per_blob_gas,
            max_fee_per_gas: raw_tx.max_fee_per_gas,
            max_priority_fee_per_gas: raw_tx.max_priority_fee_per_gas,
            to: raw_tx.to.clone(),
            fork: *fork,
            authorization_list: raw_tx.authorization_list.clone(),
            access_list,
            blob_versioned_hashes: raw_tx.blob_versioned_hashes.clone().unwrap_or_default(),
            post: Post {
                hash: raw_post.hash,
                logs: raw_post.logs,
                state: raw_post.state.clone(),
                expected_exceptions: raw_post.expect_exception.clone(),
            },
        };
        Ok(test_case)
    }
}

/// This structure represents the general enviroment for a set of specific test cases. It includes all the
/// information that is shared among the test cases.
#[derive(Debug, Clone)]
pub struct Test {
    pub name: String,  // The name of the test object inside the .json file.
    pub path: PathBuf, // The path of the .json file the Test can be found at.
    pub _info: Info,   // General information about the test.
    pub env: Env,      // The block enviroment before the test transaction happens.
    pub pre: HashMap<Address, AccountState>, // The accounts state previous to the test transaction.
    pub test_cases: Vec<TestCase>, // A vector of specific cases to be tested under these conditions (transactions).
}

fn get_chain_config_from_fork(fork: &Fork) -> ChainConfig {
    let mut basic_chain_config = ChainConfig {
        chain_id: 1,
        homestead_block: Some(0),
        dao_fork_block: Some(0),
        dao_fork_support: true,
        eip150_block: Some(0),
        eip155_block: Some(0),
        eip158_block: Some(0),
        byzantium_block: Some(0),
        constantinople_block: Some(0),
        petersburg_block: Some(0),
        istanbul_block: Some(0),
        muir_glacier_block: Some(0),
        berlin_block: Some(0),
        london_block: Some(0),
        arrow_glacier_block: Some(0),
        gray_glacier_block: Some(0),
        merge_netsplit_block: Some(0),
        shanghai_time: None, // Time for forks will be set.
        cancun_time: None,
        prague_time: None,
        verkle_time: None,
        osaka_time: None,
        terminal_total_difficulty: Some(0),
        terminal_total_difficulty_passed: true,
        blob_schedule: BlobSchedule::default(), // This will be set with the fork
        deposit_contract_address: H160::from_str("0x4242424242424242424242424242424242424242")
            .unwrap(), // Doesn't matter
    };

    if *fork < Fork::Paris {
        panic!("We don't support pre Merge forks, what are you doing here?");
    }
    if *fork >= Fork::Shanghai {
        basic_chain_config.shanghai_time = Some(0);
    }
    if *fork >= Fork::Cancun {
        basic_chain_config.cancun_time = Some(0);
    }
    if *fork >= Fork::Prague {
        basic_chain_config.prague_time = Some(0);
    }
    if *fork >= Fork::Osaka {
        basic_chain_config.osaka_time = Some(0);
    }

    basic_chain_config
}

pub fn genesis_from_test_and_fork(test: &Test, fork: &Fork) -> Genesis {
    let chain_config = get_chain_config_from_fork(fork);

    Genesis {
        alloc: {
            let mut alloc = BTreeMap::new();
            for (account, account_state) in &test.pre {
                alloc.insert(*account, GenesisAccount::from(account_state));
            }
            alloc
        },
        coinbase: test.env.current_coinbase,
        difficulty: test.env.current_difficulty,
        gas_limit: test.env.current_gas_limit,
        mix_hash: test.env.current_random.unwrap_or_default(),
        timestamp: 0,
        base_fee_per_gas: test.env.current_base_fee.map(|v| v.as_u64()),
        excess_blob_gas: test.env.current_excess_blob_gas.map(|v| v.as_u64()),
        config: chain_config,
        ..Default::default()
    }
}

// impl From<&Test> for Genesis {
//     fn from(test: &Test) -> Self {

//         let algo = test.

//         let config = ChainConfig {
//             chain_id: todo!(),
//             homestead_block: 0,
//             dao_fork_block: 0,
//             dao_fork_support: 0,
//             eip150_block: 0,
//             eip155_block: 0,
//             eip158_block: 0,
//             byzantium_block: 0,
//             constantinople_block: 0,
//             petersburg_block: 0,
//             istanbul_block: 0,
//             muir_glacier_block: 0,
//             berlin_block: 0,
//             london_block: 0,
//             arrow_glacier_block: 0,
//             gray_glacier_block: 0,
//             merge_netsplit_block: 0,
//             shanghai_time: 0,
//             cancun_time: 0,
//             prague_time: 0,
//             verkle_time: 0,
//             osaka_time: 0,
//             terminal_total_difficulty: 0,
//             terminal_total_difficulty_passed: 0,
//             blob_schedule: BlobSchedule::default(),
//             deposit_contract_address: todo!(),
//         }
//         Genesis {
//             alloc: {
//                 let mut alloc = BTreeMap::new();
//                 for (account, account_state) in &test.pre {
//                     alloc.insert(*account, GenesisAccount::from(account_state));
//                 }
//                 alloc
//             },
//             coinbase: test.env.current_coinbase,
//             difficulty: test.env.current_difficulty,
//             gas_limit: test.env.current_gas_limit,
//             mix_hash: test.env.current_random.unwrap_or_default(),
//             timestamp: 0,
//             base_fee_per_gas: test.env.current_base_fee.map(|v| v.as_u64()),
//             excess_blob_gas: test.env.current_excess_blob_gas.map(|v| v.as_u64()),
//             ..Default::default()
//         }
//     }
// }

/// General information about the test. Matches the `_info` field in the `.json` file.
#[derive(Debug, Deserialize, Clone)]
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

/// Block enviroment previous to the execution of the transaction. Matches the `env` field in the
/// `.json` file.
#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct Env {
    #[serde(default, deserialize_with = "u256::deser_hex_str_opt")]
    pub current_base_fee: Option<U256>,
    pub current_coinbase: Address,
    #[serde(deserialize_with = "u256::deser_hex_str")]
    pub current_difficulty: U256,
    #[serde(default, deserialize_with = "u256::deser_hex_str_opt")]
    pub current_excess_blob_gas: Option<U256>,
    #[serde(with = "u64::hex_str")]
    pub current_gas_limit: u64,
    #[serde(deserialize_with = "u256::deser_hex_str")]
    pub current_number: U256,
    pub current_random: Option<H256>,
    #[serde(deserialize_with = "u256::deser_hex_str")]
    pub current_timestamp: U256,
}

/// This structure represents a specific test case under general test conditions (`Test` struct). It is mainly
/// composed of a particular transaction combined with a particular fork. It includes the expected post state
/// after the transaction is executed.
#[derive(Deserialize, Debug, Clone)]
pub struct TestCase {
    pub vector: (usize, usize, usize),
    pub data: Bytes,
    pub gas: u64,
    pub value: U256,
    pub tx_bytes: Bytes,
    pub gas_price: Option<U256>,
    pub max_fee_per_gas: Option<U256>,
    pub max_priority_fee_per_gas: Option<U256>,
    pub max_fee_per_blob_gas: Option<U256>,
    pub nonce: u64,
    pub secret_key: H256,
    pub sender: Address,
    pub to: TxKind,
    pub fork: Fork,
    pub post: Post,
    pub blob_versioned_hashes: Vec<H256>,
    pub access_list: Vec<AccessListItem>,
    pub authorization_list: Option<Vec<AuthorizationListTuple>>,
}

/// Indicates the expected post state that should be obtained after executing a test case.
#[derive(Debug, Deserialize, Clone)]
pub struct Post {
    pub hash: H256,                                    // Expected post root hash.
    pub logs: H256,                                    // Expected output logs.
    pub state: Option<HashMap<Address, AccountState>>, // For new tests, the state field indicates the expected state of the involved accounts after executing the transaction.
    pub expected_exceptions: Option<Vec<TransactionExpectedException>>, // Expected exceptions. The output exception should match one of these.
}

/// The state an involved account is expected to have after executing the test case transaction.
#[derive(Debug, Deserialize, Clone)]
pub struct AccountState {
    #[serde(deserialize_with = "u256::deser_hex_str")]
    pub balance: U256,
    #[serde(with = "bytes")]
    pub code: Bytes,
    #[serde(with = "u64::hex_str")]
    pub nonce: u64,
    #[serde(with = "u256::hashmap")]
    pub storage: HashMap<U256, U256>,
}

impl From<&AccountState> for GenesisAccount {
    fn from(value: &AccountState) -> Self {
        Self {
            code: value.code.clone(),
            storage: value.storage.clone(),
            balance: value.balance,
            nonce: value.nonce,
        }
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
    Other,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AccessListItem {
    pub address: Address,
    pub storage_keys: Vec<H256>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizationListTuple {
    #[serde(deserialize_with = "u256::deser_hex_str")]
    pub chain_id: U256,
    pub address: Address,
    #[serde(with = "u64::hex_str")]
    pub nonce: u64,
    #[serde(deserialize_with = "u256::deser_hex_str")]
    pub v: U256,
    #[serde(deserialize_with = "u256::deser_hex_str")]
    pub r: U256,
    #[serde(deserialize_with = "u256::deser_hex_str")]
    pub s: U256,
    pub signer: Option<Address>,
}
impl AuthorizationListTuple {
    pub fn into_authorization_tuple(self) -> AuthorizationTuple {
        AuthorizationTuple {
            chain_id: self.chain_id,
            address: self.address,
            nonce: self.nonce,
            y_parity: self.v,
            r_signature: self.r,
            s_signature: self.s,
        }
    }
}

// ---- Raw structures ----
// Exactly as they are defined in the .json test files, mainly used for parsing purposes or
// as intermediate structures.

#[derive(Debug, Deserialize)]
pub struct RawPost {
    #[serde(flatten)]
    #[serde(deserialize_with = "deserialize_post")]
    pub forks: HashMap<Fork, Vec<RawPostValue>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RawPostValue {
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
    #[serde(default, with = "bytes")]
    pub txbytes: Bytes,
    pub state: Option<HashMap<Address, AccountState>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawTransaction {
    #[serde(with = "bytes::vec")]
    pub data: Vec<Bytes>,
    #[serde(deserialize_with = "u64::hex_str::deser_vec")]
    pub gas_limit: Vec<u64>,
    #[serde(default, deserialize_with = "u256::deser_hex_str_opt")]
    pub gas_price: Option<U256>,
    #[serde(with = "u64::hex_str")]
    pub nonce: u64,
    pub secret_key: H256,
    pub sender: Address,
    pub to: TxKind,
    #[serde(with = "u256::vec")]
    pub value: Vec<U256>,
    #[serde(default, deserialize_with = "u256::deser_hex_str_opt")]
    pub max_fee_per_gas: Option<U256>,
    #[serde(default, deserialize_with = "u256::deser_hex_str_opt")]
    pub max_priority_fee_per_gas: Option<U256>,
    #[serde(default, deserialize_with = "u256::deser_hex_str_opt")]
    pub max_fee_per_blob_gas: Option<U256>,
    pub blob_versioned_hashes: Option<Vec<H256>>,
    #[serde(default, deserialize_with = "deserialize_access_lists")]
    pub access_lists: Option<Vec<Vec<AccessListItem>>>,
    #[serde(default, deserialize_with = "deserialize_authorization_lists")]
    pub authorization_list: Option<Vec<AuthorizationListTuple>>,
}
