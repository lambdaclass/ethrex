//! JIT compilation benchmarks.
//!
//! Compares execution time between the LEVM interpreter and JIT-compiled
//! code (when `jit-bench` feature is enabled with `revmc-backend`).
//!
//! The interpreter baseline uses `runner::run_scenario()` directly.
//! The JIT path pre-compiles bytecode via the revmc backend, then
//! measures execution with JIT dispatch active.

pub use crate::types::JitBenchSuite;

#[cfg(feature = "jit-bench")]
use crate::types::JitBenchResult;

// ── Feature-gated JIT benchmark implementation ──────────────────────────────

#[cfg(feature = "jit-bench")]
use std::hint::black_box;
#[cfg(feature = "jit-bench")]
use std::sync::OnceLock;
#[cfg(feature = "jit-bench")]
use std::time::Instant;

#[cfg(feature = "jit-bench")]
use bytes::Bytes;
#[cfg(feature = "jit-bench")]
use ethrex_common::types::{Code, Fork};
#[cfg(feature = "jit-bench")]
use ethrex_levm::vm::JIT_STATE;

#[cfg(feature = "jit-bench")]
use crate::runner;

/// One-time JIT backend registration.
#[cfg(feature = "jit-bench")]
static JIT_INITIALIZED: OnceLock<()> = OnceLock::new();

/// Initialize the JIT backend (idempotent).
///
/// Registers the revmc/LLVM backend with LEVM's global `JIT_STATE`
/// and starts the background compiler thread.
#[cfg(feature = "jit-bench")]
pub fn init_jit_backend() {
    JIT_INITIALIZED.get_or_init(|| {
        tokamak_jit::register_jit_backend();
    });
}

/// Pre-compile bytecode into the JIT cache for a given fork.
///
/// Uses the registered backend to synchronously compile the bytecode.
/// Returns `Err` if compilation fails (e.g. bytecode too large for revmc).
#[cfg(feature = "jit-bench")]
fn compile_for_jit(bytecode: &Bytes, fork: Fork) -> Result<Code, String> {
    let code = Code::from_bytecode(bytecode.clone());

    let backend = JIT_STATE
        .backend()
        .expect("JIT backend not registered — call init_jit_backend() first");

    backend.compile(&code, fork, &JIT_STATE.cache)?;

    // Verify cache entry exists
    if JIT_STATE.cache.get(&(code.hash, fork)).is_none() {
        return Err("compiled code not found in cache after compilation".to_string());
    }

    Ok(code)
}

/// Bump the execution counter for a bytecode hash past the compilation threshold.
///
/// This ensures that subsequent VM executions will hit the JIT dispatch path
/// without triggering re-compilation.
#[cfg(feature = "jit-bench")]
fn prime_counter_for_jit(code: &Code) {
    let threshold = JIT_STATE.config.compilation_threshold;
    let current = JIT_STATE.counter.get(&code.hash);
    // Increment past threshold if not already there
    for _ in current..threshold.saturating_add(1) {
        JIT_STATE.counter.increment(&code.hash);
    }
}

/// Run a single JIT benchmark scenario.
///
/// Measures both interpreter and JIT execution times, computing the speedup ratio.
/// Returns `None` if JIT compilation fails for this scenario.
///
/// **Measurement order** (M1 fix — Volkov R21):
/// 1. Interpreter baseline FIRST — uses `init_vm_interpreter_only()` which sets
///    `tracer.active = true`, preventing JIT dispatch from firing.
/// 2. JIT compilation — `init_jit_backend()`, `compile_for_jit()`, `prime_counter_for_jit()`.
/// 3. JIT execution — uses `init_vm()` (JIT-enabled, `tracer.active = false`).
#[cfg(feature = "jit-bench")]
#[expect(clippy::as_conversions, reason = "ns-to-ms conversion for display")]
pub fn run_jit_scenario(
    name: &str,
    bytecode_hex: &str,
    runs: u64,
    iterations: u64,
) -> Option<JitBenchResult> {
    let bytecode = Bytes::from(hex::decode(bytecode_hex).expect("Invalid hex bytecode"));
    let calldata = runner::generate_calldata(iterations);
    let fork = Fork::Cancun;

    // ── Interpreter baseline FIRST ──────────────────────────────────────
    // Measured BEFORE any JIT compilation so the JIT cache is empty and
    // init_vm_interpreter_only() sets tracer.active=true to block JIT dispatch.
    let interp_start = Instant::now();
    for _ in 0..runs {
        let mut db = runner::init_db(bytecode.clone());
        let mut vm = runner::init_vm_interpreter_only(&mut db, calldata.clone());
        let report = black_box(vm.stateless_execute().expect("VM execution failed"));
        assert!(
            report.is_success(),
            "Interpreter execution reverted: {:?}",
            report.result
        );
    }
    let interpreter_ns = interp_start.elapsed().as_nanos();

    // ── JIT compilation ─────────────────────────────────────────────────
    init_jit_backend();

    let code = match compile_for_jit(&bytecode, fork) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("  {name}: JIT compilation failed — {e}");
            return None;
        }
    };

    // Prime counter so JIT dispatch fires during JIT measurement
    prime_counter_for_jit(&code);

    // ── JIT execution ───────────────────────────────────────────────────
    let jit_start = Instant::now();
    for _ in 0..runs {
        let mut db = runner::init_db(bytecode.clone());
        let mut vm = runner::init_vm(&mut db, calldata.clone());
        let report = black_box(vm.stateless_execute().expect("VM execution failed"));
        assert!(
            report.is_success(),
            "JIT VM execution reverted: {:?}",
            report.result
        );
    }
    let jit_ns = jit_start.elapsed().as_nanos();

    // ── Compute speedup ─────────────────────────────────────────────────
    let speedup = if jit_ns > 0 {
        Some(interpreter_ns as f64 / jit_ns as f64)
    } else {
        None
    };

    eprintln!(
        "  {name}: interp={:.3}ms, jit={:.3}ms, speedup={:.2}x",
        interpreter_ns as f64 / 1_000_000.0,
        jit_ns as f64 / 1_000_000.0,
        speedup.unwrap_or(0.0),
    );

    Some(JitBenchResult {
        scenario: name.to_string(),
        interpreter_ns,
        jit_ns: Some(jit_ns),
        speedup,
        runs,
    })
}

/// Run the full JIT benchmark suite.
///
/// Iterates all scenarios, measuring both interpreter and JIT execution times.
/// Scenarios that fail JIT compilation are skipped with a message.
#[cfg(feature = "jit-bench")]
pub fn run_jit_suite(
    scenarios: &[runner::Scenario],
    runs: u64,
    commit: &str,
) -> JitBenchSuite {
    let mut results = Vec::new();

    for scenario in scenarios {
        let bytecode = match runner::load_contract_bytecode(scenario.name) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("Skipping {}: {e}", scenario.name);
                continue;
            }
        };

        eprintln!(
            "Running JIT benchmark: {} ({} runs)...",
            scenario.name, runs
        );
        if let Some(result) =
            run_jit_scenario(scenario.name, &bytecode, runs, scenario.iterations)
        {
            results.push(result);
        }
    }

    JitBenchSuite {
        timestamp: unix_timestamp_secs(),
        commit: commit.to_string(),
        results,
    }
}

#[cfg(feature = "jit-bench")]
fn unix_timestamp_secs() -> String {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", duration.as_secs())
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::JitBenchResult;

    #[test]
    fn test_jit_bench_result_serialization() {
        let result = JitBenchResult {
            scenario: "Fibonacci".to_string(),
            interpreter_ns: 1_000_000,
            jit_ns: Some(200_000),
            speedup: Some(5.0),
            runs: 100,
        };
        let json = serde_json::to_string(&result).expect("serialize");
        let deserialized: JitBenchResult =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.scenario, "Fibonacci");
        assert_eq!(deserialized.speedup, Some(5.0));
    }

    #[test]
    fn test_jit_bench_result_no_jit() {
        let result = JitBenchResult {
            scenario: "Test".to_string(),
            interpreter_ns: 500_000,
            jit_ns: None,
            speedup: None,
            runs: 10,
        };
        let json = serde_json::to_string(&result).expect("serialize");
        assert!(json.contains("\"jit_ns\":null"));
    }

    #[test]
    fn test_jit_bench_suite_serialization() {
        let suite = JitBenchSuite {
            timestamp: "1234567890".to_string(),
            commit: "abc123".to_string(),
            results: vec![JitBenchResult {
                scenario: "Fibonacci".to_string(),
                interpreter_ns: 1_000_000,
                jit_ns: Some(200_000),
                speedup: Some(5.0),
                runs: 10,
            }],
        };
        let json = serde_json::to_string_pretty(&suite).expect("serialize");
        let deserialized: JitBenchSuite =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.commit, "abc123");
        assert_eq!(deserialized.results.len(), 1);
        assert_eq!(deserialized.results[0].scenario, "Fibonacci");
    }
}
