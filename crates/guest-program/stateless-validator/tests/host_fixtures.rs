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
//! MEASURED BASELINE, 2026-08-06, against the 768-block generated vector set,
//! with libssz 0.3.0 (the EIP-7916 progressive child-order fix): **762 exact
//! matches, 6 differing — all of them in `successful_validation` only**.
//!
//! Every root now matches, so decode, witness rebuild, public-key validation,
//! block reconstruction, merkleization and encoding all agree with
//! execution-specs. What is left is six genuine disagreements about whether a
//! block is valid. Under libssz 0.2.2 this was 8 exact / 755 root-only / 6, and
//! the 755 were entirely the reversed progressive subtree children; see
//! `test/tests/common/progressive_ssz_tests.rs`.
//!
//! Five are ethrex being too strict — the spec accepts, we reject:
//!   - `test_witness_7702::test_witness_codes_auth_nonce_mismatch`
//!   - `test_witness_7702::test_witness_codes_redelegation_old_marker_included_new_marker_excluded`
//!   - `test_witness_7702::test_witness_codes_reset_delegation`
//!   - `test_witness_bytecodes_contract_creation::test_witness_codes_failed_create_after_initcode_read`
//!   - `test_witness_validation_state::test_validation_state_extra_unused_trie_node`
//!
//! One is ethrex being too lax, which is the one that matters — the spec
//! rejects, we accept:
//!   - `test_witness_validation_headers::test_validation_headers_non_contiguous_chain` (block5)
//!
//! A guest that accepts a payload the spec rejects can prove an invalid state
//! transition, so the non-contiguous-chain case is a correctness bug rather than
//! a conformance gap. Tracked separately; this test stays red until all six are
//! resolved rather than being pinned to the current count, so no regression can
//! hide behind an expected-failure list.
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
            failures.push(format!(
                "{}\n      {}",
                fixture.name,
                describe_divergence(&output, &fixture.stateless_output_bytes)
            ));
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

/// Name the diverging fields of an `SszStatelessValidationResult`.
///
/// The output is a fixed 43-byte layout: root[0..32], successful_validation[32],
/// chain_id[33..41], schema_id[41..43]. Which field differs says what kind of
/// bug it is — a root-only difference is an encoding or merkleization problem,
/// whereas `successful_validation` is a disagreement about the block itself.
fn describe_divergence(got: &[u8], want: &[u8]) -> String {
    if got.len() != want.len() {
        return format!("length {} != expected {}", got.len(), want.len());
    }
    let mut parts = Vec::new();
    if got[..32] != want[..32] {
        parts.push(format!(
            "root {} != {}",
            hex::encode(&got[..32]),
            hex::encode(&want[..32])
        ));
    }
    if got[32] != want[32] {
        parts.push(format!(
            "successful_validation {} != {}",
            got[32] != 0,
            want[32] != 0
        ));
    }
    if got[33..41] != want[33..41] {
        parts.push(format!(
            "chain_id {} != {}",
            u64::from_le_bytes(got[33..41].try_into().expect("8 bytes")),
            u64::from_le_bytes(want[33..41].try_into().expect("8 bytes"))
        ));
    }
    if got[41..43] != want[41..43] {
        parts.push(format!(
            "schema_id {:#06x} != {:#06x}",
            u16::from_le_bytes(got[41..43].try_into().expect("2 bytes")),
            u16::from_le_bytes(want[41..43].try_into().expect("2 bytes"))
        ));
    }
    parts.join(", ")
}
