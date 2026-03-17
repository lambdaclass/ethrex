//! Phase 5b: Groth16 proof generation + fixture export for Foundry tests.
//!
//! Two modes:
//! 1. **Mock** (default): Execute program to get public_values, create mock
//!    Groth16 proof. Produces fixtures usable with SP1MockVerifier in Foundry.
//!    Fast (~seconds).
//!
//! 2. **Real** (`#[ignore]`): Actually prove with Groth16 mode on CPU.
//!    Set `SP1_DEV=true` for smaller circuits. Very slow (~minutes to hours).
//!
//! Both modes save fixtures to `tests/fixtures/<app>/batch_<N>/`:
//! - `groth16_proof_bytes.bin`  — on-chain proof bytes (`proof.bytes()`)
//! - `groth16_public_values.bin` — raw public values
//! - `groth16_vk_hash.txt` — program verifying key hash (bytes32)
//!
//! ```sh
//! # Mock fixtures (fast)
//! GUEST_PROGRAMS=zk-dex cargo test -p ethrex-prover --features sp1 --release -- groth16_mock
//!
//! # Real Groth16 (slow, use SP1_DEV=true for dev circuits)
//! SP1_DEV=true GUEST_PROGRAMS=zk-dex cargo test -p ethrex-prover --features sp1 --release -- --ignored groth16_real
//! ```

#[cfg(feature = "sp1")]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod sp1_groth16 {
    use ethrex_guest_program::programs::{EvmL2GuestProgram, ZkDexGuestProgram};
    use ethrex_guest_program::traits::{GuestProgram, backends};
    use sp1_sdk::{
        CpuProver, HashableKey, Prover, SP1ProofMode, SP1ProofWithPublicValues, SP1Stdin,
        SP1_CIRCUIT_VERSION,
    };
    use std::path::{Path, PathBuf};

    fn fixtures_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../guest-program/tests/fixtures")
            .canonicalize()
            .expect("fixtures directory should exist")
    }

    fn get_elf_for_app(app: &str) -> Option<&'static [u8]> {
        match app {
            "evm-l2" => EvmL2GuestProgram.elf(backends::SP1),
            "zk-dex" => ZkDexGuestProgram.elf(backends::SP1),
            _ => None,
        }
    }

    /// Discover all stdin.bin fixtures.
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

    /// Save Groth16 fixture files for Foundry tests.
    fn save_groth16_fixtures(
        batch_dir: &Path,
        proof: &SP1ProofWithPublicValues,
        vk_hash: &str,
    ) {
        let proof_bytes = proof.bytes();
        let public_values = proof.public_values.as_slice();

        std::fs::write(batch_dir.join("groth16_proof_bytes.bin"), &proof_bytes)
            .expect("write groth16_proof_bytes.bin");
        std::fs::write(
            batch_dir.join("groth16_public_values.bin"),
            public_values,
        )
        .expect("write groth16_public_values.bin");
        std::fs::write(batch_dir.join("groth16_vk_hash.txt"), vk_hash)
            .expect("write groth16_vk_hash.txt");

        // Also copy to Foundry test fixtures directory
        let foundry_fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../l2/contracts/test/fixtures");
        let batch_name = batch_dir.file_name().unwrap().to_str().unwrap();
        let app_name = batch_dir
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap();
        let foundry_dir = foundry_fixtures.join(app_name).join(batch_name);
        std::fs::create_dir_all(&foundry_dir).expect("create foundry fixture dir");

        std::fs::write(foundry_dir.join("groth16_proof_bytes.bin"), &proof_bytes)
            .expect("write foundry proof");
        std::fs::write(
            foundry_dir.join("groth16_public_values.bin"),
            public_values,
        )
        .expect("write foundry public_values");
        std::fs::write(foundry_dir.join("groth16_vk_hash.txt"), vk_hash)
            .expect("write foundry vk_hash");

        eprintln!(
            "  Saved: proof_bytes={} bytes, public_values={} bytes, vk={}",
            proof_bytes.len(),
            public_values.len(),
            vk_hash
        );
        eprintln!("  Foundry fixtures: {}", foundry_dir.display());
    }

    /// Generate mock Groth16 fixtures by executing (not proving) the program.
    /// Mock proofs have empty proof bytes — usable with SP1MockVerifier.
    #[test]
    fn groth16_mock_fixtures() {
        let fixtures = discover_stdin_fixtures();
        if fixtures.is_empty() {
            eprintln!("SKIP: no stdin.bin fixtures found.");
            return;
        }

        let client = CpuProver::new();
        let mut generated = 0;

        let mut current_app = String::new();
        let mut current_keys: Option<(sp1_sdk::SP1ProvingKey, sp1_sdk::SP1VerifyingKey)> = None;

        for (label, app, stdin_path) in &fixtures {
            let elf = match get_elf_for_app(app) {
                Some(elf) if !elf.is_empty() => elf,
                _ => {
                    eprintln!("[{label}] SKIP — no ELF for '{app}'");
                    continue;
                }
            };

            if *app != current_app {
                let (pk, vk) = client.setup(elf);
                current_keys = Some((pk, vk));
                current_app = app.clone();
            }
            let (pk, vk) = current_keys.as_ref().unwrap();

            eprintln!("[{label}] Executing to get public_values...");
            let stdin_bytes = std::fs::read(stdin_path).expect("read stdin.bin");
            let mut stdin = SP1Stdin::new();
            stdin.write_slice(&stdin_bytes);

            let (public_values, _report) = client
                .execute(&pk.elf, &stdin)
                .run()
                .unwrap_or_else(|e| panic!("[{label}] execution failed: {e}"));

            eprintln!(
                "[{label}] Got {} bytes of public_values",
                public_values.as_slice().len()
            );

            // Create mock Groth16 proof
            let mock_proof = SP1ProofWithPublicValues::create_mock_proof(
                pk,
                public_values,
                SP1ProofMode::Groth16,
                SP1_CIRCUIT_VERSION,
            );

            let vk_hash = vk.bytes32();
            let batch_dir = stdin_path.parent().unwrap();
            save_groth16_fixtures(batch_dir, &mock_proof, &vk_hash);
            generated += 1;
        }

        eprintln!("Phase 5b mock: {generated} fixture(s) generated");
        assert!(generated > 0, "No mock fixtures were generated");
    }

    /// Generate real Groth16 proof on CPU.
    /// Set SP1_DEV=true for smaller dev circuits (faster but not production).
    #[test]
    #[ignore = "requires SP1 toolchain + stdin.bin fixtures. Very slow on CPU. Use SP1_DEV=true"]
    fn groth16_real_prove() {
        let fixtures = discover_stdin_fixtures();
        if fixtures.is_empty() {
            eprintln!("SKIP: no stdin.bin fixtures found.");
            return;
        }

        if std::env::var("SP1_DEV").is_err() {
            eprintln!("WARNING: SP1_DEV not set. Real Groth16 on CPU may take hours.");
            eprintln!("Consider: SP1_DEV=true cargo test ...");
        }

        let client = CpuProver::new();
        let mut generated = 0;

        let mut current_app = String::new();
        let mut current_keys: Option<(sp1_sdk::SP1ProvingKey, sp1_sdk::SP1VerifyingKey)> = None;

        for (label, app, stdin_path) in &fixtures {
            let elf = match get_elf_for_app(app) {
                Some(elf) if !elf.is_empty() => elf,
                _ => {
                    eprintln!("[{label}] SKIP — no ELF for '{app}'");
                    continue;
                }
            };

            if *app != current_app {
                let (pk, vk) = client.setup(elf);
                current_keys = Some((pk, vk));
                current_app = app.clone();
            }
            let (pk, vk) = current_keys.as_ref().unwrap();

            eprintln!("[{label}] Proving with Groth16 mode (this will be slow)...");
            let stdin_bytes = std::fs::read(stdin_path).expect("read stdin.bin");
            let mut stdin = SP1Stdin::new();
            stdin.write_slice(&stdin_bytes);

            let start = std::time::Instant::now();
            let proof: SP1ProofWithPublicValues =
                <CpuProver as Prover<_>>::prove(&client, pk, &stdin, SP1ProofMode::Groth16)
                    .unwrap_or_else(|e| panic!("[{label}] Groth16 proving failed: {e}"));
            let elapsed = start.elapsed();

            eprintln!("[{label}] Groth16 proof generated in {elapsed:.1?}");

            // Verify the proof
            client
                .verify(&proof, vk)
                .unwrap_or_else(|e| panic!("[{label}] Groth16 verification failed: {e}"));

            let vk_hash = vk.bytes32();
            let batch_dir = stdin_path.parent().unwrap();
            save_groth16_fixtures(batch_dir, &proof, &vk_hash);
            generated += 1;
        }

        eprintln!("Phase 5b real: {generated} Groth16 proof(s) generated");
        assert!(generated > 0, "No Groth16 proofs were generated");
    }
}
