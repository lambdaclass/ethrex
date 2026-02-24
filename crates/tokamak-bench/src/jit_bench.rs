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
/// After this call, `JIT_STATE.cache.get(&(code.hash, fork))` returns `Some`.
#[cfg(feature = "jit-bench")]
fn compile_for_jit(bytecode: &Bytes, fork: Fork) -> Code {
    let code = Code::from_bytecode(bytecode.clone());

    let backend = JIT_STATE
        .backend()
        .expect("JIT backend not registered — call init_jit_backend() first");

    backend
        .compile(&code, fork, &JIT_STATE.cache)
        .expect("JIT compilation failed");

    // Verify cache entry exists
    assert!(
        JIT_STATE.cache.get(&(code.hash, fork)).is_some(),
        "compiled code not found in cache after compilation"
    );

    code
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
///
/// **Interpreter baseline**: Runs the scenario without JIT backend registered (or with
/// counter below threshold) using `runner::run_scenario()`.
///
/// **JIT execution**: Pre-compiles bytecode, primes the counter, and runs the VM
/// so that JIT dispatch fires on every execution.
#[cfg(feature = "jit-bench")]
#[expect(clippy::as_conversions, reason = "ns-to-ms conversion for display")]
pub fn run_jit_scenario(
    name: &str,
    bytecode_hex: &str,
    runs: u64,
    iterations: u64,
) -> JitBenchResult {
    let bytecode = Bytes::from(hex::decode(bytecode_hex).expect("Invalid hex bytecode"));
    let calldata = runner::generate_calldata(iterations);
    let fork = Fork::Cancun;

    // ── Interpreter baseline ────────────────────────────────────────────
    // Use run_scenario() which creates fresh VMs each run.
    // JIT_STATE exists but the bytecode hash counter starts from wherever
    // it was. Since we register the backend AFTER this measurement, the
    // JIT dispatch will fire but execute_jit returns None (no compiled code
    // in cache yet for this fresh bytecode). So this is a pure interpreter run.
    //
    // Actually, to be safe, measure interpreter BEFORE compiling into cache.
    let interp_result = runner::run_scenario(name, bytecode_hex, runs, iterations);
    let interpreter_ns = interp_result.total_duration_ns;

    // ── JIT execution ───────────────────────────────────────────────────
    // Ensure backend is registered
    init_jit_backend();

    // Compile bytecode into cache
    let code = compile_for_jit(&bytecode, fork);

    // Prime counter so JIT dispatch fires
    prime_counter_for_jit(&code);

    // Measure JIT execution
    let start = Instant::now();
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
    let jit_duration = start.elapsed();
    let jit_ns = jit_duration.as_nanos();

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

    JitBenchResult {
        scenario: name.to_string(),
        interpreter_ns,
        jit_ns: Some(jit_ns),
        speedup,
        runs,
    }
}

/// Run the full JIT benchmark suite.
///
/// Iterates all scenarios, measuring both interpreter and JIT execution times.
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
        let result = run_jit_scenario(scenario.name, &bytecode, runs, scenario.iterations);
        results.push(result);
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
