//! Phase 4: Offline verification tests using fixture proof.bin files.
//!
//! These tests deserialize BatchProof from fixtures and verify them
//! without re-proving. Much faster than Phase 3 (~seconds vs ~minutes).
//!
//! Requires:
//! - SP1 toolchain installed
//! - Fixture files collected via `ETHREX_DUMP_FIXTURES` (see fixture-data-collection.md)
//!
//! ```sh
//! cargo test -p ethrex-prover --features sp1 -- --ignored offline_verify
//! ```

#[cfg(feature = "sp1")]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod sp1_offline_verify {
    use ethrex_l2_common::prover::BatchProof;
    use sp1_sdk::{CpuProver, Prover, SP1ProofWithPublicValues};
    use std::path::{Path, PathBuf};

    fn fixtures_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../guest-program/tests/fixtures")
            .canonicalize()
            .expect("fixtures directory should exist")
    }

    /// Discover all proof.bin fixtures across all apps.
    fn discover_proof_fixtures() -> Vec<(String, PathBuf)> {
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
                // Look for batch directories containing proof.bin
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
                                results.push((label, proof_path));
                            }
                        }
                    }
                }
                // Also check app-level proof.bin (flat layout)
                let flat_proof = app_path.join("proof.bin");
                if flat_proof.exists() {
                    results.push((app_name, flat_proof));
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

        let elf = ethrex_guest_program::ZKVM_SP1_PROGRAM_ELF;
        let client = CpuProver::new();
        let (_pk, vk) = client.setup(elf);

        for (label, proof_path) in &fixtures {
            eprintln!("[{label}] verifying from {}", proof_path.display());
            let proof_bytes = std::fs::read(proof_path).expect("read proof.bin");

            let batch_proof: BatchProof =
                bincode::deserialize(&proof_bytes).unwrap_or_else(|e| {
                    panic!("[{label}] failed to deserialize BatchProof: {e}")
                });

            // Extract SP1ProofWithPublicValues based on BatchProof variant
            match &batch_proof {
                BatchProof::ProofCalldata(pc) => {
                    // ProofCalldata stores Groth16/PLONK calldata, not raw SP1 proof.
                    // We can verify the public_values field is present.
                    assert!(
                        !pc.public_values.is_empty(),
                        "[{label}] ProofCalldata.public_values should not be empty"
                    );
                    eprintln!(
                        "[{label}] OK — ProofCalldata with {} bytes public_values (on-chain format, no SP1 verify)",
                        pc.public_values.len()
                    );
                }
                BatchProof::ProofBytes(pb) => {
                    // ProofBytes stores the raw SP1ProofWithPublicValues
                    let sp1_proof: SP1ProofWithPublicValues =
                        bincode::deserialize(&pb.proof).unwrap_or_else(|e| {
                            panic!("[{label}] failed to deserialize SP1 proof: {e}")
                        });

                    client
                        .verify(&sp1_proof, &vk)
                        .unwrap_or_else(|e| panic!("[{label}] SP1 verification failed: {e}"));

                    eprintln!(
                        "[{label}] OK — SP1 proof verified ({} bytes)",
                        pb.proof.len()
                    );
                }
            }
        }
    }
}
