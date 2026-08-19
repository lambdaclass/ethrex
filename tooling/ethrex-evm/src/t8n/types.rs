//! Input parsing for the t8n tool: the pre-state alloc, the block
//! environment, and the transaction list, in geth `evm t8n` JSON formats.
//!
//! Inputs arrive either as individual files or bundled in a single JSON
//! object on stdin (`{"alloc": …, "env": …, "txs": …}`), selected per input
//! by passing `stdin` as its path.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use ethrex_common::types::{GenesisAccount, Withdrawal};
use ethrex_common::{Address, H256, U256};
use serde::Deserialize;

use super::T8nArgs;
use super::error::T8nError;

/// Block environment, using geth's t8n field names. Numeric values are
/// `0x`-prefixed hex strings. Unknown fields are ignored.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EnvJson {
    pub current_coinbase: Address,
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str")]
    pub current_gas_limit: u64,
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str")]
    pub current_number: u64,
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str")]
    pub current_timestamp: u64,
    pub current_random: Option<U256>,
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str_opt")]
    pub current_base_fee: Option<u64>,
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str_opt")]
    pub current_excess_blob_gas: Option<u64>,
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str_opt")]
    pub slot_number: Option<u64>,
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str_opt")]
    pub parent_timestamp: Option<u64>,
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str_opt")]
    pub parent_base_fee: Option<u64>,
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str_opt")]
    pub parent_gas_used: Option<u64>,
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str_opt")]
    pub parent_gas_limit: Option<u64>,
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str_opt")]
    pub parent_excess_blob_gas: Option<u64>,
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str_opt")]
    pub parent_blob_gas_used: Option<u64>,
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str_opt")]
    pub parent_slot_number: Option<u64>,
    pub parent_beacon_block_root: Option<H256>,
    /// Ancestor hashes for BLOCKHASH, keyed by block number (hex string).
    pub block_hashes: BTreeMap<String, H256>,
    pub withdrawals: Option<Vec<Withdrawal>>,
}

impl EnvJson {
    /// Parse the `blockHashes` map keys (hex or decimal strings) into
    /// numbers.
    pub fn parsed_block_hashes(&self) -> Result<BTreeMap<u64, H256>, T8nError> {
        let mut hashes = BTreeMap::new();
        for (key, hash) in &self.block_hashes {
            let number = parse_u64(key).ok_or_else(|| {
                T8nError::Parse("env".to_string(), format!("invalid blockHashes key: {key}"))
            })?;
            hashes.insert(number, *hash);
        }
        Ok(hashes)
    }
}

fn parse_u64(value: &str) -> Option<u64> {
    if let Some(hex_str) = value.strip_prefix("0x") {
        u64::from_str_radix(hex_str, 16).ok()
    } else {
        value.parse().ok()
    }
}

/// Blob parameters for the block's fork, as sent by test fillers via
/// `blobParams` (hex-string numbers).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlobParamsJson {
    pub target: U256,
    pub max: U256,
    pub base_fee_update_fraction: U256,
}

/// The t8n inputs. Transactions stay as raw JSON values so a malformed
/// transaction is rejected individually instead of failing the whole run.
pub struct Inputs {
    pub alloc: BTreeMap<Address, GenesisAccount>,
    pub env: EnvJson,
    pub txs: Vec<serde_json::Value>,
    pub blob_params: Option<BlobParamsJson>,
}

/// Read the alloc/env/txs inputs from files or the stdin bundle,
/// according to the `--input.*` arguments.
pub fn read_inputs(args: &T8nArgs) -> Result<Inputs, T8nError> {
    let uses_stdin = [&args.input_alloc, &args.input_env, &args.input_txs]
        .iter()
        .any(|input| input.as_str() == "stdin");
    let stdin_bundle: serde_json::Value = if uses_stdin {
        let mut raw = String::new();
        std::io::stdin()
            .read_to_string(&mut raw)
            .map_err(|e| T8nError::Io("stdin".to_string(), e))?;
        serde_json::from_str(&raw)
            .map_err(|e| T8nError::Parse("stdin".to_string(), e.to_string()))?
    } else {
        serde_json::Value::Null
    };

    let read = |input: &str, key: &str| -> Result<serde_json::Value, T8nError> {
        if input == "stdin" {
            Ok(stdin_bundle.get(key).cloned().unwrap_or_default())
        } else {
            let raw = std::fs::read_to_string(Path::new(input))
                .map_err(|e| T8nError::Io(input.to_string(), e))?;
            serde_json::from_str(&raw)
                .map_err(|e| T8nError::Parse(input.to_string(), e.to_string()))
        }
    };

    let alloc = serde_json::from_value(read(&args.input_alloc, "alloc")?)
        .map_err(|e| T8nError::Parse("alloc".to_string(), e.to_string()))?;
    let env = serde_json::from_value(read(&args.input_env, "env")?)
        .map_err(|e| T8nError::Parse("env".to_string(), e.to_string()))?;
    let txs = match read(&args.input_txs, "txs")? {
        serde_json::Value::Null => Vec::new(),
        serde_json::Value::Array(values) => values,
        _ => {
            return Err(T8nError::Parse(
                "txs".to_string(),
                "expected a JSON array of transactions".to_string(),
            ));
        }
    };
    // Blob params ride in the stdin bundle (stream mode) or a file
    // (`--input.blobParams`, filesystem mode).
    let blob_params_value = if let Some(path) = &args.input_blob_params {
        read(path, "blobParams")?
    } else {
        stdin_bundle.get("blobParams").cloned().unwrap_or_default()
    };
    let blob_params = match blob_params_value {
        serde_json::Value::Null => None,
        value => Some(
            serde_json::from_value(value)
                .map_err(|e| T8nError::Parse("blobParams".to_string(), e.to_string()))?,
        ),
    };

    Ok(Inputs {
        alloc,
        env,
        txs,
        blob_params,
    })
}
