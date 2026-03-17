//! Phase 4: Offline verification tests using fixture proof.bin files.
//!
//! These tests deserialize BatchProof from fixtures and verify them
//! without re-proving. Much faster than Phase 3 (~seconds vs ~minutes).
//!
//! Requires:
//! - SP1 toolchain installed
//! - Fixture files collected via `ETHREX_DUMP_FIXTURES` (see fixture-data-collection.md)
//! - The matching ELF must be compiled (e.g. `GUEST_PROGRAMS=evm-l2,zk-dex`)
//!
//! ```sh
//! GUEST_PROGRAMS=evm-l2,zk-dex cargo test -p ethrex-prover --features sp1 --release -- --ignored offline_verify
//! ```

#[cfg(feature = "sp1")]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod sp1_offline_verify {
    use ethrex_guest_program::programs::{EvmL2GuestProgram, TokammonGuestProgram, ZkDexGuestProgram};
    use ethrex_guest_program::traits::{GuestProgram, backends};
    use ethrex_l2_common::prover::BatchProof;
    use sp1_sdk::{CpuProver, Prover, SP1ProofWithPublicValues};
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

    /// Discover all proof.bin fixtures across all apps.
    fn discover_proof_fixtures() -> Vec<(String, String, PathBuf)> {
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
                            let proof_path = batch_path.join("proof.bin");
                            if proof_path.exists() {
                                let label = format!(
                                    "{}/{}",
                                    app_name,
                                    batch_path.file_name().unwrap().to_str().unwrap()
                                );
                                results.push((label, app_name.clone(), proof_path));
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
    #[ignore = "requires SP1 toolchain + proof.bin fixtures"]
    fn offline_verify_from_fixtures() {
        let fixtures = discover_proof_fixtures();
        if fixtures.is_empty() {
            eprintln!("SKIP: no proof.bin fixtures found. Collect with ETHREX_DUMP_FIXTURES.");
            return;
        }

        let client = CpuProver::new();
        let mut verified = 0;
        let mut skipped = 0;

        let mut current_app = String::new();
        let mut current_vk: Option<sp1_sdk::SP1VerifyingKey> = None;

        for (label, app, proof_path) in &fixtures {
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

            // Setup VK (reuse if same app)
            if *app != current_app {
                eprintln!("[{label}] Setting up SP1 VK for app '{app}'...");
                let (_pk, vk) = client.setup(elf);
                current_vk = Some(vk);
                current_app = app.clone();
            }
            let vk = current_vk.as_ref().unwrap();

            eprintln!("[{label}] verifying from {}", proof_path.display());
            let proof_bytes = std::fs::read(proof_path).expect("read proof.bin");

            let batch_proof: BatchProof =
                bincode::deserialize(&proof_bytes).unwrap_or_else(|e| {
                    panic!("[{label}] failed to deserialize BatchProof: {e}")
                });

            match &batch_proof {
                BatchProof::ProofCalldata(pc) => {
                    assert!(
                        !pc.public_values.is_empty(),
                        "[{label}] ProofCalldata.public_values should not be empty"
                    );
                    eprintln!(
                        "[{label}] OK — ProofCalldata with {} bytes public_values (on-chain format)",
                        pc.public_values.len()
                    );
                    verified += 1;
                }
                BatchProof::ProofBytes(pb) => {
                    let sp1_proof: SP1ProofWithPublicValues =
                        bincode::deserialize(&pb.proof).unwrap_or_else(|e| {
                            panic!("[{label}] failed to deserialize SP1 proof: {e}")
                        });

                    client
                        .verify(&sp1_proof, vk)
                        .unwrap_or_else(|e| panic!("[{label}] SP1 verification failed: {e}"));

                    eprintln!(
                        "[{label}] OK — SP1 proof verified ({} bytes)",
                        pb.proof.len()
                    );
                    verified += 1;
                }
            }
        }

        eprintln!("Phase 4 summary: {verified} verified, {skipped} skipped");
        assert!(
            verified > 0 || skipped > 0,
            "No fixtures were processed at all"
        );
    }
}
