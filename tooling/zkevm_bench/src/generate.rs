//! ethrex-native stress-fixture generator.
//!
//! Turns EEST `blockchain_tests` (genesis + `pre` state + blocks, no
//! embedded witness) into `generate-stress` fixtures for
//! `ethrex-zkevm-bench`, by generating the execution witness with ethrex's
//! own block-execution machinery (`ef_tests_blockchain::test_runner`)
//! instead of the external eth-act `witness-generator-cli` tool used by
//! `src/stress.rs`. Purely host-side: no zisk toolchain involved.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use ef_tests_blockchain::test_runner::blocks_and_witness_for_test;
use ef_tests_blockchain::types::TestUnit;
use ethrex_common::types::block_execution_witness::RpcExecutionWitness;
use ethrex_config::networks::{Network, PublicNetwork};
use flate2::Compression;
use flate2::write::GzEncoder;

use crate::cache::Cache;

/// Walks `input_dir` for `*.json` EEST `blockchain_test` files, generates an
/// ethrex-native execution witness for every valid (no `expectException`)
/// test found, and writes one gzipped Cache-format fixture per test into
/// `out_dir`.
///
/// A single bad fixture (unparseable file, invalid-block test, witness
/// generation failure, ...) is logged to stderr and skipped: one bad
/// fixture must not abort the whole batch.
pub fn run_generate_stress(input_dir: &str, out_dir: &str) -> eyre::Result<()> {
    std::fs::create_dir_all(out_dir)?;

    let mut json_files = Vec::new();
    collect_json_files(Path::new(input_dir), &mut json_files)?;
    json_files.sort();

    let rt = tokio::runtime::Runtime::new()?;

    let mut generated = 0usize;
    let mut skipped = 0usize;

    for path in &json_files {
        let path_str = path.display().to_string();
        let raw = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{path_str}: failed to read: {e}");
                skipped += 1;
                continue;
            }
        };
        let tests: BTreeMap<String, TestUnit> = match serde_json::from_str(&raw) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("{path_str}: failed to parse as a blockchain_test file: {e}");
                skipped += 1;
                continue;
            }
        };

        for (test_name, test) in tests {
            // Invalid-block tests are deliberately unexecutable to the end;
            // they aren't a witness-generation target.
            if test
                .blocks
                .iter()
                .any(|block| block.expect_exception.is_some())
            {
                continue;
            }

            match rt.block_on(blocks_and_witness_for_test(&test)) {
                Ok((blocks, witness)) => {
                    match write_stress_fixture(out_dir, &test_name, blocks, witness, &test) {
                        Ok(()) => generated += 1,
                        Err(e) => {
                            eprintln!("{test_name}: failed to write fixture: {e}");
                            skipped += 1;
                        }
                    }
                }
                Err(e) => {
                    eprintln!("{test_name}: witness generation failed: {e}");
                    skipped += 1;
                }
            }
        }
    }

    println!("generate-stress: wrote {generated} fixture(s) to {out_dir}, skipped {skipped}");
    Ok(())
}

/// Recursively collects every `*.json` file under `dir`.
fn collect_json_files(dir: &Path, out: &mut Vec<PathBuf>) -> eyre::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_json_files(&path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("json") {
            out.push(path);
        }
    }
    Ok(())
}

/// Replaces any character that isn't filesystem-friendly (EEST test names
/// carry `[]`, `-`, `::`, ...) with `_` so the fixture name is a valid,
/// single-path-component file name.
fn sanitize_file_stem(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Converts the generated `(blocks, ExecutionWitness)` pair plus the test's
/// own `ChainConfig` into the `Cache` JSON shape `src/cache.rs` reads, and
/// writes it gzipped to `out_dir/<test_name>.json.gz`.
///
/// The test's `ChainConfig` (not a network genesis default) is embedded
/// directly: these are EEST test chains, not mainnet, so
/// `cache_to_program_input` must use the fork rules the test itself defines.
/// `network` is set to a fixed placeholder — `cache_to_program_input` only
/// falls back to it when `chain_config` is absent, which never happens here.
fn write_stress_fixture(
    out_dir: &str,
    test_name: &str,
    blocks: Vec<ethrex_common::types::Block>,
    witness: ethrex_common::types::block_execution_witness::ExecutionWitness,
    test: &TestUnit,
) -> eyre::Result<()> {
    let rpc_witness = RpcExecutionWitness::try_from(witness)
        .map_err(|e| eyre::eyre!("witness -> RpcExecutionWitness: {e:?}"))?;
    let chain_config = *test.network.chain_config();

    let cache = Cache {
        blocks,
        witness: rpc_witness,
        network: Network::PublicNetwork(PublicNetwork::Mainnet),
        chain_config: Some(chain_config),
    };

    let json = serde_json::to_vec(&cache)?;
    let out_path = Path::new(out_dir).join(format!("{}.json.gz", sanitize_file_stem(test_name)));
    let file = std::fs::File::create(&out_path)?;
    let mut encoder = GzEncoder::new(file, Compression::default());
    encoder.write_all(&json)?;
    encoder.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Small, single-block Amsterdam blockchain_test with `pre` state and no
    // embedded witness requirement (the ethrex-native path regenerates the
    // witness itself). Gitignored (downloaded via `make zkevm-vectors` in
    // `tooling/ef_tests/blockchain`), so this test skips gracefully when
    // absent, mirroring the pattern in `micro.rs`/`tests/smoke.rs`.
    const FIXTURE_DIR: &str = "../ef_tests/blockchain/vectors_zkevm/eest/for_amsterdam/amsterdam/eip8025_optional_proofs/witness_7702";

    #[test]
    fn generates_loadable_cache_from_eest_blockchain_test() {
        if !std::path::Path::new(FIXTURE_DIR).exists() {
            eprintln!(
                "skipping: EEST fixture dir absent (run `make zkevm-vectors` in tooling/ef_tests/blockchain)"
            );
            return;
        }

        // Isolate this test's input to a single known-small fixture (the
        // directory has several `witness_7702` variants) so the run stays
        // fast and deterministic.
        let tmp = std::env::temp_dir().join(format!(
            "zkevm_bench_generate_stress_test_{}",
            std::process::id()
        ));
        let input_dir = tmp.join("in");
        let out_dir = tmp.join("out");
        std::fs::create_dir_all(&input_dir).unwrap();
        std::fs::copy(
            format!("{FIXTURE_DIR}/witness_codes_delegation_chain.json"),
            input_dir.join("witness_codes_delegation_chain.json"),
        )
        .expect("copy fixture");

        run_generate_stress(input_dir.to_str().unwrap(), out_dir.to_str().unwrap())
            .expect("generate-stress should succeed");

        let entries: Vec<_> = std::fs::read_dir(&out_dir)
            .expect("read out_dir")
            .map(|e| e.unwrap().path())
            .collect();
        assert!(
            !entries.is_empty(),
            "generate-stress produced no fixtures for a known-valid EEST test"
        );

        let produced = entries[0].to_str().unwrap();
        let cache = crate::cache::load_cache(produced).expect("produced fixture should load");
        assert!(!cache.blocks.is_empty());
        let input =
            crate::cache::cache_to_program_input(cache).expect("should build program input");
        assert!(!input.blocks.is_empty());

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
