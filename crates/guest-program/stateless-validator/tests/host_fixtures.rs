//! Host-side fixture tests for the stateless-validator guest logic.
//!
//! Runs `run_stateless_validation` over EEST `blockchain_test` fixtures that
//! embed `statelessInputBytes`/`statelessOutputBytes` (the `tests-zkevm`
//! releases of `ethereum/execution-specs`) and asserts the produced output is
//! byte-identical to the expected output. This is both the PR gate for guest
//! breakage and the equivalence harness for comparing guest integrations.
//!
//! See `tests/common/mod.rs` for the fixture source and the
//! `ETHREX_STATELESS_FIXTURES` contract; when the variable is unset the test
//! is skipped so plain `cargo test` runs stay green without a download. Point it
//! at the `blockchain_tests/` subtree of a `make -C tooling/ef_tests/blockchain
//! stateless-vector` run — not its parent, which also holds a `.meta/index.json`
//! that is not a fixture.
//!
//! MEASURED BASELINE, 2026-08-05, against the 769-block generated vector set:
//! 8 exact matches, 755 differing **only** in bytes 0..32
//! (`new_payload_request_root`), 6 differing more widely.
//!
//! The 755 are all explained by one upstream defect: `libssz-merkle 0.2.2`
//! reverses the progressive-merkleization subtree children, so every
//! `hash_tree_root` over a `ProgressiveContainer` is wrong. On those blocks
//! `successful_validation`, `chain_id` and `schema_id` are already byte-identical
//! to the reference — including on true-success cases — so decode, witness
//! rebuild, public-key validation, block reconstruction and execution all agree
//! with execution-specs today. See `test/tests/common/progressive_ssz_tests.rs`
//! for the proof and the one-line fix. Expect ~763/769 once it lands; the
//! remaining 6 need separate investigation.
#![cfg(feature = "host")]

mod common;

use std::{path::Path, sync::Arc};

use ethrex_guest_program::crypto::NativeCrypto;
use ethrex_stateless_validator::run_stateless_validation;

#[test]
fn eest_fixture_equivalence() {
    let Some(dir) = std::env::var_os(common::FIXTURES_DIR_ENV) else {
        eprintln!(
            "skipping eest_fixture_equivalence: set {} to a directory of \
             tests-zkevm blockchain_test fixtures",
            common::FIXTURES_DIR_ENV
        );
        return;
    };
    let fixtures = common::load_fixtures(Path::new(&dir));
    assert!(
        !fixtures.is_empty(),
        "no fixtures with stateless bytes found under {}",
        common::FIXTURES_DIR_ENV
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
