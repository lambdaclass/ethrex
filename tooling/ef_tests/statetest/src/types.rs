use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;

use ethrex_common::{
    Address, Bytes, H160, H256, U256,
    serde_utils::{bytes, u64, u256},
    types::{AuthorizationTuple, BlobSchedule, ChainConfig, Fork, TxKind},
};
use serde::Deserialize;
use serde_json::Value;

/// Forks we support (post-Merge only).
const SUPPORTED_FORKS: [&str; 5] = ["Merge", "Shanghai", "Cancun", "Prague", "Amsterdam"];

// ---- Top-level fixture structures ----

/// A single JSON file can contain multiple tests. Each test shares an
/// environment and pre-state, with multiple test cases (fork x tx combos).
#[derive(Debug, Clone)]
pub struct Test {
    pub name: String,
    pub path: PathBuf,
    pub env: Env,
    pub pre: HashMap<Address, AccountState>,
    pub test_cases: Vec<TestCase>,
}

/// Wrapper for deserializing the top-level JSON object.
#[derive(Debug)]
pub struct Tests(pub Vec<Test>);

impl<'de> Deserialize<'de> for Tests {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let test_file: HashMap<String, HashMap<String, Value>> =
            HashMap::deserialize(deserializer)?;
        let mut ef_tests = Vec::new();

        for test_name in test_file.keys() {
            let test_data = test_file
                .get(test_name)
                .ok_or(serde::de::Error::missing_field("test data value"))?;

            let tx_field = test_data
                .get("transaction")
                .ok_or(serde::de::Error::missing_field("transaction"))?
                .clone();
            let raw_tx: RawTransaction = serde_json::from_value(tx_field).map_err(|err| {
                serde::de::Error::custom(format!(
                    "Failed to deserialize `transaction` in test {}: {}",
                    test_name, err
                ))
            })?;

            let post_field = test_data
                .get("post")
                .ok_or(serde::de::Error::missing_field("post"))?
                .clone();
            let post: RawPost = serde_json::from_value(post_field).map_err(|err| {
                serde::de::Error::custom(format!(
                    "Failed to deserialize `post` in test {}: {}",
                    test_name, err
                ))
            })?;

            let mut test_cases = Vec::new();
            for fork in post.forks.keys() {
                if !SUPPORTED_FORKS.contains(&Into::<&str>::into(*fork)) {
                    continue;
                }
                let fork_cases = post.forks.get(fork).ok_or(serde::de::Error::custom(
                    "Failed to find fork in post value",
                ))?;
                for case in fork_cases {
                    let tc = build_test_case(&raw_tx, fork, case)
                        .map_err(|e| serde::de::Error::custom(e))?;
                    test_cases.push(tc);
                }
            }

            let env_field = test_data
                .get("env")
                .ok_or(serde::de::Error::missing_field("env"))?;
            let test_env: Env = serde_json::from_value(env_field.clone()).map_err(|err| {
                serde::de::Error::custom(format!(
                    "Failed to deserialize `env` in test {}: {}",
                    test_name, err
                ))
            })?;

            let pre_field = test_data
                .get("pre")
                .ok_or(serde::de::Error::missing_field("pre"))?;
            let test_pre: HashMap<Address, AccountState> =
                serde_json::from_value(pre_field.clone()).map_err(|err| {
                    serde::de::Error::custom(format!(
                        "Failed to deserialize `pre` in test {}: {}",
                        test_name, err
                    ))
                })?;

            ef_tests.push(Test {
                name: test_name.clone(),
                path: PathBuf::default(),
                env: test_env,
                pre: test_pre,
                test_cases,
            });
        }
        Ok(Tests(ef_tests))
    }
}

// ---- Environment ----

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
    #[serde(default, deserialize_with = "u256::deser_hex_str_opt")]
    pub slot_number: Option<U256>,
}

// ---- Account state ----

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

// ---- Test case (fork x data/gas/value combo) ----

#[derive(Debug, Clone)]
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
    pub authorization_list: Option<Vec<AuthorizationListTupleRaw>>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Post {
    pub hash: H256,
    pub logs: H256,
    pub state: Option<HashMap<Address, AccountState>>,
    pub expected_exceptions: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AccessListItem {
    pub address: Address,
    pub storage_keys: Vec<H256>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct AuthorizationListTupleRaw {
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

impl AuthorizationListTupleRaw {
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

// ---- Raw JSON structures ----

#[derive(Debug, Deserialize)]
struct RawPost {
    #[serde(flatten)]
    #[serde(deserialize_with = "deserialize_post")]
    forks: HashMap<Fork, Vec<RawPostValue>>,
}

#[derive(Debug, Deserialize, Clone)]
struct RawPostValue {
    #[serde(rename = "expectException", default)]
    expect_exception: Option<String>,
    hash: H256,
    #[serde(deserialize_with = "deserialize_ef_post_value_indexes")]
    indexes: HashMap<String, U256>,
    logs: H256,
    #[serde(default, with = "bytes")]
    txbytes: Bytes,
    state: Option<HashMap<Address, AccountState>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawTransaction {
    #[serde(with = "bytes::vec")]
    data: Vec<Bytes>,
    #[serde(deserialize_with = "u64::hex_str::deser_vec")]
    gas_limit: Vec<u64>,
    #[serde(default, deserialize_with = "u256::deser_hex_str_opt")]
    gas_price: Option<U256>,
    #[serde(with = "u64::hex_str")]
    nonce: u64,
    secret_key: H256,
    sender: Address,
    to: TxKind,
    #[serde(with = "u256::vec")]
    value: Vec<U256>,
    #[serde(default, deserialize_with = "u256::deser_hex_str_opt")]
    max_fee_per_gas: Option<U256>,
    #[serde(default, deserialize_with = "u256::deser_hex_str_opt")]
    max_priority_fee_per_gas: Option<U256>,
    #[serde(default, deserialize_with = "u256::deser_hex_str_opt")]
    max_fee_per_blob_gas: Option<U256>,
    blob_versioned_hashes: Option<Vec<H256>>,
    #[serde(default, deserialize_with = "deserialize_access_lists")]
    access_lists: Option<Vec<Vec<AccessListItem>>>,
    #[serde(default, deserialize_with = "deserialize_authorization_lists")]
    authorization_list: Option<Vec<AuthorizationListTupleRaw>>,
}

// ---- Test case builder ----

fn build_test_case(
    raw_tx: &RawTransaction,
    fork: &Fork,
    raw_post: &RawPostValue,
) -> Result<TestCase, String> {
    let data_index = raw_post
        .indexes
        .get("data")
        .ok_or("missing data index")?
        .as_usize();
    let value_index = raw_post
        .indexes
        .get("value")
        .ok_or("missing value index")?
        .as_usize();
    let gas_index = raw_post
        .indexes
        .get("gas")
        .ok_or("missing gas index")?
        .as_usize();

    let access_list_raw = raw_tx.access_lists.clone().unwrap_or_default();
    let access_list = if !access_list_raw.is_empty() {
        access_list_raw
            .get(data_index)
            .cloned()
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let expected_exceptions = raw_post.expect_exception.as_ref().map(|s| {
        s.split('|').map(|part| part.trim().to_string()).collect()
    });

    Ok(TestCase {
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
            expected_exceptions,
        },
    })
}

// ---- Chain config from fork ----

pub fn chain_config_for_fork(fork: &Fork) -> ChainConfig {
    let mut cfg = ChainConfig {
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
        shanghai_time: None,
        cancun_time: None,
        prague_time: None,
        verkle_time: None,
        osaka_time: None,
        terminal_total_difficulty: Some(0),
        terminal_total_difficulty_passed: true,
        blob_schedule: BlobSchedule::default(),
        deposit_contract_address: H160::from_str(
            "0x4242424242424242424242424242424242424242",
        )
        .unwrap(),
        ..Default::default()
    };

    if *fork >= Fork::Shanghai {
        cfg.shanghai_time = Some(0);
    }
    if *fork >= Fork::Cancun {
        cfg.cancun_time = Some(0);
    }
    if *fork >= Fork::Prague {
        cfg.prague_time = Some(0);
    }
    if *fork >= Fork::Osaka {
        cfg.osaka_time = Some(0);
    }
    if *fork >= Fork::Amsterdam {
        cfg.amsterdam_time = Some(0);
    }

    cfg
}

// ---- Custom deserializers ----

fn deserialize_ef_post_value_indexes<'de, D>(
    deserializer: D,
) -> Result<HashMap<String, U256>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let aux: HashMap<String, u64> = HashMap::deserialize(deserializer)?;
    Ok(aux.into_iter().map(|(k, v)| (k, U256::from(v))).collect())
}

fn deserialize_access_lists<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<Vec<AccessListItem>>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let lists: Option<Vec<Option<Vec<AccessListItem>>>> = Option::deserialize(deserializer)?;
    Ok(lists.map(|ls| ls.into_iter().map(|l| l.unwrap_or_default()).collect()))
}

fn deserialize_authorization_lists<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<AuthorizationListTupleRaw>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<Vec<AuthorizationListTupleRaw>>::deserialize(deserializer)
}

fn deserialize_post<'de, D>(
    deserializer: D,
) -> Result<HashMap<Fork, Vec<RawPostValue>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = HashMap::<String, Vec<RawPostValue>>::deserialize(deserializer)?;
    let mut parsed = HashMap::new();
    for (fork_str, values) in raw {
        let fork = match fork_str.as_str() {
            "Frontier" => Fork::Frontier,
            "Homestead" => Fork::Homestead,
            "Constantinople" => Fork::Constantinople,
            "ConstantinopleFix" | "Petersburg" => Fork::Petersburg,
            "Istanbul" => Fork::Istanbul,
            "Berlin" => Fork::Berlin,
            "London" => Fork::London,
            "Paris" | "Merge" => Fork::Paris,
            "Shanghai" => Fork::Shanghai,
            "Cancun" => Fork::Cancun,
            "Prague" => Fork::Prague,
            "Byzantium" => Fork::Byzantium,
            "EIP158" => Fork::SpuriousDragon,
            "EIP150" => Fork::Tangerine,
            "Osaka" => Fork::Osaka,
            "Amsterdam" => Fork::Amsterdam,
            other => {
                return Err(serde::de::Error::custom(format!(
                    "Unknown fork name: {other}"
                )));
            }
        };
        parsed.insert(fork, values);
    }
    Ok(parsed)
}
