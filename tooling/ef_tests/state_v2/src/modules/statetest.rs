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
use ethrex_common::tracing::Eip3155Step;
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
#[group(required = true, multiple = false)]
pub struct StatetestOptions {
    /// Emit full EIP-3155 JSONL trace + stateRoot line for the given fixture.
    #[arg(long, value_name = "PATH", group = "mode")]
    pub json: Option<PathBuf>,
    /// Emit only the stateRoot line for the given fixture (no per-opcode trace).
    #[arg(long, value_name = "PATH", group = "mode")]
    pub json_outcome: Option<PathBuf>,
}

impl StatetestOptions {
    /// Returns `(path, emit_trace)`. The clap `ArgGroup` guarantees exactly one is set.
    fn fixture_path(&self) -> (&PathBuf, bool) {
        match (&self.json, &self.json_outcome) {
            (Some(p), None) => (p, true),
            (None, Some(p)) => (p, false),
            _ => unreachable!("clap ArgGroup enforces exactly one of --json / --json-outcome"),
        }
    }
}

pub async fn run(opts: StatetestOptions) -> Result<ExitCode, RunnerError> {
    let (path, emit_trace) = opts.fixture_path();
    let tests = parse_file(path, false)?;

    // `Tests::from` filters out forks not in `DEFAULT_FORKS` (types.rs). A fixture
    // whose `post` map contains only unsupported forks would therefore parse fine
    // but produce zero `test_cases`, and we'd silently exit 0 with no `stateRoot`
    // emitted — a false-green that goevmlab can't detect. Surface it as an error.
    if tests.iter().all(|t| t.test_cases.is_empty()) {
        return Err(RunnerError::Custom(format!(
            "no runnable test cases in {}: none of the post-state forks are in the runnable allow-list",
            path.display(),
        )));
    }

    let mut any_mismatch = false;
    for test in &tests {
        for test_case in &test.test_cases {
            any_mismatch |= run_case(test, test_case, emit_trace).await?;
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
        load_initial_state(test, &test_case.fork, true).await;
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
        // Wrap each step in `Eip3155Step` so the serializer emits the strict
        // EIP-3155 wire shape (numeric `op` + separate `opName`, hex
        // `gas`/`gasCost`/`refund`, `stack: []` when disabled) — what goevmlab's
        // opLog unmarshaler expects, not the geth-RPC structLogger shape.
        for step in &vm.opcode_tracer.logs {
            let line = serde_json::to_string(&Eip3155Step(step))
                .map_err(|e| RunnerError::Custom(format!("failed to serialize trace step: {e}")))?;
            eprintln!("{line}");
        }
    }

    let account_updates = backends::levm::LEVM::get_state_transitions(&mut vm.db.clone())
        .map_err(|e| RunnerError::FailedToGetAccountsUpdates(e.to_string()))?;
    let computed_root = post_state_root(&account_updates, initial_block_hash, storage);

    eprintln!("{}", stateroot_line(&computed_root));

    Ok(computed_root != test_case.post.hash)
}

/// Formats a state root as the literal line goevmlab's adapter scans for in
/// each client's stderr stream: the substring `"stateRoot":"0x<64 lowercase hex>"`.
///
/// Extracted so the regression test below can pin the exact wire format without
/// reaching into `eprintln!`. Surrounding JSON shape is flexible per the goevmlab
/// spec — only the literal substring matters — but emitting it as a valid one-key
/// JSON object keeps the line parseable too.
fn stateroot_line(root: &ethrex_common::H256) -> String {
    format!("{{\"stateRoot\":\"0x{root:x}\"}}")
}

#[cfg(test)]
mod tests {
    //! Regression tests for the wire-format contract that goevmlab consumes.
    //!
    //! Two invariants matter end-to-end:
    //!   1. Each opcode trace line is JSON parseable by goevmlab's `opLog`
    //!      unmarshaler (`evms/gen_oplog.go`). That means `op` is a number
    //!      (cast to `vm.OpCode`), `gas`/`gasCost` are decimal-or-hex numbers,
    //!      `stack` is a non-null array. We rely on `Eip3155Step`'s serializer
    //!      to emit this shape — see `crates/common/tracing.rs`.
    //!   2. The final stateRoot line contains the exact literal substring
    //!      `"stateRoot":"0x<64 hex chars>"` so goevmlab can scan for it by
    //!      raw byte search (see [revm.go](https://github.com/holiman/goevmlab/blob/master/evms/revm.go)).

    use super::stateroot_line;
    use ethrex_common::{H256, U256, tracing::Eip3155Step, tracing::OpcodeStep};
    use serde_json::Value;

    /// Builds a minimal `OpcodeStep` for `PUSH1` (opcode 0x60) with one stack entry.
    fn sample_step() -> OpcodeStep {
        OpcodeStep {
            pc: 0,
            op: 0x60,
            gas: 21_000,
            gas_cost: 3,
            mem_size: 0,
            depth: 1,
            return_data: bytes::Bytes::new(),
            refund: 0,
            stack: Some(vec![U256::from(0x42)]),
            memory: None,
            storage: None,
            error: None,
        }
    }

    #[test]
    fn eip3155_step_matches_goevmlab_oplog_shape() {
        let line = serde_json::to_string(&Eip3155Step(&sample_step())).expect("serialize");
        let v: Value = serde_json::from_str(&line).expect("valid JSON");

        // EIP-3155 spec types, mirroring the fields goevmlab's gen_oplog.go
        // expects to unmarshal into uint64/vm.OpCode/uint256.Int/etc.
        assert!(v["pc"].is_number(), "pc must be a JSON number");
        assert!(
            v["op"].is_number(),
            "op must be a NUMERIC opcode byte (goevmlab casts to vm.OpCode); got: {}",
            v["op"]
        );
        assert_eq!(v["op"].as_u64(), Some(0x60));
        assert_eq!(v["opName"].as_str(), Some("PUSH1"));

        let gas = v["gas"].as_str().expect("gas must be a hex string");
        assert!(
            gas.starts_with("0x"),
            "gas must be `\"0x...\"` form per EIP-3155 Hex-Number; got: {gas}"
        );
        let gas_cost = v["gasCost"].as_str().expect("gasCost must be a hex string");
        assert!(gas_cost.starts_with("0x"));

        // EIP-3155: `stack` MUST be `[]`, never null.
        assert!(v["stack"].is_array(), "stack must be an array, never null");
        assert_eq!(v["stack"][0].as_str(), Some("0x42"));
    }

    #[test]
    fn eip3155_step_stack_disabled_renders_as_empty_array() {
        let mut step = sample_step();
        step.stack = None;
        let line = serde_json::to_string(&Eip3155Step(&step)).expect("serialize");
        let v: Value = serde_json::from_str(&line).expect("valid JSON");
        assert_eq!(
            v["stack"],
            Value::Array(vec![]),
            "EIP-3155: stack must be `[]` when disabled, not null",
        );
    }

    #[test]
    fn stateroot_line_pins_literal_goevmlab_scan_pattern() {
        let root = H256::repeat_byte(0xab);
        let line = stateroot_line(&root);

        // The literal substring `"stateRoot":"0x<64 hex>"` is what goevmlab byte-
        // scans for; surrounding JSON shape is flexible. Pin both halves.
        let expected_hex = format!("0x{}", "ab".repeat(32));
        assert_eq!(expected_hex.len(), 66, "64 hex chars + 0x prefix");
        assert!(
            line.contains(&format!("\"stateRoot\":\"{expected_hex}\"")),
            "missing goevmlab scan pattern; line={line}"
        );

        // Sanity: H256's LowerHex zero-pads to 64 chars even for low-value roots.
        let small = H256::from_low_u64_be(1);
        let line_small = stateroot_line(&small);
        assert!(line_small.contains(&format!("\"0x{:0>64}\"", "1")));
    }
}
