/// Shared fixture types for app-specific offline tests.
/// Each fixture is captured from a real deployment's prover/committer logs.
use ethrex_common::{H256, U256};
use ethrex_common::types::balance_diff::BalanceDiff;
use ethrex_guest_program::l2::ProgramOutput;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

// ── JSON schema ──────────────────────────────────────────────

#[derive(Deserialize)]
pub struct TestFixture {
    pub app: String,
    pub batch_number: u64,
    pub program_type_id: u8,
    pub chain_id: u64,
    pub description: String,
    pub prover: ProverFixture,
    pub committer: CommitterFixture,
}

#[derive(Deserialize)]
pub struct ProverFixture {
    pub initial_state_hash: String,
    pub final_state_hash: String,
    pub l1_out_messages_merkle_root: String,
    pub l1_in_messages_rolling_hash: String,
    pub blob_versioned_hash: String,
    pub last_block_hash: String,
    pub non_privileged_count: u64,
    pub balance_diffs: Vec<BalanceDiffFixture>,
    pub l2_in_message_rolling_hashes: Vec<(u64, String)>,
    pub encoded_public_values: String,
    pub sha256_public_values: String,
}

#[derive(Deserialize)]
pub struct BalanceDiffFixture {
    pub chain_id: String,
    pub value: String,
    pub message_hashes: Vec<String>,
}

#[derive(Deserialize)]
pub struct CommitterFixture {
    pub new_state_root: String,
    pub withdrawals_merkle_root: String,
    pub priv_tx_rolling_hash: String,
    pub non_privileged_txs: u64,
    pub balance_diffs: Vec<BalanceDiffFixture>,
    pub l2_in_message_rolling_hashes: Vec<(u64, String)>,
}

// ── Loader ───────────────────────────────────────────────────

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Discover all app directories under tests/fixtures/ that contain at least one .json file.
pub fn discover_all_apps() -> Vec<String> {
    let dir = fixtures_dir();
    let mut apps = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let has_json = std::fs::read_dir(&path)
                    .map(|rd| rd.flatten().any(|e| e.path().extension().is_some_and(|ext| ext == "json")))
                    .unwrap_or(false);
                if has_json {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        apps.push(name.to_string());
                    }
                }
            }
        }
    }
    apps.sort();
    apps
}

pub fn load_fixture(app: &str, filename: &str) -> TestFixture {
    let path = fixtures_dir().join(app).join(filename);
    let data = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read fixture {}: {e}", path.display()));
    serde_json::from_str(&data)
        .unwrap_or_else(|e| panic!("Failed to parse fixture {}: {e}", path.display()))
}

pub fn load_all_fixtures(app: &str) -> Vec<TestFixture> {
    let dir = fixtures_dir().join(app);
    let mut fixtures = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        let mut paths: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
            .map(|e| e.path())
            .collect();
        paths.sort();
        for path in paths {
            let data = std::fs::read_to_string(&path).unwrap();
            let fixture: TestFixture = serde_json::from_str(&data).unwrap();
            fixtures.push(fixture);
        }
    }
    fixtures
}

// ── Conversion helpers ───────────────────────────────────────

pub fn hex_to_h256(s: &str) -> H256 {
    let s = s.strip_prefix("0x").unwrap_or(s);
    let bytes = hex::decode(s).unwrap_or_else(|e| panic!("bad hex '{s}': {e}"));
    assert_eq!(bytes.len(), 32, "expected 32 bytes, got {}", bytes.len());
    H256::from_slice(&bytes)
}

pub fn hex_to_bytes(s: &str) -> Vec<u8> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    hex::decode(s).unwrap_or_else(|e| panic!("bad hex '{s}': {e}"))
}

/// Build a ProgramOutput from the prover fixture fields.
pub fn fixture_to_program_output(f: &TestFixture) -> ProgramOutput {
    let p = &f.prover;
    ProgramOutput {
        initial_state_hash: hex_to_h256(&p.initial_state_hash),
        final_state_hash: hex_to_h256(&p.final_state_hash),
        l1_out_messages_merkle_root: hex_to_h256(&p.l1_out_messages_merkle_root),
        l1_in_messages_rolling_hash: hex_to_h256(&p.l1_in_messages_rolling_hash),
        blob_versioned_hash: hex_to_h256(&p.blob_versioned_hash),
        last_block_hash: hex_to_h256(&p.last_block_hash),
        chain_id: U256::from(f.chain_id),
        non_privileged_count: U256::from(p.non_privileged_count),
        balance_diffs: p.balance_diffs.iter().map(|bd| {
            BalanceDiff {
                chain_id: U256::from_str_radix(
                    bd.chain_id.strip_prefix("0x").unwrap_or(&bd.chain_id), 16
                ).unwrap(),
                value: U256::from_str_radix(
                    bd.value.strip_prefix("0x").unwrap_or(&bd.value), 16
                ).unwrap(),
                value_per_token: vec![],
                message_hashes: bd.message_hashes.iter().map(|h| hex_to_h256(h)).collect(),
            }
        }).collect(),
        l2_in_message_rolling_hashes: p.l2_in_message_rolling_hashes
            .iter()
            .map(|(cid, h)| (*cid, hex_to_h256(h)))
            .collect(),
    }
}

/// Compute sha256 of encoded public values and return hex string.
pub fn sha256_hex(data: &[u8]) -> String {
    format!("0x{}", hex::encode(Sha256::digest(data)))
}
