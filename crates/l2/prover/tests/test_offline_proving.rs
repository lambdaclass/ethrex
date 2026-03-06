//! Phase 3: Offline proving tests using fixture stdin.bin files.
//!
//! These tests re-prove batches from serialized SP1Stdin fixtures,
//! verifying that proving is deterministic and public_values match.
//!
//! Requires:
//! - SP1 toolchain installed
//! - Fixture files collected via `ETHREX_DUMP_FIXTURES` (see fixture-data-collection.md)
//!
//! ```sh
//! cargo test -p ethrex-prover --features sp1 -- --ignored offline_prove
//! ```

#[cfg(feature = "sp1")]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod sp1_offline_proving {
    use sp1_sdk::{CpuProver, Prover, SP1ProofMode, SP1Stdin, SP1ProofWithPublicValues};
    use std::path::{Path, PathBuf};

    fn fixtures_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../guest-program/tests/fixtures")
            .canonicalize()
            .expect("fixtures directory should exist")
    }

    /// Discover all stdin.bin fixtures across all apps.
    fn discover_stdin_fixtures() -> Vec<(String, PathBuf)> {
        let dir = fixtures_dir();
        let mut results = Vec::new();
        if let Ok(apps) = std::fs::read_dir(&dir) {
            for app_entry in apps.flatten() {
                let app_path = app_entry.path();
                if !app_path.is_dir() {
                    continue;
                }
                let app_name = app_path
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_string();
                // Look for batch directories containing stdin.bin
                if let Ok(batches) = std::fs::read_dir(&app_path) {
                    for batch_entry in batches.flatten() {
                        let batch_path = batch_entry.path();
                        if batch_path.is_dir() {
                            let stdin_path = batch_path.join("stdin.bin");
                            if stdin_path.exists() {
                                let label = format!(
                                    "{}/{}",
                                    app_name,
                                    batch_path.file_name().unwrap().to_str().unwrap()
                                );
                                results.push((label, stdin_path));
                            }
                        }
                    }
                }
                // Also check app-level stdin.bin (flat layout)
                let flat_stdin = app_path.join("stdin.bin");
                if flat_stdin.exists() {
                    results.push((app_name, flat_stdin));
                }
            }
        }
        results.sort_by(|a, b| a.0.cmp(&b.0));
        results
    }

    #[test]
    #[ignore = "requires SP1 toolchain + stdin.bin fixtures (~10 min per batch)"]
    fn offline_prove_from_fixtures() {
        let fixtures = discover_stdin_fixtures();
        if fixtures.is_empty() {
            eprintln!("SKIP: no stdin.bin fixtures found. Collect with ETHREX_DUMP_FIXTURES.");
            return;
        }

        let elf = ethrex_guest_program::ZKVM_SP1_PROGRAM_ELF;
        let client = CpuProver::new();
        let (pk, vk) = client.setup(elf);

        for (label, stdin_path) in &fixtures {
            eprintln!("[{label}] proving from {}", stdin_path.display());
            let stdin_bytes = std::fs::read(stdin_path).expect("read stdin.bin");

            let mut stdin = SP1Stdin::new();
            stdin.write_slice(&stdin_bytes);

            let proof: SP1ProofWithPublicValues =
                <CpuProver as Prover<_>>::prove(&client, &pk, &stdin, SP1ProofMode::Compressed)
                    .unwrap_or_else(|e| panic!("[{label}] proving failed: {e}"));

            // Verify the proof
            client
                .verify(&proof, &vk)
                .unwrap_or_else(|e| panic!("[{label}] verification failed: {e}"));

            // If a corresponding prover.json exists, compare public_values
            let batch_dir = stdin_path.parent().unwrap();
            let prover_json = batch_dir.join("prover.json");
            if prover_json.exists() {
                let pj: serde_json::Value =
                    serde_json::from_str(&std::fs::read_to_string(&prover_json).unwrap()).unwrap();
                if let Some(expected_hex) = pj.get("encoded_public_values").and_then(|v| v.as_str())
                {
                    let expected = hex::decode(expected_hex.strip_prefix("0x").unwrap_or(expected_hex))
                        .expect("decode hex");
                    let actual = proof.public_values.as_slice();
                    assert_eq!(
                        actual, &expected[..],
                        "[{label}] public_values mismatch after re-proving"
                    );
                    eprintln!("[{label}] OK — public_values match ({} bytes)", actual.len());
                }
            } else {
                eprintln!("[{label}] OK — proved successfully (no prover.json to compare)");
            }
        }
    }
}
