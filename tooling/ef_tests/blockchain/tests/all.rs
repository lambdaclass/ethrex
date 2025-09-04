use ef_tests_blockchain::test_runner::parse_and_execute;
use ethrex_vm::EvmEngine;
use std::path::Path;

const TEST_FOLDER: &str = "vectors/";

const SKIPPED_TESTS: &[&str] = &["system_contract_deployment"];

// If neither `sp1` nor `stateless` is enabled: run with whichever engine
// the features imply (LEVM if `levm` is on; otherwise REVM).
#[cfg(not(any(feature = "sp1", feature = "stateless")))]
fn blockchain_runner(path: &Path) -> datatest_stable::Result<()> {
    let engine = if cfg!(feature = "revm") {
        EvmEngine::REVM
    } else {
        EvmEngine::LEVM
    };

    parse_and_execute(path, engine, Some(SKIPPED_TESTS), None)
}

// If `sp1` or `stateless` is enabled: always use LEVM with the appropriate backend.
#[cfg(any(feature = "sp1", feature = "stateless"))]
fn blockchain_runner(path: &Path) -> datatest_stable::Result<()> {
    #[cfg(feature = "stateless")]
    let backend = Some(ethrex_prover_lib::backends::Backend::Exec);
    #[cfg(feature = "sp1")]
    let backend = Some(ethrex_prover_lib::backends::Backend::SP1);

    parse_and_execute(path, EvmEngine::LEVM, Some(SKIPPED_TESTS), backend)
}

#[cfg(feature = "revm")]
datatest_stable::harness!(blockchain_runner, TEST_FOLDER, r".*");

#[cfg(any(
    all(feature = "sp1", feature = "stateless"),
    all(feature = "sp1", feature = "revm"),
    all(feature = "stateless", feature = "revm"),
))]
compile_error!("Only one of `sp1`, `stateless`, or `revm` can be enabled at a time.");
