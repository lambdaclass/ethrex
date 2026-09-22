//! Shared fixture loading for the host-side tests.
//!
//! Fixtures are EEST `blockchain_test` JSON files embedding
//! `statelessInputBytes`/`statelessOutputBytes` (the `tests-zkevm` releases
//! of `ethereum/execution-specs`); they are downloaded out of band and pointed
//! to by the `ETHREX_STATELESS_FIXTURES` environment variable, never
//! committed.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Deserializer};

pub const FIXTURES_DIR_ENV: &str = "ETHREX_STATELESS_FIXTURES";

/// A fixture normalized to canonical schema-prefixed SSZ input and expected
/// output bytes, mirroring the loader in ere-guests' stateless-validator-test.
pub struct Fixture {
    pub name: String,
    pub stateless_input_bytes: Vec<u8>,
    pub stateless_output_bytes: Vec<u8>,
}

type EestFixtureFile = BTreeMap<String, EestTest>;

/// Minimal projection of an EEST `blockchain_test` body.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EestTest {
    blocks: Vec<EestBlock>,
}

/// Minimal projection of a single EEST block.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EestBlock {
    #[serde(default, deserialize_with = "opt_hex_bytes")]
    stateless_input_bytes: Option<Vec<u8>>,
    #[serde(default, deserialize_with = "opt_hex_bytes")]
    stateless_output_bytes: Option<Vec<u8>>,
}

/// Deserializes an optional `0x`-prefixed hex string into bytes.
fn opt_hex_bytes<'de, D>(deserializer: D) -> Result<Option<Vec<u8>>, D::Error>
where
    D: Deserializer<'de>,
{
    let Some(s) = Option::<String>::deserialize(deserializer)? else {
        return Ok(None);
    };
    hex::decode(s.trim_start_matches("0x"))
        .map(Some)
        .map_err(serde::de::Error::custom)
}

/// Recursively collects every `.json` fixture file under `dir`.
fn fixture_files(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("fixture directory should be readable") {
        let path = entry
            .expect("fixture directory entry should be readable")
            .path();
        if path.is_dir() {
            fixture_files(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "json") {
            files.push(path);
        }
    }
}

/// Loads every fixture under `dir`, sorted by name for determinism. Blocks
/// without embedded stateless bytes (e.g. pre-zkevm fixtures) are skipped.
pub fn load_fixtures(dir: &Path) -> Vec<Fixture> {
    let mut files = Vec::new();
    fixture_files(dir, &mut files);
    let mut fixtures: Vec<Fixture> = files
        .iter()
        .flat_map(|path| {
            let bytes = fs::read(path).expect("fixture file should be readable");
            let tests: EestFixtureFile =
                serde_json::from_slice(&bytes).expect("fixture file should be valid EEST JSON");
            tests
                .into_iter()
                .flat_map(|(test_id, test)| {
                    test.blocks
                        .into_iter()
                        .enumerate()
                        .filter_map(move |(idx, block)| {
                            let input = block.stateless_input_bytes?;
                            let output = block.stateless_output_bytes?;
                            (!input.is_empty()).then(|| Fixture {
                                name: format!("{test_id}#block{idx}"),
                                stateless_input_bytes: input,
                                stateless_output_bytes: output,
                            })
                        })
                })
                .collect::<Vec<_>>()
        })
        .collect();
    fixtures.sort_by(|a, b| a.name.cmp(&b.name));
    fixtures
}
