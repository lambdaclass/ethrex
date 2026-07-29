//! Host-side fixture tests for the stateless-validator guest logic.
//!
//! Runs `run_stateless_validation` over EEST `blockchain_test` fixtures that
//! embed `statelessInputBytes`/`statelessOutputBytes` (the `tests-zkevm`
//! releases of `ethereum/execution-specs`) and asserts the produced output is
//! byte-identical to the expected output. This is both the PR gate for guest
//! breakage and the equivalence harness for comparing guest integrations.
//!
//! Fixtures are not committed to the repository. Point
//! `ETHREX_STATELESS_FIXTURES` at a directory containing fixture JSON files,
//! e.g. the `fixtures/blockchain_tests` subdirectory of
//! `https://github.com/ethereum/execution-specs/releases/download/tests-zkevm@v0.6.2/fixtures_zkevm.tar.gz`.
//! When the variable is unset the test is skipped so plain `cargo test` runs
//! stay green without a download.
#![cfg(feature = "host")]

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use ethrex_guest_program::crypto::NativeCrypto;
use ethrex_stateless_validator::run_stateless_validation;
use serde::{Deserialize, Deserializer};

const FIXTURES_DIR_ENV: &str = "ETHREX_STATELESS_FIXTURES";

/// A fixture normalized to canonical schema-prefixed SSZ input and expected
/// output bytes, mirroring the loader in ere-guests' stateless-validator-test.
struct Fixture {
    name: String,
    stateless_input_bytes: Vec<u8>,
    stateless_output_bytes: Vec<u8>,
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
fn load_fixtures(dir: &Path) -> Vec<Fixture> {
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

#[test]
fn eest_fixture_equivalence() {
    let Some(dir) = std::env::var_os(FIXTURES_DIR_ENV) else {
        eprintln!(
            "skipping eest_fixture_equivalence: set {FIXTURES_DIR_ENV} to a directory of \
             tests-zkevm blockchain_test fixtures"
        );
        return;
    };
    let fixtures = load_fixtures(Path::new(&dir));
    assert!(
        !fixtures.is_empty(),
        "no fixtures with stateless bytes found under {FIXTURES_DIR_ENV}"
    );

    let crypto = Arc::new(NativeCrypto);
    let mut failures = Vec::new();
    for fixture in &fixtures {
        let output = run_stateless_validation(&fixture.stateless_input_bytes, crypto.clone());
        if output != fixture.stateless_output_bytes {
            failures.push(fixture.name.clone());
        }
    }
    assert!(
        failures.is_empty(),
        "{}/{} fixtures produced output bytes different from the expected \
         statelessOutputBytes: {failures:#?}",
        failures.len(),
        fixtures.len(),
    );
    println!("{} fixtures matched expected output bytes", fixtures.len());
}
