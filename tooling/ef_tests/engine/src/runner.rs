use std::fmt;

use ethrex_common::H256;
use serde_json::Value;

use crate::fixture::{EngineFixture, FixturePayload, ValidationError, is_pre_paris};
use crate::harness::{Backend, EngineApiHarness};

// ─── Public types ─────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct RunOptions {
    pub backend: Backend,
    pub strict_exceptions: bool,
}

impl RunOptions {
    pub fn from_env() -> Self {
        Self {
            backend: parse_backend_env(),
            strict_exceptions: std::env::var("ETHREX_ENGINE_STRICT_EXCEPTIONS")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
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
    FollowupFcu {
        index: usize,
        msg: String,
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
            FixtureFailure::FollowupFcu { index, msg } => {
                write!(f, "followup_fcu[{index}]  error={msg}")
            }
        }
    }
}

// ─── Main entry point ─────────────────────────────────────────────────────────

pub async fn run_fixture(
    name: &str,
    fix: &EngineFixture,
    opts: &RunOptions,
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
    let harness = Box::pin(EngineApiHarness::from_genesis(genesis, opts.backend))
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
        let resp = Box::pin(harness.new_payload(payload.new_payload_version, &payload.params))
            .await
            .map_err(|e| FixtureFailure::PayloadRpc {
                index: i,
                msg: e.to_string(),
            })?;
        check_payload_response(&resp, payload, i, opts.strict_exceptions, name)?;
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

    Ok(())
}

// ─── Private helpers ──────────────────────────────────────────────────────────

fn check_payload_response(
    resp: &Value,
    payload: &FixturePayload,
    index: usize,
    strict: bool,
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
    if status == "INVALID" {
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
                        strict,
                    };
                    if strict {
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
            strict_exceptions: false,
        }
    }

    /// Happy path: every non-pre-Paris fixture in the sample file must pass.
    ///
    /// Uses a multi-thread runtime so the heavy engine API awaits run on a worker
    /// thread (which gets its own full stack), keeping the test thread stack small.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn runs_sample_fixture_ok() {
        let raw = include_str!("../fixtures_sample/one_payload.json");
        let fixtures: EngineFixtureFile = serde_json::from_str(raw).expect("fixture parse");
        let opts = inmem_opts();
        let mut exercised = 0usize;
        for (name, fixture) in &fixtures {
            let result = run_fixture(name, fixture, &opts).await;
            match result {
                Ok(()) => exercised += 1,
                Err(e) if e.is_skip() => {}
                Err(e) => panic!("fixture '{name}' failed: {e}"),
            }
        }
        assert!(
            exercised > 0,
            "all fixtures were skipped as pre-Paris — sample fixture must include at least one Paris+ fixture"
        );
    }

    /// INVALID-payload path: a fixture with a bad payload must pass when ethrex
    /// correctly rejects it (runner succeeds because actual == expected INVALID).
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn runs_invalid_payload_fixture_ok() {
        let raw = include_str!("../fixtures_sample/invalid_payload.json");
        let fixtures: EngineFixtureFile =
            serde_json::from_str(raw).expect("invalid_payload fixture parse");
        let opts = inmem_opts();
        let mut exercised = 0usize;
        for (name, fixture) in &fixtures {
            let result = run_fixture(name, fixture, &opts).await;
            match result {
                Ok(()) => exercised += 1,
                Err(e) if e.is_skip() => {}
                Err(e) => panic!("fixture '{name}' failed: {e}"),
            }
        }
        assert!(
            exercised > 0,
            "all invalid_payload fixtures were skipped — must include at least one Paris+ fixture"
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
        let err = run_fixture(name, fixture, &opts)
            .await
            .expect_err("London fixture must be skipped");
        assert!(err.is_skip(), "expected SkippedPreParis, got: {err}");
    }
}
