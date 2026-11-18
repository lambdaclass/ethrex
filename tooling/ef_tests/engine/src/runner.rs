use std::fmt;

use ethrex_common::H256;
use serde_json::Value;

use crate::fixture::{EngineFixture, FixturePayload, ValidationError, is_pre_paris};
use crate::harness::{Backend, EngineApiHarness};

// ─── Public types ─────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct RunOptions {
    pub backend: Backend,
    /// Check an `INVALID` payload's `validationError` against the fixture's, as
    /// the cargo engine harness does: a mismatch is warned about, or fails the
    /// fixture with `strict_exceptions`. Without it a fixture's verdict depends
    /// only on which payloads it expects to be rejected (and the JSON-RPC error
    /// code it expects).
    pub check_exception_names: bool,
    pub strict_exceptions: bool,
    /// Run blocks that carry an access list on the BAL-driven parallel executor,
    /// as a node does unless started with `--no-bal-parallel-exec`.
    pub bal_parallel_exec: bool,
    /// Cache precompile results within a block, as a node does unless started with
    /// `--no-precompile-cache`.
    pub precompile_cache: bool,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            backend: Backend::InMemory,
            check_exception_names: true,
            strict_exceptions: false,
            bal_parallel_exec: true,
            precompile_cache: true,
        }
    }
}

impl RunOptions {
    pub fn from_env() -> Self {
        Self {
            backend: parse_backend_env(),
            strict_exceptions: std::env::var("ETHREX_ENGINE_STRICT_EXCEPTIONS")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            ..Self::default()
        }
    }
}

fn parse_backend_env() -> Backend {
    match std::env::var("ETHREX_ENGINE_BACKEND").as_deref() {
        Ok("inmemory") | Err(_) => Backend::InMemory,
        #[cfg(feature = "rocksdb")]
        Ok("rocksdb") => Backend::RocksDB,
        Ok(other) => panic!(
            "ETHREX_ENGINE_BACKEND='{other}' invalid; valid: inmemory{}",
            if cfg!(feature = "rocksdb") {
                ", rocksdb"
            } else {
                ""
            }
        ),
    }
}

/// A payload the client rejected, with an `INVALID` status or a JSON-RPC error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    /// The payload's position in the fixture's `engineNewPayloads`, from 0.
    pub index: usize,
    /// The payload's `blockHash`, for an `INVALID` status.
    pub hash: Option<H256>,
    /// The `validationError`, or for a JSON-RPC error `<code>: <message>` and,
    /// when the error carries data, `: <data>`.
    pub error: String,
}

#[derive(Debug)]
pub enum FixtureFailure {
    SkippedPreParis,
    FixtureParse(String),
    HarnessSetup(String),
    EmptyPayloads,
    InitialFcu(String),
    GenesisRpc(String),
    GenesisMismatch {
        expected: H256,
        got: H256,
    },
    PayloadRpc {
        index: usize,
        msg: String,
    },
    PayloadParse {
        index: usize,
        msg: String,
    },
    WrongStatus {
        index: usize,
        expected: String,
        got: String,
        validation_error: Option<String>,
    },
    WrongErrorCode {
        index: usize,
        want: i32,
        got: i32,
        msg: String,
    },
    UnexpectedJsonRpcError {
        index: usize,
        code: i32,
        msg: String,
    },
    MissingErrorCode {
        index: usize,
        want: i32,
    },
    MissingValidationError {
        index: usize,
    },
    ValidationErrorMismatch {
        index: usize,
        expected: Vec<String>,
        got: String,
        strict: bool,
    },
    MalformedResponse {
        index: usize,
        detail: String,
    },
    WitnessMismatch {
        index: usize,
        detail: String,
    },
    FollowupFcu {
        index: usize,
        msg: String,
    },
    HeadRpc(String),
    LastBlockHashMismatch {
        expected: H256,
        got: H256,
    },
}

impl FixtureFailure {
    pub fn is_skip(&self) -> bool {
        matches!(self, FixtureFailure::SkippedPreParis)
    }
}

impl fmt::Display for FixtureFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FixtureFailure::SkippedPreParis => write!(f, "skipped  reason=pre-Paris fork"),
            FixtureFailure::FixtureParse(e) => write!(f, "fixture_parse  error={e}"),
            FixtureFailure::HarnessSetup(e) => write!(f, "harness_setup  error={e}"),
            FixtureFailure::EmptyPayloads => write!(f, "empty_payloads  expected=at least 1"),
            FixtureFailure::InitialFcu(e) => write!(f, "initial_fcu  error={e}"),
            FixtureFailure::GenesisRpc(e) => write!(f, "genesis_rpc  error={e}"),
            FixtureFailure::GenesisMismatch { expected, got } => {
                write!(f, "genesis_mismatch  expected={expected:#x}  got={got:#x}")
            }
            FixtureFailure::PayloadRpc { index, msg } => {
                write!(f, "payload_rpc[{index}]  error={msg}")
            }
            FixtureFailure::PayloadParse { index, msg } => {
                write!(f, "payload_parse[{index}]  error={msg}")
            }
            FixtureFailure::WrongStatus {
                index,
                expected,
                got,
                validation_error,
            } => match validation_error {
                Some(ve) => write!(
                    f,
                    "wrong_status[{index}]  expected={expected}  got={got}  validationError={ve}"
                ),
                None => write!(f, "wrong_status[{index}]  expected={expected}  got={got}"),
            },
            FixtureFailure::WrongErrorCode {
                index,
                want,
                got,
                msg,
            } => write!(
                f,
                "wrong_error_code[{index}]  expected={want}  got={got}  msg={msg}"
            ),
            FixtureFailure::UnexpectedJsonRpcError { index, code, msg } => {
                write!(
                    f,
                    "unexpected_jsonrpc_error[{index}]  code={code}  msg={msg}"
                )
            }
            FixtureFailure::MissingErrorCode { index, want } => {
                write!(f, "missing_error_code[{index}]  expected_code={want}")
            }
            FixtureFailure::MissingValidationError { index } => {
                write!(
                    f,
                    "missing_validation_error[{index}]  expected=non-null validationError"
                )
            }
            FixtureFailure::ValidationErrorMismatch {
                index,
                expected,
                got,
                strict,
            } => write!(
                f,
                "validation_error_mismatch[{index}]  expected={expected:?}  got={got:?}  strict={strict}"
            ),
            FixtureFailure::MalformedResponse { index, detail } => {
                write!(f, "malformed_response[{index}]  detail={detail}")
            }
            FixtureFailure::WitnessMismatch { index, detail } => {
                write!(f, "witness_mismatch[{index}]  {detail}")
            }
            FixtureFailure::FollowupFcu { index, msg } => {
                write!(f, "followup_fcu[{index}]  error={msg}")
            }
            FixtureFailure::HeadRpc(e) => write!(f, "head_rpc  error={e}"),
            FixtureFailure::LastBlockHashMismatch { expected, got } => {
                write!(
                    f,
                    "lastblockhash_mismatch  expected={expected:#x}  got={got:#x}"
                )
            }
        }
    }
}

// ─── Main entry point ─────────────────────────────────────────────────────────

/// Run one fixture, adding each payload the client rejects to `rejections` as it
/// happens, so they are there even when the fixture then fails.
pub async fn run_fixture(
    name: &str,
    fix: &EngineFixture,
    opts: &RunOptions,
    rejections: &mut Vec<Rejection>,
) -> Result<(), FixtureFailure> {
    // 1. Pre-Paris skip
    let fork = fix
        .fork()
        .map_err(|e| FixtureFailure::FixtureParse(e.to_string()))?;
    if is_pre_paris(fork) {
        return Err(FixtureFailure::SkippedPreParis);
    }

    // 2. Build harness
    let genesis = fix
        .build_genesis()
        .map_err(|e| FixtureFailure::FixtureParse(e.to_string()))?;
    let harness = Box::pin(EngineApiHarness::from_genesis(genesis, opts))
        .await
        .map_err(|e| FixtureFailure::HarnessSetup(e.to_string()))?;

    // 3. Initial FCU to genesis (mirrors test_via_engine.py:80–105)
    let first = fix
        .engine_new_payloads
        .first()
        .ok_or(FixtureFailure::EmptyPayloads)?;
    let resp = Box::pin(harness.fcu(
        first.forkchoice_updated_version,
        fix.genesis_block_header.block_hash,
    ))
    .await
    .map_err(|e| FixtureFailure::InitialFcu(e.to_string()))?;
    assert_fcu_valid(&resp).map_err(FixtureFailure::InitialFcu)?;

    // 4. Genesis hash check (mirrors test_via_engine.py:107–122)
    let block = Box::pin(harness.get_block_by_number_zero())
        .await
        .map_err(|e| FixtureFailure::GenesisRpc(e.to_string()))?;
    let got_hash = parse_block_hash(&block).map_err(FixtureFailure::GenesisRpc)?;
    if got_hash != fix.genesis_block_header.block_hash {
        return Err(FixtureFailure::GenesisMismatch {
            expected: fix.genesis_block_header.block_hash,
            got: got_hash,
        });
    }

    // 5. Per-payload loop (mirrors test_via_engine.py:124–240)
    for (i, payload) in fix.engine_new_payloads.iter().enumerate() {
        // zkevm fixtures carry an expected witness per payload: route those
        // through `engine_newPayloadWithWitnessV{4,5}` (same params, same status
        // semantics as the plain call) so the engine-side witness generation is
        // exercised and its output verified against the fixture. Only V4 and V5
        // have a witness variant; a witnessed payload at any other version would
        // be a fixture-format change to look at, not something to route silently.
        let with_witness =
            matches!(payload.new_payload_version, 4 | 5) && payload.execution_witness.is_some();
        let resp = if with_witness {
            Box::pin(harness.new_payload_with_witness(payload.new_payload_version, &payload.params))
                .await
        } else {
            Box::pin(harness.new_payload(payload.new_payload_version, &payload.params)).await
        }
        .map_err(|e| FixtureFailure::PayloadRpc {
            index: i,
            msg: e.to_string(),
        })?;
        if let Some(error) = rejection_error(&resp) {
            let hash = if resp.get("error").is_some() {
                None
            } else {
                payload.head_block_hash().ok()
            };
            rejections.push(Rejection {
                index: i,
                hash,
                error,
            });
        }
        check_payload_response(&resp, payload, i, opts, name)?;
        if with_witness && payload.valid() {
            check_witness_response(&resp, payload, i, name)?;
        }
        if payload.valid() {
            let head = payload
                .head_block_hash()
                .map_err(|e| FixtureFailure::PayloadParse {
                    index: i,
                    msg: e.to_string(),
                })?;
            let fcu_resp = Box::pin(harness.fcu(payload.forkchoice_updated_version, head))
                .await
                .map_err(|e| FixtureFailure::FollowupFcu {
                    index: i,
                    msg: e.to_string(),
                })?;
            assert_fcu_valid(&fcu_resp)
                .map_err(|e| FixtureFailure::FollowupFcu { index: i, msg: e })?;
        }
    }

    // 6. The head the payloads and forkchoice updates left must be `lastblockhash`.
    let block = Box::pin(harness.get_latest_block())
        .await
        .map_err(|e| FixtureFailure::HeadRpc(e.to_string()))?;
    let head = parse_block_hash(&block).map_err(FixtureFailure::HeadRpc)?;
    if head != fix.lastblockhash {
        return Err(FixtureFailure::LastBlockHashMismatch {
            expected: fix.lastblockhash,
            got: head,
        });
    }

    Ok(())
}

// ─── Private helpers ──────────────────────────────────────────────────────────

/// Verify the witness returned by `engine_newPayloadWithWitnessV{4,5}` against the
/// fixture's expected `executionWitness`. The endpoint returns geth's
/// `ExtWitness` shape — an RLP list `(headers, codes, state, keys)` with
/// headers ascending by block number and codes/state sorted lexicographically —
/// which is the same canonical ordering the EEST fixture witness carries, so
/// the comparison is exact per section. `keys` is optional in the fixture
/// (geth emits it empty) and compared symmetrically: an absent section is
/// treated as empty.
fn check_witness_response(
    resp: &Value,
    payload: &FixturePayload,
    index: usize,
    name: &str,
) -> Result<(), FixtureFailure> {
    use ethrex_common::types::block_execution_witness::ExtWitness;
    use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};

    // EEST consumption-validation fixtures (`witness_validation_*`,
    // `*extra_unused*`) deliberately ship corrupted or padded witnesses to
    // test the consumer side; they are not generation oracles.
    if name.contains("witness_validation") || name.contains("extra_unused") {
        return Ok(());
    }

    let mismatch = |detail: String| FixtureFailure::WitnessMismatch { index, detail };

    let expected = payload
        .execution_witness
        .as_ref()
        .expect("caller gates on execution_witness presence");

    let witness_hex = resp
        .get("result")
        .and_then(|r| r.get("witness"))
        .and_then(|w| w.as_str())
        .ok_or_else(|| mismatch("missing 'witness' in response".into()))?;
    let witness_bytes = hex::decode(witness_hex.strip_prefix("0x").unwrap_or(witness_hex))
        .map_err(|e| mismatch(format!("witness hex decode failed: {e}")))?;

    let witness = ExtWitness::decode(&witness_bytes)
        .map_err(|e| mismatch(format!("witness RLP decode failed: {e}")))?;

    let got_headers: Vec<Vec<u8>> = witness.headers.iter().map(|h| h.encode_to_vec()).collect();
    let got_codes: Vec<Vec<u8>> = witness.codes.iter().map(|b| b.to_vec()).collect();
    let got_state: Vec<Vec<u8>> = witness.state.iter().map(|b| b.to_vec()).collect();

    for (section, got) in [
        ("headers", got_headers),
        ("codes", got_codes),
        ("state", got_state),
    ] {
        let exp = parse_witness_hex_array(expected, section).map_err(mismatch)?;
        if got != exp {
            return Err(mismatch(witness_diff_detail(section, &got, &exp)));
        }
    }

    // `keys` is optional in EEST fixtures (geth emits it empty). Compare it
    // symmetrically — an absent fixture section counts as empty — so a non-empty
    // `keys` is diffed like the other sections instead of being blindly
    // rejected, and an empty response against a populated fixture is caught.
    let got_keys: Vec<Vec<u8>> = witness.keys.iter().map(|b| b.to_vec()).collect();
    let exp_keys = parse_optional_witness_hex_array(expected, "keys").map_err(mismatch)?;
    if got_keys != exp_keys {
        return Err(mismatch(witness_diff_detail("keys", &got_keys, &exp_keys)));
    }
    Ok(())
}

/// Parse one `executionWitness` section (array of hex strings) into bytes.
fn parse_witness_hex_array(expected: &Value, key: &str) -> Result<Vec<Vec<u8>>, String> {
    expected
        .get(key)
        .and_then(|v| v.as_array())
        .ok_or_else(|| format!("fixture executionWitness missing '{key}' array"))?
        .iter()
        .map(|v| {
            let s = v
                .as_str()
                .ok_or_else(|| format!("non-string item in executionWitness '{key}'"))?;
            hex::decode(s.strip_prefix("0x").unwrap_or(s))
                .map_err(|e| format!("hex decode failed in executionWitness '{key}': {e}"))
        })
        .collect()
}

/// Like [`parse_witness_hex_array`] but treats an absent or null section as an
/// empty list. Used for the optional `keys` section, which EEST fixtures omit
/// and geth emits empty.
fn parse_optional_witness_hex_array(expected: &Value, key: &str) -> Result<Vec<Vec<u8>>, String> {
    match expected.get(key) {
        Some(v) if !v.is_null() => parse_witness_hex_array(expected, key),
        _ => Ok(Vec::new()),
    }
}

/// Compact diff summary for a mismatched witness section.
fn witness_diff_detail(section: &str, got: &[Vec<u8>], exp: &[Vec<u8>]) -> String {
    use std::collections::BTreeSet;
    let fmt_items = |items: &[&&[u8]]| {
        items
            .iter()
            .take(4)
            .map(|b| format!("0x{}…({}B)", hex::encode(&b[..b.len().min(8)]), b.len()))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let got_set: BTreeSet<&[u8]> = got.iter().map(|v| v.as_slice()).collect();
    let exp_set: BTreeSet<&[u8]> = exp.iter().map(|v| v.as_slice()).collect();
    let missing: Vec<&&[u8]> = exp_set.difference(&got_set).collect();
    let extra: Vec<&&[u8]> = got_set.difference(&exp_set).collect();
    if missing.is_empty() && extra.is_empty() {
        format!(
            "{section}: same items, different order/multiplicity (got {}, expected {})",
            got.len(),
            exp.len()
        )
    } else {
        format!(
            "{section}: got {} items, expected {}; missing=[{}]  extra=[{}]",
            got.len(),
            exp.len(),
            fmt_items(&missing),
            fmt_items(&extra)
        )
    }
}

/// The client's error for a `newPayload` response that rejects the payload: its
/// `validationError` for an `INVALID` status, or the JSON-RPC error as
/// `<code>: <message>`, plus `: <data>` when the error carries data (a string as
/// is, anything else as compact JSON).
fn rejection_error(resp: &Value) -> Option<String> {
    if let Some(err) = resp.get("error") {
        let code = err.get("code").map(Value::to_string).unwrap_or_default();
        let message = err.get("message").and_then(Value::as_str).unwrap_or("");
        let mut error = format!("{code}: {message}");
        match err.get("data") {
            None | Some(Value::Null) => {}
            Some(Value::String(data)) => error.push_str(&format!(": {data}")),
            Some(data) => error.push_str(&format!(": {data}")),
        }
        return Some(error);
    }
    let result = resp.get("result")?;
    if result.get("status").and_then(Value::as_str) != Some("INVALID") {
        return None;
    }
    let validation_error = result.get("validationError").and_then(Value::as_str);
    Some(validation_error.unwrap_or_default().to_string())
}

fn check_payload_response(
    resp: &Value,
    payload: &FixturePayload,
    index: usize,
    opts: &RunOptions,
    name: &str,
) -> Result<(), FixtureFailure> {
    // JSON-RPC error path
    if let Some(err) = resp.get("error") {
        let got_code = err.get("code").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let got_msg = err
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        match payload.error_code {
            Some(want) if want == got_code => return Ok(()),
            Some(want) => {
                return Err(FixtureFailure::WrongErrorCode {
                    index,
                    want,
                    got: got_code,
                    msg: got_msg,
                });
            }
            None => {
                return Err(FixtureFailure::UnexpectedJsonRpcError {
                    index,
                    code: got_code,
                    msg: got_msg,
                });
            }
        }
    }

    // Success path — extract status
    let result = resp
        .get("result")
        .ok_or_else(|| FixtureFailure::MalformedResponse {
            index,
            detail: "missing result".into(),
        })?;
    let status = result
        .get("status")
        .and_then(|v| v.as_str())
        .ok_or_else(|| FixtureFailure::MalformedResponse {
            index,
            detail: "missing status".into(),
        })?
        .to_string();

    let expected = if payload.valid() { "VALID" } else { "INVALID" };
    if status != expected {
        let validation_error = result
            .get("validationError")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        return Err(FixtureFailure::WrongStatus {
            index,
            expected: expected.into(),
            got: status,
            validation_error,
        });
    }

    // Expected error code but got success response
    if let Some(want) = payload.error_code {
        return Err(FixtureFailure::MissingErrorCode { index, want });
    }

    // INVALID payloads: check validationError only when the fixture expects one
    if status == "INVALID" && opts.check_exception_names {
        let got_ve = result
            .get("validationError")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        match (&payload.validation_error, got_ve) {
            (None, _) => return Ok(()),
            (Some(_), None) => return Err(FixtureFailure::MissingValidationError { index }),
            (Some(expected_ve), Some(got)) => {
                let candidates: Vec<&str> = match expected_ve {
                    ValidationError::Single(s) => vec![s.as_str()],
                    ValidationError::List(v) => v.iter().map(|s| s.as_str()).collect(),
                };
                // Use the ported EthrexExceptionMapper: try the canonical-exception lookup
                // first; fall back to literal substring for forward-compatibility with
                // exceptions the mapper hasn't been taught yet.
                let matches = candidates
                    .iter()
                    .any(|c| crate::exception_mapper::matches(c, &got) || got.contains(c));
                if !matches {
                    let f = FixtureFailure::ValidationErrorMismatch {
                        index,
                        expected: candidates.iter().map(|s| s.to_string()).collect(),
                        got,
                        strict: opts.strict_exceptions,
                    };
                    if opts.strict_exceptions {
                        return Err(f);
                    } else {
                        eprintln!("warn [{name}]: {f}");
                    }
                }
            }
        }
    }

    Ok(())
}

/// Verify an FCU response contains `payloadStatus.status == "VALID"`.
fn assert_fcu_valid(resp: &Value) -> Result<(), String> {
    if let Some(err) = resp.get("error") {
        let msg = err
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        return Err(format!("FCU returned JSON-RPC error: {msg}"));
    }
    let status = resp
        .get("result")
        .and_then(|r| r.get("payloadStatus"))
        .and_then(|ps| ps.get("status"))
        .and_then(|s| s.as_str())
        .unwrap_or("<missing>");
    if status != "VALID" {
        return Err(format!("FCU status expected=VALID  got={status}"));
    }
    Ok(())
}

/// Extract `result.hash` from an `eth_getBlockByNumber` response and parse it.
fn parse_block_hash(block_resp: &Value) -> Result<H256, String> {
    let hash_str = block_resp
        .get("result")
        .and_then(|r| r.get("hash"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| "result.hash missing or not a string".to_string())?;
    hash_str
        .parse::<H256>()
        .map_err(|e| format!("invalid block hash hex '{hash_str}': {e}"))
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::EngineFixtureFile;

    fn inmem_opts() -> RunOptions {
        RunOptions {
            backend: Backend::InMemory,
            check_exception_names: true,
            strict_exceptions: false,
            bal_parallel_exec: true,
            precompile_cache: true,
        }
    }

    #[test]
    fn rejection_error_is_the_clients_error() {
        let error = |resp: Value| rejection_error(&resp);
        assert_eq!(
            error(serde_json::json!({"result": {"status": "INVALID", "validationError": "bad"}})),
            Some("bad".to_string())
        );
        assert_eq!(
            error(serde_json::json!({"result": {"status": "VALID", "validationError": null}})),
            None
        );
        assert_eq!(
            error(serde_json::json!({"error": {"code": -38005, "message": "Unsupported fork"}})),
            Some("-38005: Unsupported fork".to_string())
        );
        assert_eq!(
            error(serde_json::json!({"error": {"code": -32602, "message": "m", "data": "d"}})),
            Some("-32602: m: d".to_string())
        );
        assert_eq!(
            error(
                serde_json::json!({"error": {"code": -32602, "message": "m", "data": {"err": "e"}}})
            ),
            Some(r#"-32602: m: {"err":"e"}"#.to_string())
        );
    }

    /// Pre-Paris fixtures must return SkippedPreParis (is_skip() == true).
    #[tokio::test]
    async fn pre_paris_returns_skip() {
        let raw = serde_json::json!({
            "test_london_skip": {
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
                "engineNewPayloads": [{
                    "params": [{}],
                    "newPayloadVersion": "1",
                    "forkchoiceUpdatedVersion": "1"
                }]
            }
        })
        .to_string();
        let fixtures: EngineFixtureFile = serde_json::from_str(&raw).unwrap();
        let opts = inmem_opts();
        let (name, fixture) = fixtures.iter().next().unwrap();
        let err = run_fixture(name, fixture, &opts, &mut Vec::new())
            .await
            .expect_err("London fixture must be skipped");
        assert!(err.is_skip(), "expected SkippedPreParis, got: {err}");
    }
}
