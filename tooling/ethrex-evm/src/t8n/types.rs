//! Input parsing for the t8n tool: the pre-state alloc, the block
//! environment, and the transaction list, in geth `evm t8n` JSON formats.
//!
//! Inputs arrive either as individual files or bundled in a single JSON
//! object on stdin (`{"alloc": …, "env": …, "txs": …}`), selected per input
//! by passing `stdin` as its path.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use ethrex_common::types::{BlockHeader, GenesisAccount, Withdrawal};
use ethrex_common::{Address, H256, U256};
use ethrex_rlp::decode::RLPDecode;
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
    /// RLP of the ancestor headers as hex strings, with the keys of
    /// `block_hashes`. The execution witness needs the real headers, and the
    /// t8n has no other source for them.
    pub block_headers: BTreeMap<String, String>,
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

    /// Decode the `blockHeaders` map into headers keyed by block number,
    /// checking each one against `block_hashes` (the parsed `blockHashes`).
    pub fn parsed_block_headers(
        &self,
        block_hashes: &BTreeMap<u64, H256>,
    ) -> Result<BTreeMap<u64, BlockHeader>, T8nError> {
        let invalid = |message: String| T8nError::Parse("env".to_string(), message);
        let mut headers = BTreeMap::new();
        for (key, encoded) in &self.block_headers {
            let number = parse_u64(key)
                .ok_or_else(|| invalid(format!("invalid blockHeaders key: {key}")))?;
            let encoded = hex::decode(encoded.strip_prefix("0x").unwrap_or(encoded))
                .map_err(|e| invalid(format!("blockHeaders[{number}] is not hex: {e}")))?;
            let header = BlockHeader::decode(&encoded)
                .map_err(|e| invalid(format!("failed to decode blockHeaders[{number}]: {e}")))?;
            if header.number != number {
                return Err(invalid(format!(
                    "blockHeaders[{number}] decodes to header number {}",
                    header.number
                )));
            }
            let want = block_hashes.get(&number).ok_or_else(|| {
                invalid(format!(
                    "blockHeaders[{number}] has no entry in blockHashes"
                ))
            })?;
            // The hash is taken over the header as ethrex encodes it, which
            // is what goes into the witness, so this also rejects an entry
            // that isn't canonical RLP.
            let have = header.hash();
            if have != *want {
                return Err(invalid(format!(
                    "blockHeaders[{number}] hash mismatch: have {have:#x}, want {want:#x}"
                )));
            }
            headers.insert(number, header);
        }
        Ok(headers)
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

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_rlp::encode::RLPEncode;

    fn header(number: u64) -> BlockHeader {
        BlockHeader {
            number,
            ..Default::default()
        }
    }

    fn hex_rlp(header: &BlockHeader) -> String {
        format!("0x{}", hex::encode(header.encode_to_vec()))
    }

    /// An environment with `headers` under the given `blockHeaders` keys.
    fn env_with_headers(headers: &[(&str, &BlockHeader)]) -> EnvJson {
        EnvJson {
            block_headers: headers
                .iter()
                .map(|(key, header)| (key.to_string(), hex_rlp(header)))
                .collect(),
            ..Default::default()
        }
    }

    fn parse_error(env: &EnvJson, block_hashes: &BTreeMap<u64, H256>) -> String {
        env.parsed_block_headers(block_hashes)
            .expect_err("the headers were accepted")
            .to_string()
    }

    #[test]
    fn accepts_headers_that_have_their_block_hashes() {
        let (first, second) = (header(1), header(2));
        let env = env_with_headers(&[("0x01", &first), ("2", &second)]);
        let block_hashes = BTreeMap::from([(1, first.hash()), (2, second.hash())]);

        let headers = env.parsed_block_headers(&block_hashes).expect("headers");

        assert_eq!(headers.keys().copied().collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(headers[&1].hash(), first.hash());
        assert_eq!(headers[&2].hash(), second.hash());
    }

    #[test]
    fn rejects_a_header_with_another_hash() {
        let first = header(1);
        let env = env_with_headers(&[("0x01", &first)]);
        let block_hashes = BTreeMap::from([(1, H256::repeat_byte(0x11))]);

        let error = parse_error(&env, &block_hashes);
        assert!(error.contains("blockHeaders[1] hash mismatch"), "{error}");
    }

    #[test]
    fn rejects_a_header_under_the_number_of_another_block() {
        let second = header(2);
        let env = env_with_headers(&[("0x01", &second)]);
        let block_hashes = BTreeMap::from([(1, second.hash())]);

        let error = parse_error(&env, &block_hashes);
        assert!(
            error.contains("blockHeaders[1] decodes to header number 2"),
            "{error}"
        );
    }

    #[test]
    fn rejects_a_header_without_a_block_hash() {
        let first = header(1);
        let env = env_with_headers(&[("0x01", &first)]);

        let error = parse_error(&env, &BTreeMap::new());
        assert!(
            error.contains("blockHeaders[1] has no entry in blockHashes"),
            "{error}"
        );
    }

    #[test]
    fn rejects_a_header_that_is_not_the_rlp_of_one() {
        let mut env = env_with_headers(&[]);
        env.block_headers
            .insert("0x01".to_string(), "0xc0".to_string());
        let error = parse_error(&env, &BTreeMap::new());
        assert!(
            error.contains("failed to decode blockHeaders[1]"),
            "{error}"
        );

        env.block_headers
            .insert("0x01".to_string(), "0xzz".to_string());
        let error = parse_error(&env, &BTreeMap::new());
        assert!(error.contains("blockHeaders[1] is not hex"), "{error}");
    }
}
