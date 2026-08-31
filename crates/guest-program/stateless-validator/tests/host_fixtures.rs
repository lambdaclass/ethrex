//! Host-side fixture tests for the stateless-validator guest logic.
//!
//! Runs `run_stateless_validation` over EEST `blockchain_test` fixtures that
//! embed `statelessInputBytes`/`statelessOutputBytes` and asserts the produced
//! output is byte-identical. This is both the PR gate for guest breakage and the
//! equivalence harness for comparing guest integrations.
//!
//! Fixtures come from `ETHREX_STATELESS_FIXTURES` (see `tests/common/mod.rs`);
//! when it is unset the test skips, so plain `cargo test` stays green without a
//! download. Point it at `vectors_zkevm/eest` after a `make -C
//! tooling/ef_tests/blockchain zkevm-vectors` run — not its parent, which also
//! holds the downloaded tarball that is not a fixture.
//!
//! The current baseline is recorded in `docs/eip-8025.md`. Divergences are left
//! failing rather than pinned to an expected-failure count, so no regression can
//! hide behind a list.
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
