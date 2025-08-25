use ef_tests_blockchain::test_runner::parse_and_execute;
use ethrex_vm::EvmEngine;
use std::path::Path;

const TEST_FOLDER: &str = "vectors/";

// If neither `sp1` nor `stateless` is enabled: run with whichever engine
// the features imply (LEVM if `levm` is on; otherwise REVM).
#[cfg(not(any(feature = "sp1", feature = "stateless")))]
fn blockchain_runner(path: &Path) -> datatest_stable::Result<()> {
    let engine = if cfg!(feature = "levm") {
        EvmEngine::LEVM
    } else {
        EvmEngine::REVM
    };

    parse_and_execute(path, engine, None, None)
}

// If `sp1` or `stateless` is enabled: always use LEVM with the appropriate backend.
#[cfg(any(feature = "sp1", feature = "stateless"))]
fn blockchain_runner(path: &Path) -> datatest_stable::Result<()> {
    #[cfg(feature = "stateless")]
    let backend = Some(ethrex_prover_lib::backends::Backend::Exec);
    #[cfg(feature = "sp1")]
    let backend = Some(ethrex_prover_lib::backends::Backend::SP1);

    parse_and_execute(path, EvmEngine::LEVM, None, backend)
}

datatest_stable::harness!(blockchain_runner, TEST_FOLDER, r".*");

#[cfg(all(feature = "sp1", feature = "stateless"))]
compile_error!("`sp1` and `stateless` cannot be enabled together.");
