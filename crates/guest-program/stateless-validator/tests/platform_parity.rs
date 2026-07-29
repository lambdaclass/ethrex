//! Byte-parity between the ere-platform entrypoint path (mirror spike) and
//! the plain runner over the same fixtures. `TestPlatform` uses the trait's
//! no-op defaults for print/cycle scopes, so only the shared validation path
//! is exercised — the zkVM IO defaults are never invoked.
#![cfg(all(feature = "host", feature = "ere"))]

mod common;

use std::{path::Path, sync::Arc};

use ethrex_guest_program::crypto::NativeCrypto;
use ethrex_stateless_validator::platform::{Platform, run_stateless_guest};
use ethrex_stateless_validator::run_stateless_validation;

struct TestPlatform;

impl Platform for TestPlatform {}

#[test]
fn platform_path_matches_plain_runner() {
    let Some(dir) = std::env::var_os(common::FIXTURES_DIR_ENV) else {
        eprintln!(
            "skipping platform_path_matches_plain_runner: set {} to a directory of \
             tests-zkevm blockchain_test fixtures",
            common::FIXTURES_DIR_ENV
        );
        return;
    };
    let fixtures = common::load_fixtures(Path::new(&dir));
    assert!(!fixtures.is_empty(), "no fixtures with stateless bytes found");

    let crypto = Arc::new(NativeCrypto);
    for fixture in &fixtures {
        let platform_output = run_stateless_guest::<TestPlatform>(&fixture.stateless_input_bytes);
        let plain_output =
            run_stateless_validation(&fixture.stateless_input_bytes, crypto.clone());
        assert_eq!(
            platform_output, plain_output,
            "platform and plain runner diverged on {}",
            fixture.name
        );
        assert_eq!(
            platform_output, fixture.stateless_output_bytes,
            "platform runner output differs from expected statelessOutputBytes on {}",
            fixture.name
        );
    }
    println!("{} fixtures matched across both paths", fixtures.len());
}
