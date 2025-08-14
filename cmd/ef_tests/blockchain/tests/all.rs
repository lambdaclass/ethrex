use ef_tests_blockchain::test_runner::parse_and_execute;
use ethrex_vm::EvmEngine;
use std::path::Path;

const TEST_FOLDER: &str = "vectors/";

fn parse_and_execute_runner(path: &Path) -> datatest_stable::Result<()> {
    let engine = if cfg!(feature = "revm") {
        EvmEngine::REVM
    } else {
        EvmEngine::LEVM
    };

    parse_and_execute(path, engine, None, false)
}

#[cfg(feature = "levm")]
fn parse_and_execute_stateless_runner(path: &Path) -> datatest_stable::Result<()> {
    parse_and_execute(path, EvmEngine::LEVM, None, true)
}
#[cfg(feature = "levm")]
datatest_stable::harness!(
    parse_and_execute_runner,
    TEST_FOLDER,
    r".*",
    parse_and_execute_stateless_runner,
    TEST_FOLDER,
    r".*"
);
#[cfg(not(feature = "levm"))]
datatest_stable::harness!(parse_and_execute_runner, TEST_FOLDER, r".*",);
