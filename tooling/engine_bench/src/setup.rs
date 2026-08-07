//! One-time run setup: fork detection, real payload-id acquisition, and blob
//! hash loading.

use crate::cli::ForkArg;
use crate::transports::json_rpc;
use crate::workloads::ZERO_HASH;
use ethrex_common::H256;
use ethrex_rpc::engine_rest::types::blobs::MAX_BLOBS_REQUEST;
use eyre::{Context, Result, eyre};
use reqwest::Client;
use serde_json::{Value, json};
use std::path::Path;

/// Head-block facts needed for forkchoice calls.
pub struct HeadInfo {
    pub hash: String,
    pub timestamp: u64,
    pub number: u64,
}

/// Fetch the latest block header (hash, timestamp, number, raw fields).
pub async fn latest_block(client: &Client, url: &str, token: &str) -> Result<(HeadInfo, Value)> {
    let resp = json_rpc::call(
        client,
        url,
        token,
        "eth_getBlockByNumber",
        ("latest", false),
    )
    .await?;
    let v = resp
        .json()
        .ok_or_else(|| eyre!("eth_getBlockByNumber: non-JSON response"))?;
    let head = v["result"].clone();
    let hash = head["hash"]
        .as_str()
        .ok_or_else(|| eyre!("eth_getBlockByNumber returned no head hash: {v}"))?
        .to_owned();
    let timestamp = head["timestamp"]
        .as_str()
        .and_then(|t| u64::from_str_radix(t.trim_start_matches("0x"), 16).ok())
        .ok_or_else(|| eyre!("eth_getBlockByNumber returned no parsable timestamp"))?;
    let number = head["number"]
        .as_str()
        .and_then(|t| u64::from_str_radix(t.trim_start_matches("0x"), 16).ok())
        .unwrap_or(0);
    Ok((
        HeadInfo {
            hash,
            timestamp,
            number,
        },
        head,
    ))
}

/// The fork's forkchoiceUpdated version and payload-attributes shape:
/// V1 Paris, V2 +withdrawals, V3 +parentBeaconBlockRoot, V4 +slotNumber.
pub fn fcu_method_and_attrs(
    fork: ForkArg,
    timestamp: u64,
    head_number: u64,
) -> (&'static str, Value) {
    let mut attrs = json!({
        "timestamp": format!("0x{timestamp:x}"),
        "prevRandao": ZERO_HASH,
        "suggestedFeeRecipient": "0x0000000000000000000000000000000000000000",
    });
    let method = match fork {
        ForkArg::Paris => "engine_forkchoiceUpdatedV1",
        ForkArg::Shanghai => {
            attrs["withdrawals"] = json!([]);
            "engine_forkchoiceUpdatedV2"
        }
        ForkArg::Cancun | ForkArg::Prague | ForkArg::Osaka => {
            attrs["withdrawals"] = json!([]);
            attrs["parentBeaconBlockRoot"] = json!(ZERO_HASH);
            "engine_forkchoiceUpdatedV3"
        }
        ForkArg::Amsterdam => {
            attrs["withdrawals"] = json!([]);
            attrs["parentBeaconBlockRoot"] = json!(ZERO_HASH);
            // Slot is synthetic; one slot per block keeps it monotonic.
            attrs["slotNumber"] = json!(format!("0x{:x}", head_number + 1));
            "engine_forkchoiceUpdatedV4"
        }
    };
    (method, attrs)
}

/// Send forkchoiceUpdated with payload attributes on top of `head` and return
/// the payload id. Safe/finalized are sent as zero, so the node's chain and
/// finality state are not modified beyond starting one payload build.
pub async fn start_payload_build(
    client: &Client,
    url: &str,
    token: &str,
    fork: ForkArg,
    head: &HeadInfo,
    timestamp: u64,
) -> Result<String> {
    let fcu_state = json!({
        "headBlockHash": head.hash,
        "safeBlockHash": ZERO_HASH,
        "finalizedBlockHash": ZERO_HASH,
    });
    let (fcu_method, attrs) = fcu_method_and_attrs(fork, timestamp, head.number);
    let resp = json_rpc::call(client, url, token, fcu_method, (fcu_state, attrs)).await?;
    let v = resp
        .json()
        .ok_or_else(|| eyre!("{fcu_method}: non-JSON response"))?;
    if v.get("error").is_some_and(|err| !err.is_null()) {
        return Err(eyre!("{fcu_method} error: {}", v["error"]));
    }
    v["result"]["payloadId"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| {
            eyre!(
                "{fcu_method} returned no payloadId (payload status: {})",
                v["result"]["payloadStatus"]["status"]
            )
        })
}

/// Obtain a real payload id by asking the node to start a payload build on
/// top of its own head.
pub async fn acquire_payload_id(
    client: &Client,
    url_base: &str,
    secret: &[u8],
    fork: ForkArg,
) -> Result<String> {
    let token = crate::jwt::mint(secret)?;
    let (head, _) = latest_block(client, url_base, &token).await?;
    let ts = head.timestamp + 12;
    start_payload_build(client, url_base, &token, fork, &head, ts).await
}

/// Detect the fork era of an external node from its latest header fields
/// (every post-merge header addition is optional-and-skipped in JSON), then
/// disambiguate Prague vs Osaka — identical headers — by probing
/// `engine_getPayloadV5` with a freshly built payload (V5 rejects Prague-era
/// payloads with UnsupportedFork).
pub async fn detect_fork(client: &Client, url: &str, secret: &[u8]) -> Result<ForkArg> {
    let token = crate::jwt::mint(secret)?;
    let (_, header) = latest_block(client, url, &token).await?;

    if header.get("slotNumber").is_some() || header.get("blockAccessListHash").is_some() {
        return Ok(ForkArg::Amsterdam);
    }
    if header.get("requestsHash").is_some() {
        let id = acquire_payload_id(client, url, secret, ForkArg::Prague)
            .await
            .context("Prague/Osaka disambiguation needs a payload build")?;
        let resp =
            json_rpc::call(client, url, &token, "engine_getPayloadV5", (id.as_str(),)).await?;
        let v = resp
            .json()
            .ok_or_else(|| eyre!("engine_getPayloadV5: non-JSON response"))?;
        let unsupported = v["error"]["message"]
            .as_str()
            .is_some_and(|m| m.to_lowercase().contains("unsupported"));
        return Ok(if unsupported {
            ForkArg::Prague
        } else {
            ForkArg::Osaka
        });
    }
    if header.get("excessBlobGas").is_some() {
        return Ok(ForkArg::Cancun);
    }
    if header.get("withdrawalsRoot").is_some() {
        return Ok(ForkArg::Shanghai);
    }
    Ok(ForkArg::Paris)
}

/// Load newline-separated 0x-prefixed versioned hashes. Lines that are empty
/// or start with `#` are skipped. Capped at `MAX_BLOBS_REQUEST`.
pub fn load_blob_hashes(path: &Path) -> Result<Vec<H256>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading blob hashes from {}", path.display()))?;
    let mut hashes = Vec::new();
    for (lineno, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let bytes = hex::decode(line.trim_start_matches("0x"))
            .with_context(|| format!("{}:{}: invalid hex", path.display(), lineno + 1))?;
        if bytes.len() != 32 {
            return Err(eyre!(
                "{}:{}: expected 32 bytes, got {}",
                path.display(),
                lineno + 1,
                bytes.len()
            ));
        }
        hashes.push(H256::from_slice(&bytes));
    }
    if hashes.is_empty() {
        return Err(eyre!("no versioned hashes in {}", path.display()));
    }
    if hashes.len() > MAX_BLOBS_REQUEST {
        eprintln!(
            "WARNING: {} hashes supplied, truncating to MAX_BLOBS_REQUEST ({MAX_BLOBS_REQUEST})",
            hashes.len()
        );
        hashes.truncate(MAX_BLOBS_REQUEST);
    }
    Ok(hashes)
}
