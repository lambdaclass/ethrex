//! `statetest` subcommand: single-fixture runner for goevmlab differential fuzzing.
//!
//! Takes one EF state-test JSON file and runs every `(fork, post-index)` case through
//! LEVM. For each case, emits EIP-3155 JSONL steps and a final `stateRoot` line to
//! **stderr** (stdout is reserved for crash diagnostics, matching geth/revm convention).
//!
//! Exit status:
//! - `0`: all cases produced the expected post-state root
//! - `1`: at least one case had a post-state root mismatch (tolerated by goevmlab)
//! - other: actual crash (panic, parse error, etc.)

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Args;
use ethrex_crypto::NativeCrypto;
use ethrex_levm::{
    opcode_tracer::{LevmOpcodeTracer, OpcodeTracerConfig},
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use ethrex_vm::backends;

use crate::modules::{
    error::RunnerError,
    parser::parse_file,
    result_check::post_state_root,
    runner::{get_tx_from_test_case, get_vm_env_for_test},
    utils::load_initial_state,
};

#[derive(Args, Debug)]
pub struct StatetestOptions {
    /// Emit full EIP-3155 JSONL trace + stateRoot line for the given fixture.
    #[arg(long, value_name = "PATH", conflicts_with = "json_outcome")]
    pub json: Option<PathBuf>,
    /// Emit only the stateRoot line for the given fixture (no per-opcode trace).
    #[arg(long, value_name = "PATH", conflicts_with = "json")]
    pub json_outcome: Option<PathBuf>,
}

impl StatetestOptions {
    /// Returns `(path, emit_trace)`. Exactly one of `--json` / `--json-outcome` must be set.
    fn fixture_path(&self) -> Result<(&PathBuf, bool), RunnerError> {
        match (&self.json, &self.json_outcome) {
            (Some(p), None) => Ok((p, true)),
            (None, Some(p)) => Ok((p, false)),
            _ => Err(RunnerError::Custom(
                "exactly one of --json or --json-outcome must be provided".to_string(),
            )),
        }
    }
}

pub async fn run(opts: StatetestOptions) -> Result<ExitCode, RunnerError> {
    let (path, emit_trace) = opts.fixture_path()?;
    let tests = parse_file(path, false)?;

    let mut any_mismatch = false;
    for test in &tests {
        for test_case in &test.test_cases {
            let mismatch = run_case(test, test_case, emit_trace).await?;
            if mismatch {
                any_mismatch = true;
            }
        }
    }

    Ok(if any_mismatch {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

/// Runs a single `(fork, post-index)` test case. Emits per-opcode JSONL when
/// `emit_trace` is true, then emits the final `stateRoot` line. Returns `true`
/// when the computed root differs from the fixture's expected root.
async fn run_case(
    test: &crate::modules::types::Test,
    test_case: &crate::modules::types::TestCase,
    emit_trace: bool,
) -> Result<bool, RunnerError> {
    let (mut db, initial_block_hash, storage, _genesis) =
        load_initial_state(test, &test_case.fork).await;
    let env = get_vm_env_for_test(test.env, test_case)?;
    let tx = get_tx_from_test_case(test_case).await?;

    let mut vm = VM::new(
        env,
        &mut db,
        &tx,
        LevmCallTracer::disabled(),
        VMType::L1,
        &NativeCrypto,
    )
    .map_err(RunnerError::VMError)?;

    if emit_trace {
        vm.opcode_tracer = LevmOpcodeTracer::new(OpcodeTracerConfig::default());
    }

    // Execution errors here are not necessarily fatal — a state test can expect
    // a tx to fail. The post-state root check is what determines pass/fail.
    let _ = vm.execute();

    if emit_trace {
        for step in &vm.opcode_tracer.logs {
            let line = serde_json::to_string(step)
                .map_err(|e| RunnerError::Custom(format!("failed to serialize trace step: {e}")))?;
            eprintln!("{line}");
        }
    }

    let account_updates = backends::levm::LEVM::get_state_transitions(&mut vm.db.clone())
        .map_err(|e| RunnerError::FailedToGetAccountsUpdates(e.to_string()))?;
    let computed_root = post_state_root(&account_updates, initial_block_hash, storage);

    eprintln!("{{\"stateRoot\":\"0x{computed_root:x}\"}}");

    Ok(computed_root != test_case.post.hash)
}
