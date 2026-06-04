// EEST BlockchainEngineFixture deserialization. Schema: packages/testing/src/execution_testing/fixtures/blockchain.py

use anyhow::Context;
use ethrex_common::{H256, U256, types::Genesis};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

/// One JSON file holds many fixtures keyed by test name.
pub type EngineFixtureFile = BTreeMap<String, EngineFixture>;

#[derive(Debug, Deserialize)]
pub struct EngineFixture {
    pub network: String,
    #[serde(rename = "genesisBlockHeader")]
    pub genesis_block_header: FixtureHeader,
    pub pre: Value,
    pub config: Value,
    #[serde(rename = "engineNewPayloads")]
    pub engine_new_payloads: Vec<FixturePayload>,
    pub lastblockhash: H256,
}

/// Genesis block header as represented in EEST engine fixtures.
/// Captures all fields required for genesis construction and block identification.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FixtureHeader {
    /// The computed block hash (JSON key: "hash").
    #[serde(rename = "hash")]
    pub block_hash: H256,
    pub state_root: H256,
    pub number: U256,
    // Fields required for Genesis construction (keys match camelCase of the field name):
    pub coinbase: String,
    pub difficulty: String,
    pub gas_limit: String,
    pub nonce: String,
    pub mix_hash: String,
    pub timestamp: String,
    #[serde(default)]
    pub extra_data: Option<String>,
    #[serde(default)]
    pub base_fee_per_gas: Option<String>,
    #[serde(default)]
    pub withdrawals_root: Option<String>,
    #[serde(default)]
    pub blob_gas_used: Option<String>,
    #[serde(default)]
    pub excess_blob_gas: Option<String>,
    #[serde(default)]
    pub parent_beacon_block_root: Option<String>,
    #[serde(default)]
    pub requests_hash: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct FixturePayload {
    /// Pre-built positional args for engine_newPayloadVx.
    pub params: Vec<Value>,
    #[serde(rename = "newPayloadVersion", deserialize_with = "de_str_u8")]
    pub new_payload_version: u8,
    #[serde(rename = "forkchoiceUpdatedVersion", deserialize_with = "de_str_u8")]
    pub forkchoice_updated_version: u8,
    #[serde(default, rename = "validationError")]
    pub validation_error: Option<ValidationError>,
    #[serde(default, rename = "errorCode", deserialize_with = "de_opt_str_i32")]
    pub error_code: Option<i32>,
    /// Expected execution witness for this payload (zkevm fixtures):
    /// `{state, codes, headers}` arrays of hex strings. When present (and the
    /// payload is V5), the runner exercises `engine_newPayloadWithWitnessV5`
    /// and compares the returned witness against it.
    #[serde(default, rename = "executionWitness")]
    pub execution_witness: Option<Value>,
    /// EIP-7805 (FOCIL): the inclusion-list transactions for this payload,
    /// carried by the fixture as a field SEPARATE from `params`. Present (and
    /// possibly empty) only on FOCIL engine fixtures. When present it is
    /// appended as the final `engine_newPayload` argument.
    #[serde(default, rename = "inclusionListTransactions")]
    pub inclusion_list_transactions: Option<Vec<Value>>,
    /// Explicit expected payload status (e.g. `"INCLUSION_LIST_UNSATISFIED"`).
    /// FOCIL fixtures use this third status value instead of the
    /// `validationError`/`errorCode` VALID/INVALID model. `null` for ordinary
    /// fixtures.
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ValidationError {
    Single(String),
    List(Vec<String>),
}

impl FixturePayload {
    /// The expected engine payload `status` string. An explicit fixture
    /// `status` (FOCIL's `INCLUSION_LIST_UNSATISFIED`, or a literal `VALID`)
    /// wins; otherwise it derives from the VALID/INVALID model: `INVALID` when
    /// a `validationError` is set, else `VALID`. `errorCode` (a JSON-RPC error)
    /// is handled separately by the response checker.
    pub fn expected_status(&self) -> &str {
        if let Some(s) = &self.status {
            return s;
        }
        if self.validation_error.is_some() {
            "INVALID"
        } else {
            "VALID"
        }
    }

    /// Returns `true` when the payload is expected to be accepted as the new
    /// canonical head (status `VALID`, no JSON-RPC error). A FOCIL
    /// `INCLUSION_LIST_UNSATISFIED` payload is NOT valid for head advancement:
    /// the block executes but must not be attested, so the follow-up FCU is
    /// skipped.
    pub fn valid(&self) -> bool {
        self.error_code.is_none() && self.expected_status() == "VALID"
    }

    /// The `engine_newPayload` version to call and the positional args for it.
    ///
    /// For FOCIL fixtures (`inclusionListTransactions` present) the IL is
    /// appended as the final argument and the call is routed to ethrex's
    /// IL-aware handler. NOTE: EEST emits `newPayloadVersion: 5` for FOCIL, but
    /// ethrex exposes the IL-bearing method as `engine_newPayloadV6` (V5 is the
    /// non-IL Amsterdam method); we remap here. This bridges a version-number
    /// divergence that should be reconciled against the final execution-apis
    /// FOCIL spec.
    pub fn engine_call(&self) -> (u8, Vec<Value>) {
        match &self.inclusion_list_transactions {
            Some(il) => {
                let mut params = self.params.clone();
                params.push(Value::Array(il.clone()));
                (6, params)
            }
            None => (self.new_payload_version, self.params.clone()),
        }
    }

    /// Extract `blockHash` from `params[0]` (the ExecutionPayload object).
    pub fn head_block_hash(&self) -> anyhow::Result<H256> {
        let payload = self
            .params
            .first()
            .context("engineNewPayload params is empty")?;
        let hash_str = payload["blockHash"]
            .as_str()
            .context("params[0].blockHash is missing or not a string")?;
        let hash = hash_str
            .parse::<H256>()
            .map_err(|e| anyhow::anyhow!("invalid blockHash hex: {e}"))?;
        Ok(hash)
    }
}

impl EngineFixture {
    /// Parse the fixture `network` field. Returns `(genesis_fork, transition)` where
    /// `transition = Some((target_fork, activation_time_secs))` when the fixture is a
    /// fork-transition test (e.g. `CancunToPragueAtTime15k`).
    pub(crate) fn schedule(
        &self,
    ) -> anyhow::Result<(
        ethrex_common::types::Fork,
        Option<(ethrex_common::types::Fork, u64)>,
    )> {
        parse_network(&self.network)
    }

    /// The "active" fork for skip checks: the transition target if present, else the genesis fork.
    pub fn fork(&self) -> anyhow::Result<ethrex_common::types::Fork> {
        let (genesis_fork, transition) = self.schedule()?;
        Ok(transition.map(|(to, _)| to).unwrap_or(genesis_fork))
    }

    /// Build a `Genesis` value from the fixture's `pre` + `config` + `genesisBlockHeader`.
    ///
    /// Mirrors what `hive/clients/ethrex/mapper.jq` does: assembles a Geth-style genesis
    /// JSON (alloc, config with fork activations, header fields) and deserializes it.
    pub fn build_genesis(&self) -> anyhow::Result<Genesis> {
        let chain_id = parse_chain_id(&self.config)?;
        let (genesis_fork, transition) = self.schedule()?;
        let mut config_json = build_chain_config_json(genesis_fork, transition, chain_id);
        // EEST fixtures carry their own per-fork blobSchedule (Cancun/Prague/Osaka/BPO*/Amsterdam)
        // with fork-name keys and hex-string values. Convert and inject; otherwise post-Cancun
        // payloads that rely on non-default blob params get rejected.
        if let Some(eest_schedule) = self.config.get("blobSchedule") {
            let converted = convert_blob_schedule(eest_schedule);
            config_json
                .as_object_mut()
                .expect("config_json is an object")
                .insert("blobSchedule".into(), converted);
        }
        let genesis_json = build_genesis_json(&self.genesis_block_header, &self.pre, config_json);
        serde_json::from_value::<Genesis>(genesis_json).context("Failed to deserialize Genesis")
    }
}

// ─── Private helpers ──────────────────────────────────────────────────────────

/// Deserialize a `u8` from either a JSON number or a string (e.g. `"1"`).
/// EEST fixtures encode version fields as strings.
fn de_str_u8<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u8, D::Error> {
    use serde::de::Error;
    let v: Value = Value::deserialize(d)?;
    match &v {
        Value::Number(n) => n
            .as_u64()
            .and_then(|n| u8::try_from(n).ok())
            .ok_or_else(|| D::Error::custom(format!("invalid u8: {n}"))),
        Value::String(s) => s
            .parse::<u8>()
            .map_err(|_| D::Error::custom(format!("invalid u8 string: {s}"))),
        other => Err(D::Error::custom(format!("expected u8, got: {other}"))),
    }
}

/// Deserialize `Option<i32>` from a JSON number OR a string (EEST encodes `errorCode`
/// as a quoted decimal e.g. `"-32602"` in some fixtures, plain int in others).
fn de_opt_str_i32<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<i32>, D::Error> {
    use serde::de::Error;
    let opt = Option::<Value>::deserialize(d)?;
    let Some(v) = opt else { return Ok(None) };
    let n: i64 = match &v {
        Value::Null => return Ok(None),
        Value::Number(n) => n
            .as_i64()
            .ok_or_else(|| D::Error::custom(format!("not an i64: {n}")))?,
        Value::String(s) => s
            .parse::<i64>()
            .map_err(|e| D::Error::custom(format!("invalid i32 string '{s}': {e}")))?,
        other => return Err(D::Error::custom(format!("expected i32, got: {other}"))),
    };
    i32::try_from(n)
        .map(Some)
        .map_err(|_| D::Error::custom(format!("i32 overflow: {n}")))
}

/// Parse `config.chainid` which may be a hex string ("0x01") or decimal string ("1").
fn parse_chain_id(config: &Value) -> anyhow::Result<u64> {
    let raw = config
        .get("chainid")
        .or_else(|| config.get("chainId"))
        .context("config missing 'chainid' field")?;

    if let Some(s) = raw.as_str() {
        let id = if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
            u64::from_str_radix(hex, 16)
        } else {
            s.parse::<u64>()
        }
        .map_err(|e| anyhow::anyhow!("invalid chainid '{s}': {e}"))?;
        Ok(id)
    } else if let Some(n) = raw.as_u64() {
        Ok(n)
    } else {
        anyhow::bail!("config.chainid is not a string or number: {raw}")
    }
}

/// Parse an EEST `network` string into (genesis_fork, optional transition target+time).
///
/// Single forks: `"Cancun"`, `"Prague"`, `"Amsterdam"`, `"BPO1"`, ...
/// Transitions:  `"CancunToPragueAtTime15k"`, `"OsakaToBPO1AtTime15k"`, ...
///   → returns `(Cancun, Some((Prague, 15_000)))`.
fn parse_network(
    s: &str,
) -> anyhow::Result<(
    ethrex_common::types::Fork,
    Option<(ethrex_common::types::Fork, u64)>,
)> {
    if let Some(at_idx) = s.find("AtTime") {
        let head = &s[..at_idx];
        let tail = &s[at_idx + "AtTime".len()..];
        let secs: u64 = if let Some(k) = tail.strip_suffix('k') {
            k.parse::<u64>()
                .map_err(|e| anyhow::anyhow!("bad time '{tail}': {e}"))?
                * 1000
        } else {
            tail.parse::<u64>()
                .map_err(|e| anyhow::anyhow!("bad time '{tail}': {e}"))?
        };
        let to_idx = head
            .find("To")
            .ok_or_else(|| anyhow::anyhow!("missing 'To' in transition network: {s}"))?;
        let from = single_fork(&head[..to_idx])?;
        let to = single_fork(&head[to_idx + 2..])?;
        return Ok((from, Some((to, secs))));
    }
    Ok((single_fork(s)?, None))
}

fn single_fork(s: &str) -> anyhow::Result<ethrex_common::types::Fork> {
    use ethrex_common::types::Fork;
    let f = match s {
        "Frontier" => Fork::Frontier,
        "Homestead" => Fork::Homestead,
        "EIP150" | "Tangerine" => Fork::Tangerine,
        "EIP158" | "SpuriousDragon" => Fork::SpuriousDragon,
        "Byzantium" => Fork::Byzantium,
        "Constantinople" => Fork::Constantinople,
        "ConstantinopleFix" | "Petersburg" => Fork::Petersburg,
        "Istanbul" => Fork::Istanbul,
        "MuirGlacier" => Fork::MuirGlacier,
        "Berlin" => Fork::Berlin,
        "London" => Fork::London,
        "ArrowGlacier" => Fork::ArrowGlacier,
        "GrayGlacier" => Fork::GrayGlacier,
        "Paris" | "Merge" => Fork::Paris,
        "Shanghai" => Fork::Shanghai,
        "Cancun" => Fork::Cancun,
        "Prague" => Fork::Prague,
        "Osaka" => Fork::Osaka,
        "BPO1" => Fork::BPO1,
        "BPO2" => Fork::BPO2,
        "BPO3" => Fork::BPO3,
        "BPO4" => Fork::BPO4,
        "BPO5" => Fork::BPO5,
        "Amsterdam" => Fork::Amsterdam,
        other => anyhow::bail!("Unknown network: {other}"),
    };
    Ok(f)
}

/// Build a Geth-style chain config. All pre-Paris block-numbered forks activate at block 0.
/// Time-based forks activate at 0 for everything `<=` the target fork, except the transition
/// target (when set) which activates at `transition.1`. Later forks are not set.
fn build_chain_config_json(
    genesis_fork: ethrex_common::types::Fork,
    transition: Option<(ethrex_common::types::Fork, u64)>,
    chain_id: u64,
) -> Value {
    use ethrex_common::types::Fork;

    let mut cfg = serde_json::json!({
        "chainId": chain_id,
        "homesteadBlock": 0,
        "daoForkBlock": 0,
        "daoForkSupport": true,
        "eip150Block": 0,
        "eip155Block": 0,
        "eip158Block": 0,
        "byzantiumBlock": 0,
        "constantinopleBlock": 0,
        "petersburgBlock": 0,
        "istanbulBlock": 0,
        "muirGlacierBlock": 0,
        "berlinBlock": 0,
        "londonBlock": 0,
        "arrowGlacierBlock": 0,
        "grayGlacierBlock": 0,
        "mergeNetsplitBlock": 0,
        "terminalTotalDifficulty": 0,
        "terminalTotalDifficultyPassed": true,
        // Required by ChainConfig even on pre-Prague chains; use the canonical EIP-6110 address.
        "depositContractAddress": "0x00000000219ab540356cBB839Cbe05303d7705Fa",
    });

    let target_fork = transition.map(|(to, _)| to).unwrap_or(genesis_fork);
    let obj = cfg.as_object_mut().expect("json object");

    const TIME_FORKS: &[(Fork, &str)] = &[
        (Fork::Shanghai, "shanghaiTime"),
        (Fork::Cancun, "cancunTime"),
        (Fork::Prague, "pragueTime"),
        (Fork::Osaka, "osakaTime"),
        (Fork::BPO1, "bpo1Time"),
        (Fork::BPO2, "bpo2Time"),
        (Fork::BPO3, "bpo3Time"),
        (Fork::BPO4, "bpo4Time"),
        (Fork::BPO5, "bpo5Time"),
        (Fork::Amsterdam, "amsterdamTime"),
    ];

    for &(fork, field) in TIME_FORKS {
        if fork > target_fork {
            continue;
        }
        let time = match transition {
            Some((to, t)) if to == fork => t,
            _ => 0,
        };
        obj.insert(field.into(), time.into());
    }

    cfg
}

/// Convert EEST's blob-schedule shape (fork-name keys, hex-string values) into ethrex's
/// `ChainConfig::blob_schedule` shape (camelCase keys, numeric values).
fn convert_blob_schedule(eest: &Value) -> Value {
    let mut out = serde_json::Map::new();
    let Some(obj) = eest.as_object() else {
        return Value::Object(out);
    };
    for (k, v) in obj {
        out.insert(k.to_lowercase(), convert_blob_entry(v));
    }
    Value::Object(out)
}

fn convert_blob_entry(entry: &Value) -> Value {
    let Some(obj) = entry.as_object() else {
        return entry.clone();
    };
    serde_json::json!({
        "target": hex_or_num(obj.get("target")),
        "max": hex_or_num(obj.get("max")),
        "baseFeeUpdateFraction": hex_or_num(obj.get("baseFeeUpdateFraction")),
    })
}

fn hex_or_num(v: Option<&Value>) -> u64 {
    match v {
        Some(Value::String(s)) => s
            .strip_prefix("0x")
            .or_else(|| s.strip_prefix("0X"))
            .map(|h| u64::from_str_radix(h, 16).unwrap_or(0))
            .unwrap_or_else(|| s.parse().unwrap_or(0)),
        Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
        _ => 0,
    }
}

/// Assemble the Geth-style genesis JSON from the fixture header, pre alloc, and chain config.
fn build_genesis_json(header: &FixtureHeader, alloc: &Value, config: Value) -> Value {
    let mut genesis = serde_json::json!({
        "config": config,
        "alloc": alloc,
        "coinbase": header.coinbase,
        "difficulty": header.difficulty,
        "gasLimit": header.gas_limit,
        "nonce": header.nonce,
        "mixHash": header.mix_hash,
        "timestamp": header.timestamp,
    });

    let obj = genesis.as_object_mut().expect("json object");

    // Optional header fields — include only when present.
    // JSON keys must match Genesis serde camelCase field names.
    macro_rules! insert_opt {
        ($key:expr, $field:expr) => {
            if let Some(ref v) = $field {
                obj.insert($key.into(), v.clone().into());
            }
        };
    }
    insert_opt!("extraData", header.extra_data);
    insert_opt!("baseFeePerGas", header.base_fee_per_gas);
    insert_opt!("withdrawalsRoot", header.withdrawals_root);
    insert_opt!("blobGasUsed", header.blob_gas_used);
    insert_opt!("excessBlobGas", header.excess_blob_gas);
    insert_opt!("parentBeaconBlockRoot", header.parent_beacon_block_root);
    insert_opt!("requestsHash", header.requests_hash);

    genesis
}

/// Returns `true` when `fork` predates the Engine API (before Paris).
pub fn is_pre_paris(fork: ethrex_common::types::Fork) -> bool {
    fork < ethrex_common::types::Fork::Paris
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_paris_fork_skipped() {
        let raw = serde_json::json!({
            "test_london": {
                "network": "London",
                "lastblockhash": "0x0000000000000000000000000000000000000000000000000000000000000000",
                "genesisBlockHeader": {
                    "hash": "0x0000000000000000000000000000000000000000000000000000000000000000",
                    "stateRoot": "0x0000000000000000000000000000000000000000000000000000000000000000",
                    "number": "0x00",
                    "coinbase": "0x0000000000000000000000000000000000000000",
                    "difficulty": "0x00",
                    "gasLimit": "0x1000",
                    "nonce": "0x0000000000000000",
                    "mixHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
                    "timestamp": "0x00"
                },
                "pre": {},
                "config": { "chainid": "0x01" },
                "engineNewPayloads": []
            }
        })
        .to_string();
        let fixtures: EngineFixtureFile = serde_json::from_str(&raw).unwrap();
        let (_, fixture) = fixtures.iter().next().unwrap();
        let fork = fixture.fork().expect("fork() must succeed for London");
        assert_eq!(fork, ethrex_common::types::Fork::London);
        assert!(is_pre_paris(fork), "London is pre-Paris");
    }
}
