use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    fork_choice::apply_fork_choice,
};
use ethrex_common::{
    H256,
    constants::EMPTY_KECCACK_HASH,
    types::{
        Account as CoreAccount,
        requests::compute_requests_hash,
    },
};
use ethrex_storage::{EngineType, Store};
use serde::Serialize;

use crate::types::{
    EngineNewPayload, EngineTestUnit, FixtureExecutionPayload,
    compute_raw_bal_hash, parse_beacon_root, parse_execution_requests,
    parse_versioned_hashes,
};

#[derive(Serialize)]
pub struct TestResult {
    pub name: String,
    pub pass: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Run a single engine test. Returns `Ok(())` on pass, `Err(msg)` on
/// failure.
pub async fn run_engine_test(
    test_key: &str,
    test: &EngineTestUnit,
) -> Result<(), String> {
    let store = build_store(test).await;
    let blockchain =
        Blockchain::new(store.clone(), BlockchainOptions::default());

    // Track the current head hash for fork-choice updates.
    #[allow(unused_assignments)]
    let mut head_hash = test.genesis_block_header.hash;

    for (i, payload_entry) in
        test.engine_new_payloads.iter().enumerate()
    {
        let expects_error = payload_entry.expects_error();
        let expects_rpc_error = payload_entry.error_code.is_some();

        // ---- 1. Validate engine version-specific parameters ----
        if let Err(rpc_err) =
            validate_engine_params(payload_entry)
        {
            if expects_rpc_error {
                // RPC-level error expected (e.g. -32602), skip this
                // payload.
                continue;
            }
            return Err(format!(
                "payload[{i}]: unexpected RPC validation error: {rpc_err}"
            ));
        }
        if expects_rpc_error {
            return Err(format!(
                "payload[{i}]: expected RPC error code {:?} but \
                 validation passed",
                payload_entry.error_code
            ));
        }

        // ---- 2. Parse the execution payload ----
        let payload_json = &payload_entry.params[0];
        let fixture_payload: FixtureExecutionPayload =
            serde_json::from_value(payload_json.clone()).map_err(
                |e| {
                    format!(
                        "payload[{i}]: failed to parse \
                         ExecutionPayload: {e}"
                    )
                },
            )?;
        let block_hash = fixture_payload.block_hash;

        // ---- 3. Parse version-dependent extra params ----
        let version = payload_entry.new_payload_version;

        let (versioned_hashes, beacon_root, requests_hash, bal_hash) =
            parse_extra_params(payload_entry, payload_json, version)
                .map_err(|e| {
                    format!("payload[{i}]: {e}")
                })?;

        // ---- 4. Convert payload to Block ----
        // Transaction RLP decode failures are treated as INVALID
        // (same as the real engine handler returning
        // PayloadStatus::invalid_with_err).
        let block = match fixture_payload
            .into_block(beacon_root, requests_hash, bal_hash)
        {
            Ok(b) => b,
            Err(decode_err) => {
                if expects_error {
                    continue;
                }
                return Err(format!(
                    "payload[{i}]: {decode_err}"
                ));
            }
        };

        if let Some(ref expected_hashes) = versioned_hashes {
            let actual_hashes: Vec<H256> = block
                .body
                .transactions
                .iter()
                .flat_map(|tx| tx.blob_versioned_hashes())
                .collect();
            if *expected_hashes != actual_hashes {
                if expects_error {
                    continue;
                }
                return Err(format!(
                    "payload[{i}]: blob versioned hashes mismatch"
                ));
            }
        }

        // ---- 5. Validate block hash ----
        let actual_hash = block.hash();
        if block_hash != actual_hash {
            if expects_error {
                continue;
            }
            return Err(format!(
                "payload[{i}]: block hash mismatch: \
                 expected {block_hash:#x}, got {actual_hash:#x}"
            ));
        }

        // ---- 6. Execute the block through the real pipeline ----
        let chain_result =
            blockchain.add_block_pipeline(block.clone(), None);

        match chain_result {
            Err(error) => {
                if !expects_error {
                    return Err(format!(
                        "payload[{i}]: execution failed \
                         unexpectedly: {error:?}"
                    ));
                }
                // Expected error -- do NOT advance fork choice, but
                // continue processing subsequent payloads.
                continue;
            }
            Ok(()) => {
                if expects_error {
                    return Err(format!(
                        "payload[{i}]: expected error \
                         ({:?}) but execution succeeded",
                        payload_entry.validation_error
                    ));
                }
            }
        }

        // ---- 7. Apply fork choice (advance the canonical head) ----
        head_hash = block_hash;
        apply_fork_choice(
            &store, head_hash, head_hash, head_hash,
        )
        .await
        .map_err(|e| {
            format!(
                "payload[{i}]: fork choice update \
                 failed: {e:?}"
            )
        })?;
    }

    // ---- 8. Verify post-state ----
    verify_post_state(test_key, test, &store).await?;

    Ok(())
}

// ---- Engine parameter validation ----
//
// These mirror the checks in ethrex's RPC engine handlers
// (validate_execution_payload_v1 .. v4, validate_execution_requests).

fn validate_engine_params(
    entry: &EngineNewPayload,
) -> Result<(), String> {
    let version = entry.new_payload_version;
    let payload_json = &entry.params[0];

    // Check param count matches version expectation.
    match version {
        1 => {
            if entry.params.len() != 1 {
                return Err(format!(
                    "V1 expects 1 param, got {}",
                    entry.params.len()
                ));
            }
        }
        2 => {
            if entry.params.len() != 1 {
                return Err(format!(
                    "V2 expects 1 param, got {}",
                    entry.params.len()
                ));
            }
        }
        3 => {
            if entry.params.len() != 3 {
                return Err(format!(
                    "V3 expects 3 params, got {}",
                    entry.params.len()
                ));
            }
        }
        4 | 5 => {
            if entry.params.len() != 4 {
                return Err(format!(
                    "V{version} expects 4 params, got {}",
                    entry.params.len()
                ));
            }
        }
        _ => {
            return Err(format!(
                "Unsupported newPayload version: {version}"
            ));
        }
    }

    let has_withdrawals = payload_json.get("withdrawals").is_some()
        && !payload_json["withdrawals"].is_null();
    let has_blob_gas =
        payload_json.get("blobGasUsed").is_some()
            && !payload_json["blobGasUsed"].is_null();
    let has_excess_blob_gas =
        payload_json.get("excessBlobGas").is_some()
            && !payload_json["excessBlobGas"].is_null();

    match version {
        1 => {
            // V1: no withdrawals, no blob fields
            if has_withdrawals {
                return Err(
                    "V1: withdrawals must not be present"
                        .to_string(),
                );
            }
            if has_blob_gas || has_excess_blob_gas {
                return Err(
                    "V1: blob gas fields must not be present"
                        .to_string(),
                );
            }
        }
        2 => {
            // V2: withdrawals required for Shanghai, no blob fields
            if has_blob_gas || has_excess_blob_gas {
                return Err(
                    "V2: blob gas fields must not be present"
                        .to_string(),
                );
            }
        }
        3 | 4 | 5 => {
            // V3+: withdrawals required, blob gas required
            if !has_withdrawals {
                return Err(format!(
                    "V{version}: withdrawals required"
                ));
            }
            if !has_blob_gas || !has_excess_blob_gas {
                return Err(format!(
                    "V{version}: blob gas fields required"
                ));
            }
        }
        _ => {}
    }

    // V4/V5: validate execution requests ordering
    if version >= 4 && entry.params.len() >= 4 {
        if let Ok(requests) =
            parse_execution_requests(&entry.params[3])
        {
            let mut last_type: i32 = -1;
            for req in &requests {
                if req.0.len() < 2 {
                    return Err(
                        "Empty request data".to_string()
                    );
                }
                let req_type = req.0[0] as i32;
                if last_type >= req_type {
                    return Err(
                        "Invalid requests order".to_string()
                    );
                }
                last_type = req_type;
            }
        }
    }

    Ok(())
}

/// Parse the version-dependent extra parameters from the fixture.
fn parse_extra_params(
    entry: &EngineNewPayload,
    payload_json: &serde_json::Value,
    version: u8,
) -> Result<
    (
        Option<Vec<H256>>,
        Option<H256>,
        Option<H256>,
        Option<H256>,
    ),
    String,
> {
    let mut versioned_hashes = None;
    let mut beacon_root = None;
    let mut requests_hash = None;
    let mut bal_hash = None;

    if version >= 3 && entry.params.len() >= 3 {
        versioned_hashes =
            Some(parse_versioned_hashes(&entry.params[1])?);
        beacon_root = Some(parse_beacon_root(&entry.params[2])?);
    }

    if version >= 4 && entry.params.len() >= 4 {
        let requests =
            parse_execution_requests(&entry.params[3])?;
        requests_hash = Some(compute_requests_hash(&requests));
    }

    if version >= 5 {
        bal_hash = compute_raw_bal_hash(payload_json);
    }

    Ok((versioned_hashes, beacon_root, requests_hash, bal_hash))
}

// ---- Store setup ----

async fn build_store(test: &EngineTestUnit) -> Store {
    let mut store = Store::new("store.db", EngineType::InMemory)
        .expect("Failed to build DB for engine test");
    let genesis = test.get_genesis();
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to add genesis state");
    store
}

// ---- Post-state verification ----

async fn verify_post_state(
    test_key: &str,
    test: &EngineTestUnit,
    store: &Store,
) -> Result<(), String> {
    let latest_block_number =
        store.get_latest_block_number().await.map_err(|e| {
            format!("Failed to get latest block number: {e}")
        })?;

    // Verify account state
    if let Some(post_state) = &test.post_state {
        for (addr, account) in post_state {
            let expected: CoreAccount = account.clone().into();

            let db_info = store
                .get_account_info(latest_block_number, *addr)
                .await
                .map_err(|e| format!("DB read error: {e}"))?
                .ok_or_else(|| {
                    format!(
                        "{test_key}: account {addr} not found \
                         in post-state"
                    )
                })?;

            if db_info != expected.info {
                return Err(format!(
                    "{test_key}: account {addr} info mismatch: \
                     expected {:?}, got {:?}",
                    expected.info, db_info
                ));
            }

            let code_hash = expected.info.code_hash;
            if code_hash != *EMPTY_KECCACK_HASH {
                let db_code = store
                    .get_account_code(code_hash)
                    .map_err(|e| format!("DB read error: {e}"))?
                    .ok_or_else(|| {
                        format!(
                            "{test_key}: code {code_hash} \
                             not found"
                        )
                    })?;
                if db_code != expected.code {
                    return Err(format!(
                        "{test_key}: code mismatch for {addr}"
                    ));
                }
            }

            for (key, value) in &expected.storage {
                let db_val = store
                    .get_storage_at(
                        latest_block_number,
                        *addr,
                        *key,
                    )
                    .map_err(|e| format!("DB read error: {e}"))?
                    .ok_or_else(|| {
                        format!(
                            "{test_key}: storage {key} for \
                             {addr} not found"
                        )
                    })?;
                if db_val != *value {
                    return Err(format!(
                        "{test_key}: storage mismatch for \
                         {addr} key {key}: expected {value}, \
                         got {db_val}"
                    ));
                }
            }
        }
    }

    // Verify lastblockhash
    let last_header = store
        .get_block_header(latest_block_number)
        .map_err(|e| format!("DB read error: {e}"))?
        .ok_or_else(|| {
            format!("{test_key}: last block header not found")
        })?;
    let last_hash = last_header.hash();

    if test.lastblockhash != last_hash {
        return Err(format!(
            "{test_key}: lastblockhash mismatch: expected \
             {:#x}, got {last_hash:#x}",
            test.lastblockhash
        ));
    }

    Ok(())
}

impl EngineNewPayload {
    /// Returns true if this payload entry expects an error (INVALID
    /// status or validation error).
    pub fn expects_error(&self) -> bool {
        self.validation_error
            .as_ref()
            .is_some_and(|s| !s.is_empty())
    }
}
