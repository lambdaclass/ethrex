//! Phase 3: Offline proving tests using fixture stdin.bin files.
//!
//! These tests re-prove batches from serialized SP1Stdin fixtures,
//! verifying that proving is deterministic and public_values match.
//!
//! Requires:
//! - SP1 toolchain installed
//! - Fixture files collected via `ETHREX_DUMP_FIXTURES` (see fixture-data-collection.md)
//! - The matching ELF must be compiled (e.g. `GUEST_PROGRAMS=evm-l2,zk-dex`)
//!
//! ```sh
//! GUEST_PROGRAMS=evm-l2,zk-dex cargo test -p ethrex-prover --features sp1 --release -- --ignored offline_prove
//! ```

#[cfg(feature = "sp1")]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod sp1_offline_proving {
    use ethrex_guest_program::programs::{EvmL2GuestProgram, TokammonGuestProgram, ZkDexGuestProgram};
    use ethrex_guest_program::traits::{GuestProgram, backends};
    use sp1_sdk::{CpuProver, Prover, SP1ProofMode, SP1ProofWithPublicValues, SP1Stdin};
    use std::path::{Path, PathBuf};

    fn fixtures_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../guest-program/tests/fixtures")
            .canonicalize()
            .expect("fixtures directory should exist")
    }

    /// Get the SP1 ELF for a given app name, or None if not compiled.
    fn get_elf_for_app(app: &str) -> Option<&'static [u8]> {
        match app {
            "evm-l2" => EvmL2GuestProgram.elf(backends::SP1),
            "zk-dex" => ZkDexGuestProgram.elf(backends::SP1),
            "tokamon" => TokammonGuestProgram.elf(backends::SP1),
            _ => None,
        }
    }

    /// Discover all stdin.bin fixtures across all apps.
    fn discover_stdin_fixtures() -> Vec<(String, String, PathBuf)> {
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
                                results.push((label, app_name.clone(), stdin_path));
                            }
                        }
                    }
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

        let client = CpuProver::new();
        let mut proved = 0;
        let mut skipped = 0;

        // Group fixtures by app to reuse setup
        let mut current_app = String::new();
        let mut current_setup: Option<(sp1_sdk::SP1ProvingKey, sp1_sdk::SP1VerifyingKey)> = None;

        for (label, app, stdin_path) in &fixtures {
            // Get the correct ELF for this app
            let elf = match get_elf_for_app(app) {
                Some(elf) if !elf.is_empty() => elf,
                _ => {
                    eprintln!(
                        "[{label}] SKIP — no SP1 ELF for app '{app}'. Build with GUEST_PROGRAMS={app}"
                    );
                    skipped += 1;
                    continue;
                }
            };

            // Setup keys (reuse if same app)
            if *app != current_app {
                eprintln!("[{label}] Setting up SP1 keys for app '{app}'...");
                let (pk, vk) = client.setup(elf);
                current_setup = Some((pk, vk));
                current_app = app.clone();
            }
            let (pk, vk) = current_setup.as_ref().unwrap();

            eprintln!("[{label}] proving from {}", stdin_path.display());
            let stdin_bytes = std::fs::read(stdin_path).expect("read stdin.bin");

            let mut stdin = SP1Stdin::new();
            stdin.write_slice(&stdin_bytes);

            let proof: SP1ProofWithPublicValues =
                <CpuProver as Prover<_>>::prove(&client, pk, &stdin, SP1ProofMode::Compressed)
                    .unwrap_or_else(|e| panic!("[{label}] proving failed: {e}"));

            // Verify the proof
            client
                .verify(&proof, vk)
                .unwrap_or_else(|e| panic!("[{label}] verification failed: {e}"));

            // If a corresponding prover.json exists, compare public_values
            let batch_dir = stdin_path.parent().unwrap();
            let prover_json = batch_dir.join("prover.json");
            if prover_json.exists() {
                let pj: serde_json::Value =
                    serde_json::from_str(&std::fs::read_to_string(&prover_json).unwrap()).unwrap();
                if let Some(expected_hex) =
                    pj.get("encoded_public_values").and_then(|v| v.as_str())
                {
                    let expected =
                        hex::decode(expected_hex.strip_prefix("0x").unwrap_or(expected_hex))
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
            proved += 1;
        }

        eprintln!("Phase 3 summary: {proved} proved, {skipped} skipped");
        assert!(
            proved > 0 || skipped > 0,
            "No fixtures were processed at all"
        );
    }
}
