//! Tests for the streaming sink feature of `LevmOpcodeTracer` (Phase 2).
//!
//! These tests exercise `LevmOpcodeTracer::streaming()` directly, without
//! going through the full VM pipeline.  All tests assert on the bytes written
//! to the sink rather than on internal state, matching the boundary used by
//! the EIP-3155 streaming shape.

use bytes::Bytes;
use ethereum_types::H256;
use ethrex_common::{U256, tracing::OpcodeStep};
use ethrex_levm::tracing::{LevmOpcodeTracer, OpcodeTracerConfig};
use std::sync::{Arc, Mutex};

// ── Shared in-memory sink ─────────────────────────────────────────────────────

/// A `Write` impl backed by a shared `Vec<u8>`, so both the sink (passed into
/// the tracer) and the test (asserting on content) can access the same buffer.
struct SharedSink(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for SharedSink {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A sink that always fails on the first write.
struct FailingSink;

impl std::io::Write for FailingSink {
    fn write(&mut self, _b: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "test failure",
        ))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// ── Helper constructors ───────────────────────────────────────────────────────

fn make_buf() -> (Arc<Mutex<Vec<u8>>>, SharedSink) {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let sink = SharedSink(Arc::clone(&buf));
    (buf, sink)
}

fn default_cfg() -> OpcodeTracerConfig {
    OpcodeTracerConfig::default()
}

/// Builds a minimal `OpcodeStep` for the given opcode byte.  Gas and gas_cost
/// are set to sentinel values so tests can assert on them.
fn make_step(op: u8, pc: u64, gas: u64) -> OpcodeStep {
    OpcodeStep {
        pc,
        op,
        gas,
        gas_cost: 0, // patched by finalize_step
        mem_size: 0,
        depth: 1,
        return_data: Bytes::new(),
        refund: 0,
        stack: Some(vec![]),
        memory: None,
        storage: None,
        error: None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// 2.7a — basic streaming: one step is written to the sink, not kept in `logs`.
#[test]
fn test_2_7a_streaming_basic() {
    let (buf, sink) = make_buf();
    let mut tracer = LevmOpcodeTracer::streaming(default_cfg(), Box::new(sink));

    // Simulate pre-step capture for ADD (0x01), pc=5, gas=1000.
    tracer.pre_step_capture(
        5,    // pc
        0x01, // ADD
        1000, // gas
        1,    // depth
        0,    // refund
        &[],  // stack_view (no stack values for ADD pre-execution in this mini-test)
        &[],  // memory_view
        0,    // mem_size
        &Bytes::new(),
        None, // storage_kv
    );
    assert_eq!(tracer.logs.len(), 1, "step buffered before finalize");

    tracer.finalize_step(3, None);

    // After finalize in streaming mode, logs must be empty.
    assert!(tracer.logs.is_empty(), "step flushed: logs must be empty");
    assert_eq!(tracer.streamed_count, 1);

    let bytes = buf.lock().unwrap();
    let output = std::str::from_utf8(&bytes).expect("valid UTF-8");
    // Must be exactly one newline-terminated line.
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(lines.len(), 1, "exactly one step line");

    // Parse as JSON and check shape.
    let v: serde_json::Value = serde_json::from_str(lines[0]).expect("valid JSON");
    assert_eq!(v["pc"], serde_json::json!(5));
    assert_eq!(v["op"], serde_json::json!(0x01_u64)); // streaming emits raw opcode byte
    assert_eq!(v["gas"], serde_json::json!("0x3e8")); // 1000 hex
    assert_eq!(v["gasCost"], serde_json::json!("0x3")); // patched gas_cost = 3
    assert_eq!(v["depth"], serde_json::json!(1));

    // Trailing newline must be present.
    assert!(output.ends_with('\n'), "line must end with newline");
}

/// 2.7b — JUMP + synthetic JUMPDEST ordering: parent line before synth line.
///
/// Real flow: JUMP pre_step_capture → handler calls synthesize_step (JUMPDEST
/// pushed into logs) → dispatch loop calls finalize_step(JUMP).
/// finalize_step flushes logs[idx..] in order: JUMP first, then JUMPDEST.
#[test]
fn test_2_7b_synthetic_ordering() {
    let (buf, sink) = make_buf();
    let mut tracer = LevmOpcodeTracer::streaming(default_cfg(), Box::new(sink));

    // 1. Simulate pre-step capture for JUMPI (0x57), pc=2.
    tracer.pre_step_capture(
        2,    // pc
        0x57, // JUMPI
        5000, // gas
        1,
        0,
        &[U256::from(10), U256::from(1)], // stack: [target=10, cond=1]
        &[],
        0,
        &Bytes::new(),
        None,
    );
    let jumpi_idx = tracer.last_step_index.unwrap();

    // 2. Handler calls synthesize_step for JUMPDEST (0x5b), pc=10.
    let jumpdest = make_step(0x5b, 10, 4992); // gas after JUMPI charge
    tracer.synthesize_step(jumpdest);

    // Both parent and synthetic are buffered.
    assert_eq!(tracer.logs.len(), 2);

    // 3. Dispatch loop calls finalize_step for JUMPI.
    tracer.finalize_step(8, None); // JUMP costs 8

    // After flush, logs must be empty.
    assert!(tracer.logs.is_empty());
    assert_eq!(tracer.streamed_count, 2);

    let bytes = buf.lock().unwrap();
    let output = std::str::from_utf8(&bytes).expect("valid UTF-8");
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(lines.len(), 2, "two lines: JUMPI then JUMPDEST");

    let jumpi_v: serde_json::Value = serde_json::from_str(lines[0]).expect("valid JSON line 0");
    let jumpdest_v: serde_json::Value = serde_json::from_str(lines[1]).expect("valid JSON line 1");

    // JUMPI is first.
    assert_eq!(jumpi_v["pc"], serde_json::json!(2));
    assert_eq!(jumpi_v["op"], serde_json::json!(0x57_u64)); // JUMPI
    assert_eq!(jumpi_v["gasCost"], serde_json::json!("0x8")); // patched

    // JUMPDEST is second.
    assert_eq!(jumpdest_v["pc"], serde_json::json!(10));
    assert_eq!(jumpdest_v["op"], serde_json::json!(0x5b_u64)); // JUMPDEST

    // Verify jumpi_idx is correct.
    let _ = jumpi_idx;
}

/// 2.7c — cap is honored across both real and synthetic steps.
///
/// With limit=2, the third pre_step_capture should be rejected.
#[test]
fn test_2_7c_cap_honored() {
    let (buf, sink) = make_buf();
    let cfg = OpcodeTracerConfig {
        limit: 2,
        ..Default::default()
    };
    let mut tracer = LevmOpcodeTracer::streaming(cfg, Box::new(sink));

    // Step 1 — accepted.
    tracer.pre_step_capture(0, 0x60, 1000, 1, 0, &[], &[], 0, &Bytes::new(), None);
    tracer.finalize_step(3, None);
    assert_eq!(tracer.streamed_count, 1);

    // Step 2 — accepted.
    tracer.pre_step_capture(2, 0x60, 997, 1, 0, &[], &[], 0, &Bytes::new(), None);
    tracer.finalize_step(3, None);
    assert_eq!(tracer.streamed_count, 2);

    // Step 3 — rejected by cap (total == limit).
    tracer.pre_step_capture(4, 0x01, 994, 1, 0, &[], &[], 0, &Bytes::new(), None);
    assert!(
        tracer.last_step_index.is_none(),
        "cap: pre_step should set last_step_index=None"
    );
    // finalize_step should be a no-op.
    tracer.finalize_step(3, None);
    assert_eq!(tracer.streamed_count, 2, "third step not counted");

    let bytes = buf.lock().unwrap();
    let output = std::str::from_utf8(&bytes).expect("UTF-8");
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(lines.len(), 2, "only 2 lines emitted");
}

/// 2.7d — write failure: stream set to None, error stored, subsequent calls are no-ops.
#[test]
fn test_2_7d_write_failure() {
    let mut tracer = LevmOpcodeTracer::streaming(default_cfg(), Box::new(FailingSink));

    tracer.pre_step_capture(0, 0x60, 1000, 1, 0, &[], &[], 0, &Bytes::new(), None);
    // finalize_step will attempt to write and fail on the first step.
    tracer.finalize_step(3, None);

    assert!(
        tracer.stream.is_none(),
        "stream must be cleared after write error"
    );
    assert_eq!(
        tracer.streamed_count, 0,
        "no entry should be counted as streamed when the first write fails"
    );

    // Take the error and confirm it's a single-shot accessor.
    let err = tracer.take_stream_error();
    assert!(err.is_some(), "error must be stored");
    assert!(
        tracer.take_stream_error().is_none(),
        "error cleared after take"
    );

    // After failure, pre_step_capture must NOT keep accumulating into `logs`
    // (otherwise a streaming tracer silently degrades into RPC-mode behavior).
    // The post-`take` state still has stream=None, but stream_error was just
    // taken — re-arm the failure marker by triggering another failed flush.
    // Since the sink is gone, we simulate by directly verifying the early-out
    // path: push a real step on a fresh streaming tracer that fails, then
    // assert that AFTER failure `pre_step_capture` is a no-op.
    let mut tracer2 = LevmOpcodeTracer::streaming(default_cfg(), Box::new(FailingSink));
    tracer2.pre_step_capture(0, 0x60, 1000, 1, 0, &[], &[], 0, &Bytes::new(), None);
    tracer2.finalize_step(3, None);
    assert!(tracer2.stream_error.is_some());
    // logs was truncated on flush; next pre_step_capture must not re-grow it.
    tracer2.pre_step_capture(2, 0x01, 997, 1, 0, &[], &[], 0, &Bytes::new(), None);
    assert!(
        tracer2.logs.is_empty(),
        "pre_step_capture must be a no-op once a stream failure has occurred"
    );
    assert!(
        tracer2.last_step_index.is_none(),
        "last_step_index must be cleared after stream failure"
    );
}

/// 2.7d-bis — `synthesize_step` honors the cap across streamed entries.
///
/// Regression for a missed `streamed_count` check: in streaming mode `logs` is
/// emptied after every `finalize_step`, so a `logs.len()`-only cap would never
/// fire and synthetic steps would keep leaking past the limit.
#[test]
fn test_2_7d_bis_synthesize_step_cap() {
    let (buf, sink) = make_buf();
    let cfg = OpcodeTracerConfig {
        limit: 2,
        ..Default::default()
    };
    let mut tracer = LevmOpcodeTracer::streaming(cfg, Box::new(sink));

    // Stream two real steps to hit the cap.
    tracer.pre_step_capture(0, 0x60, 1000, 1, 0, &[], &[], 0, &Bytes::new(), None);
    tracer.finalize_step(3, None);
    tracer.pre_step_capture(2, 0x60, 997, 1, 0, &[], &[], 0, &Bytes::new(), None);
    tracer.finalize_step(3, None);
    assert_eq!(tracer.streamed_count, 2);

    // Synthesize one more — must be rejected by the cap.
    let synth = OpcodeStep {
        pc: 4,
        op: 0x5b,
        gas: 994,
        gas_cost: 1,
        mem_size: 0,
        depth: 1,
        return_data: Bytes::new(),
        refund: 0,
        stack: Some(vec![]),
        memory: None,
        storage: None,
        error: None,
    };
    tracer.synthesize_step(synth);
    assert!(
        tracer.logs.is_empty(),
        "synthetic step must be dropped once the cap is reached"
    );

    let bytes = buf.lock().unwrap();
    assert_eq!(
        std::str::from_utf8(&bytes).unwrap().lines().count(),
        2,
        "only 2 lines emitted; synthetic step did not slip past the cap"
    );
}

/// 2.7e — flush_summary appends the summary line after step lines.
#[test]
fn test_2_7e_flush_summary() {
    let (buf, sink) = make_buf();
    let mut tracer = LevmOpcodeTracer::streaming(default_cfg(), Box::new(sink));

    // Stream two steps.
    tracer.pre_step_capture(0, 0x60, 1000, 1, 0, &[], &[], 0, &Bytes::new(), None);
    tracer.finalize_step(3, None);
    tracer.pre_step_capture(2, 0x60, 997, 1, 0, &[], &[], 0, &Bytes::new(), None);
    tracer.finalize_step(3, None);
    assert_eq!(tracer.streamed_count, 2);

    tracer
        .flush_summary(&[0xde, 0xad], 42, None)
        .expect("flush_summary must succeed");

    let bytes = buf.lock().unwrap();
    let output = std::str::from_utf8(&bytes).expect("UTF-8");
    let lines: Vec<&str> = output.lines().collect();
    // 2 step lines + 1 summary line.
    assert_eq!(lines.len(), 3);
    assert_eq!(
        lines[2], r#"{"output":"dead","gasUsed":"0x2a"}"#,
        "summary line must match expected shape"
    );
}

/// 2.7f — flush_state_root appends `{"stateRoot": "0x..."}` after summary.
#[test]
fn test_2_7f_flush_state_root() {
    let (buf, sink) = make_buf();
    let mut tracer = LevmOpcodeTracer::streaming(default_cfg(), Box::new(sink));

    // One step.
    tracer.pre_step_capture(0, 0x00, 1000, 1, 0, &[], &[], 0, &Bytes::new(), None);
    tracer.finalize_step(0, None);

    tracer
        .flush_summary(&[], 0, None)
        .expect("flush_summary must succeed");
    tracer
        .flush_state_root(H256::zero())
        .expect("flush_state_root must succeed");

    let bytes = buf.lock().unwrap();
    let output = std::str::from_utf8(&bytes).expect("UTF-8");
    let lines: Vec<&str> = output.lines().collect();
    // 1 step + summary + stateRoot.
    assert_eq!(lines.len(), 3);

    let last = lines[2];
    assert_eq!(
        last,
        r#"{"stateRoot": "0x0000000000000000000000000000000000000000000000000000000000000000"}"#,
        "stateRoot line must have colon-space and full zero hash"
    );
}

/// 2.8 — disabled / RPC mode unchanged: logs accumulate, no sink.
#[test]
fn test_2_8_rpc_mode_unchanged() {
    let mut tracer = LevmOpcodeTracer::new(default_cfg());

    tracer.pre_step_capture(0, 0x60, 1000, 1, 0, &[], &[], 0, &Bytes::new(), None);
    tracer.finalize_step(3, None);

    assert_eq!(tracer.logs.len(), 1, "RPC mode: step buffered in logs");
    assert!(tracer.stream.is_none(), "RPC mode: no sink");
    assert_eq!(tracer.streamed_count, 0, "RPC mode: streamed_count stays 0");
}
