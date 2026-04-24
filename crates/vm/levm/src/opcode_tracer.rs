//! One-shot opcode tracer for diffing against external traces (e.g. bsc-geth vmtrace).
//!
//! Activated when the `ETHREX_TRACE_TX` env var is set to a tx hash (hex, with or without
//! `0x`). Output path is taken from `ETHREX_TRACE_FILE` (default `/tmp/ethrex-vmtrace.csv`).
//!
//! Gated to skip the prewarmer: tracing only fires during real execution
//! (`env.disable_balance_check == false`).
//!
//! Output format matches bsc-geth's vmtrace CSV export:
//! `Step,PC,Operation,Gas,GasCost,Depth` (quoted values).

use ethrex_common::H256;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::str::FromStr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::opcodes::Opcode;

struct TracerState {
    target_tx: Option<H256>,
    path: Option<String>,
    writer: Option<BufWriter<File>>,
    step: u64,
}

impl TracerState {
    const fn new() -> Self {
        Self {
            target_tx: None,
            path: None,
            writer: None,
            step: 0,
        }
    }
}

static STATE: Mutex<TracerState> = Mutex::new(TracerState::new());
static INIT: std::sync::Once = std::sync::Once::new();
/// Global fast-path flag. True only after env is parsed and a target hash is set.
static CONFIGURED: AtomicBool = AtomicBool::new(false);
/// True while a target tx is actively being traced. Reset on `end_tx`.
static ACTIVE: AtomicBool = AtomicBool::new(false);

fn init_from_env() {
    INIT.call_once(|| {
        let Ok(tx_hex) = std::env::var("ETHREX_TRACE_TX") else {
            return;
        };
        let trimmed = tx_hex.trim().trim_start_matches("0x");
        let Ok(hash) = H256::from_str(trimmed) else {
            eprintln!("[opcode_tracer] invalid ETHREX_TRACE_TX: {tx_hex}");
            return;
        };
        let path =
            std::env::var("ETHREX_TRACE_FILE").unwrap_or_else(|_| "/tmp/ethrex-vmtrace.csv".into());
        if let Ok(mut state) = STATE.lock() {
            state.target_tx = Some(hash);
            state.path = Some(path.clone());
            CONFIGURED.store(true, Ordering::Relaxed);
            eprintln!("[opcode_tracer] configured to trace tx {hash:?} -> {path}");
        }
    });
}

/// Called at the start of `VM::execute()`. Returns true if the current tx is the
/// target and this is a real (non-prewarm) execution. Truncates the trace file
/// on each activation so retries don't append to stale data.
pub fn begin_tx(tx_hash: H256, is_prewarm: bool) -> bool {
    init_from_env();
    if !CONFIGURED.load(Ordering::Relaxed) {
        return false;
    }
    let Ok(mut state) = STATE.lock() else {
        return false;
    };
    let is_target = state.target_tx == Some(tx_hash);
    let active = is_target && !is_prewarm;
    if active {
        let path = state.path.clone();
        if let Some(path) = path {
            match File::create(&path) {
                Ok(file) => {
                    let mut writer = BufWriter::new(file);
                    let _ = writeln!(writer, "Step,PC,Operation,Gas,GasCost,Depth");
                    state.writer = Some(writer);
                }
                Err(e) => {
                    eprintln!("[opcode_tracer] could not open trace file {path}: {e}");
                    return false;
                }
            }
        }
        state.step = 0;
        ACTIVE.store(true, Ordering::Relaxed);
        eprintln!("[opcode_tracer] begin trace for tx {tx_hash:?}");
    } else {
        ACTIVE.store(false, Ordering::Relaxed);
    }
    active
}

/// Cheap per-opcode check. Inlined so the compiler can hoist the load.
#[inline(always)]
pub fn is_active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// Emit one opcode step. Callers must gate with `is_active()` for the fast path.
pub fn trace(pc: usize, opcode: u8, gas: i64, gas_cost: i64, depth: usize) {
    if !ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    let Ok(mut state) = STATE.lock() else {
        return;
    };
    state.step = state.step.saturating_add(1);
    let step = state.step;
    let name = format!("{:?}", Opcode::from(opcode));
    let gas_str = gas.max(0);
    let cost_str = gas_cost.max(0);
    if let Some(writer) = state.writer.as_mut() {
        let _ = writeln!(
            writer,
            "\"{step}\",\"{pc}\",\"{name}\",\"{gas_str}\",\"{cost_str}\",\"{depth}\""
        );
    }
}

/// Called when the traced tx finishes (success or failure). Flushes the writer.
pub fn end_tx() {
    if !ACTIVE.swap(false, Ordering::Relaxed) {
        return;
    }
    let Ok(mut state) = STATE.lock() else {
        return;
    };
    if let Some(writer) = state.writer.as_mut() {
        let _ = writer.flush();
    }
    eprintln!("[opcode_tracer] end trace ({} steps)", state.step);
}
