//! Micro-benchmarks for isolated opcode patterns.
//!
//! Each benchmark constructs a tight EVM bytecode loop that exercises a specific
//! opcode pattern many times, measuring execution time. Loop overhead is minimized
//! by using DUP to keep operands on the stack (avoiding PUSH32 in the hot loop)
//! and by unrolling the inner operation 16x per loop iteration.
//!
//! Usage:
//!   cargo run --release --bin opcode_microbench
//!
//! Findings targeted:
//!   1. U256 overflowing_add/sub carry tracking overhead (ADD/SUB)
//!   2. NEON→GPR register spill in comparison handlers (EQ/LT)
//!   3. Rc<RefCell> borrow checks on memory access (MLOAD/MSTORE)

use bytes::Bytes;
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::{
    Address, U256,
    constants::EMPTY_TRIE_HASH,
    types::{Account, BlockHeader, Code, EIP1559Transaction, Transaction, TxKind},
};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::{
    Environment,
    db::gen_db::GeneralizedDatabase,
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use ethrex_storage::Store;
use ethrex_vm::DynVmDatabase;
use rustc_hash::FxHashMap;
use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

const SENDER_ADDRESS: u64 = 0x100;
const CONTRACT_ADDRESS: u64 = 0x42;
const RUNS: u32 = 50;
const LOOP_ITERS: u32 = 10_000;
const UNROLL: u32 = 16;

fn main() {
    println!("LEVM Opcode Micro-benchmarks");
    println!("============================\n");
    println!(
        "Each benchmark: {LOOP_ITERS} loop iters x {UNROLL} unrolled ops = {} target ops, repeated {RUNS} times.\n",
        LOOP_ITERS * UNROLL
    );

    let mut results: Vec<(&str, Duration)> = Vec::new();

    results.push(("ADD  (U256 carry)", bench("ADD  (U256 carry)", &build_binop_bench(0x01), RUNS)));
    results.push(("SUB  (U256 borrow)", bench("SUB  (U256 borrow)", &build_binop_bench(0x03), RUNS)));
    results.push(("EQ   (comparison)", bench("EQ   (comparison)", &build_binop_bench(0x14), RUNS)));
    results.push(("LT   (comparison)", bench("LT   (comparison)", &build_binop_bench(0x10), RUNS)));
    results.push(("AND  (bitwise ref)", bench("AND  (bitwise ref)", &build_binop_bench(0x16), RUNS)));
    results.push(("MLOAD  (mem read)", bench("MLOAD  (mem read)", &build_mload_bench(), RUNS)));
    results.push(("MSTORE (mem write)", bench("MSTORE (mem write)", &build_mstore_bench(), RUNS)));
    results.push(("PUSH1  (1-byte)", bench("PUSH1  (1-byte)", &build_push_n_bench(1), RUNS)));
    results.push(("PUSH2  (2-byte)", bench("PUSH2  (2-byte)", &build_push_n_bench(2), RUNS)));
    results.push(("PUSH4  (4-byte)", bench("PUSH4  (4-byte)", &build_push_n_bench(4), RUNS)));
    results.push(("PUSH20 (20-byte)", bench("PUSH20 (20-byte)", &build_push_n_bench(20), RUNS)));
    results.push(("PUSH32 (32-byte)", bench("PUSH32 (32-byte)", &build_push_n_bench(32), RUNS)));
    let baseline = bench("Baseline (DUP+POP)", &build_baseline_bench(), RUNS);
    results.push(("Baseline (DUP+POP)", baseline));

    // Summary: overhead relative to baseline
    let total_ops = LOOP_ITERS as f64 * UNROLL as f64;
    let base_ns = baseline.as_nanos() as f64;

    println!("\n--- Overhead vs Baseline ---");
    println!("{:25} {:>10} {:>10} {:>10}", "Benchmark", "Overhead", "ns/op Δ", "% slower");
    println!("{}", "-".repeat(60));
    for (name, median) in &results {
        let m_ns = median.as_nanos() as f64;
        let delta_ns = m_ns - base_ns;
        let delta_per_op = delta_ns / total_ops;
        let pct = if base_ns > 0.0 { delta_ns / base_ns * 100.0 } else { 0.0 };
        if *name != "Baseline (DUP+POP)" {
            println!("{name:25} {:>8.2?} {:>9.1}ns {:>8.1}%", median.saturating_sub(baseline), delta_per_op, pct);
        }
    }
}

fn bench(name: &str, bytecode: &[u8], runs: u32) -> Duration {
    let bytecode = Bytes::from(bytecode.to_vec());
    let mut db = init_db(bytecode);

    // Warmup
    for _ in 0..3 {
        let mut vm = init_vm(&mut db, Bytes::new());
        let report = vm.stateless_execute().unwrap();
        assert!(
            report.is_success(),
            "{name}: execution failed: {:?}",
            report.result
        );
    }

    let mut times = Vec::with_capacity(runs as usize);
    for _ in 0..runs {
        let mut vm = init_vm(&mut db, Bytes::new());
        let start = Instant::now();
        let report = black_box(vm.stateless_execute().unwrap());
        let elapsed = start.elapsed();
        assert!(report.is_success());
        times.push(elapsed);
    }

    times.sort();
    let median = times[times.len() / 2];
    let min = times[0];
    let max = times[times.len() - 1];
    let mean: Duration = times.iter().sum::<Duration>() / runs;

    let total_ops = LOOP_ITERS as u64 * UNROLL as u64;
    let ns_per_op = median.as_nanos() as f64 / total_ops as f64;

    println!(
        "{name:25} | median {median:>8.2?} | mean {mean:>8.2?} | min {min:>8.2?} | max {max:>8.2?} | {ns_per_op:>5.1}ns/op"
    );

    median
}

// ─── Bytecode builders ───────────────────────────────────────────────────

/// Binary operation benchmark (ADD, SUB, EQ, LT, AND, etc.)
///
/// Strategy: push two U256 operands ONCE before the loop, then inside the loop
/// use DUP2+DUP2 to copy them and apply the opcode+POP. Unrolled 16x.
///
/// Stack layout during loop: [counter, val_a, val_b]
///   (counter at top, val_a at stack[1], val_b at stack[2])
///
/// Each unrolled op:
///   DUP2   ; copy val_a  → [val_a, counter, val_a, val_b]
///   DUP4   ; copy val_b  → [val_b, val_a, counter, val_a, val_b]
///   <OP>   ; result       → [result, counter, val_a, val_b]
///   POP    ; discard      → [counter, val_a, val_b]
fn build_binop_bench(opcode: u8) -> Vec<u8> {
    let mut code = Vec::with_capacity(512);

    // Push operands that cause carry across all 4 U256 limbs
    // Stack: [] → [val_b]
    push32(&mut code, &(U256::MAX / 3));
    // Stack: [val_b] → [val_a, val_b]
    push32(&mut code, &(U256::MAX / 2));

    // Push loop counter
    // Stack: [val_a, val_b] → [counter, val_a, val_b]
    push4(&mut code, LOOP_ITERS);

    let loop_start = code.len();
    code.push(0x5B); // JUMPDEST

    // Unrolled inner body: 16x (DUP2, DUP4, <OP>, POP)
    for _ in 0..UNROLL {
        code.push(0x81); // DUP2  (copy val_a)
        code.push(0x83); // DUP4  (copy val_b)
        code.push(opcode);
        code.push(0x50); // POP
    }

    // Decrement counter: PUSH1 1, SWAP1, SUB → but counter is at top already
    // Actually counter is at stack[0], so: PUSH1 1, SWAP1, SUB
    // Wait — counter IS at top. So: PUSH1 1, SWAP1 is wrong.
    // counter is at top → PUSH1 1, SUB (pops counter and 1, pushes counter-1)
    // But SUB pops [a, b] and pushes a-b. With stack [1, counter, ...]:
    //   That gives 1 - counter, which is wrong.
    // We need: counter - 1. So: PUSH1 1, SWAP1, SUB
    //   stack: [counter, val_a, val_b]
    //   PUSH1 1 → [1, counter, val_a, val_b]
    //   SWAP1   → [counter, 1, val_a, val_b]
    //   SUB     → [counter-1, val_a, val_b]
    code.push(0x60); // PUSH1
    code.push(0x01);
    code.push(0x90); // SWAP1
    code.push(0x03); // SUB

    // DUP1 (copy counter for JUMPI condition check)
    code.push(0x80); // DUP1

    // PUSH1 <loop_start>, JUMPI
    assert!(loop_start < 256);
    code.push(0x60);
    code.push(loop_start as u8);
    code.push(0x57); // JUMPI

    // Cleanup: POP counter, POP val_a, POP val_b, STOP
    code.push(0x50); // POP counter (0)
    code.push(0x50); // POP val_a
    code.push(0x50); // POP val_b
    code.push(0x00); // STOP

    code
}

/// MLOAD benchmark: pre-store a word, then tight loop of PUSH1 0 + MLOAD + POP.
fn build_mload_bench() -> Vec<u8> {
    let mut code = Vec::with_capacity(512);

    // Setup: store a value at memory offset 0
    push32(&mut code, &(U256::MAX / 7));
    code.push(0x60); code.push(0x00); // PUSH1 0
    code.push(0x52); // MSTORE

    // Push loop counter
    push4(&mut code, LOOP_ITERS);

    let loop_start = code.len();
    code.push(0x5B); // JUMPDEST

    // Unrolled: 16x (PUSH1 0, MLOAD, POP)
    for _ in 0..UNROLL {
        code.push(0x60); code.push(0x00); // PUSH1 0
        code.push(0x51); // MLOAD
        code.push(0x50); // POP
    }

    // Decrement + loop
    code.push(0x60); code.push(0x01); // PUSH1 1
    code.push(0x90); // SWAP1
    code.push(0x03); // SUB
    code.push(0x80); // DUP1
    code.push(0x60); code.push(loop_start as u8); // PUSH1 <loop>
    code.push(0x57); // JUMPI

    code.push(0x50); // POP
    code.push(0x00); // STOP

    code
}

/// MSTORE benchmark: tight loop of PUSH32 + PUSH1 0 + MSTORE.
/// We can't avoid PUSH32 here since MSTORE consumes its value argument.
/// But we can use DUP to keep the value on stack.
fn build_mstore_bench() -> Vec<u8> {
    let mut code = Vec::with_capacity(512);

    // Push value to keep on stack
    // Stack: [] → [value]
    push32(&mut code, &(U256::MAX / 7));

    // Push loop counter
    // Stack: [value] → [counter, value]
    push4(&mut code, LOOP_ITERS);

    let loop_start = code.len();
    code.push(0x5B); // JUMPDEST

    // Unrolled: 16x (DUP2, PUSH1 0, MSTORE)
    // DUP2 copies value → [value, counter, value]
    // PUSH1 0           → [0, value, counter, value]
    // MSTORE consumes offset+value → [counter, value]
    for _ in 0..UNROLL {
        code.push(0x81); // DUP2 (copy value)
        code.push(0x60); code.push(0x00); // PUSH1 0
        code.push(0x52); // MSTORE
    }

    // Decrement + loop
    code.push(0x60); code.push(0x01);
    code.push(0x90); // SWAP1
    code.push(0x03); // SUB
    code.push(0x80); // DUP1
    code.push(0x60); code.push(loop_start as u8);
    code.push(0x57); // JUMPI

    code.push(0x50); // POP counter
    code.push(0x50); // POP value
    code.push(0x00); // STOP

    code
}

/// PUSHn benchmark: tight loop of PUSHn <value> + POP.
/// Measures the overhead of reading N bytes from bytecode and converting to U256.
fn build_push_n_bench(n: usize) -> Vec<u8> {
    assert!(n >= 1 && n <= 32);
    let mut code = Vec::with_capacity(2048);

    // Push loop counter
    push4(&mut code, LOOP_ITERS);

    let loop_start = code.len();
    code.push(0x5B); // JUMPDEST

    // Unrolled: 16x (PUSHn <value>, POP)
    #[expect(clippy::as_conversions)]
    let push_opcode = (0x5F + n) as u8; // PUSH1=0x60, PUSH2=0x61, ..., PUSH32=0x7F
    let value_bytes: Vec<u8> = (0..n).map(|i| (0xAB_u8.wrapping_add(i as u8))).collect();
    for _ in 0..UNROLL {
        code.push(push_opcode);
        code.extend_from_slice(&value_bytes);
        code.push(0x50); // POP
    }

    // Decrement + loop
    code.push(0x60); code.push(0x01);
    code.push(0x90); // SWAP1
    code.push(0x03); // SUB
    code.push(0x80); // DUP1
    assert!(loop_start < 256);
    code.push(0x60); code.push(loop_start as u8);
    code.push(0x57); // JUMPI

    code.push(0x50); // POP
    code.push(0x00); // STOP

    code
}

/// Baseline: DUP+POP only (same loop structure, no target opcode).
/// This measures pure dispatch + gas check + loop overhead.
fn build_baseline_bench() -> Vec<u8> {
    let mut code = Vec::with_capacity(512);

    // Push dummy values to match binop stack layout
    push32(&mut code, &(U256::MAX / 3));
    push32(&mut code, &(U256::MAX / 2));

    // Push loop counter
    push4(&mut code, LOOP_ITERS);

    let loop_start = code.len();
    code.push(0x5B); // JUMPDEST

    // Unrolled: 16x (DUP2, DUP4, POP, POP) — same DUP pattern but no opcode
    for _ in 0..UNROLL {
        code.push(0x81); // DUP2
        code.push(0x83); // DUP4
        code.push(0x50); // POP (replaces the binop)
        code.push(0x50); // POP
    }

    // Decrement + loop
    code.push(0x60); code.push(0x01);
    code.push(0x90);
    code.push(0x03);
    code.push(0x80);
    code.push(0x60); code.push(loop_start as u8);
    code.push(0x57);

    code.push(0x50);
    code.push(0x50);
    code.push(0x50);
    code.push(0x00);

    code
}

// ─── Bytecode helpers ────────────────────────────────────────────────────

fn push32(code: &mut Vec<u8>, value: &U256) {
    code.push(0x7F); // PUSH32
    code.extend_from_slice(&value.to_big_endian());
}

fn push4(code: &mut Vec<u8>, value: u32) {
    code.push(0x63); // PUSH4
    code.extend_from_slice(&value.to_be_bytes());
}

// ─── VM setup (same pattern as revm_comparison/levm_bench.rs) ────────────

fn init_db(bytecode: Bytes) -> GeneralizedDatabase {
    let in_memory_db = Store::new("", ethrex_storage::EngineType::InMemory).unwrap();
    let header = BlockHeader {
        state_root: *EMPTY_TRIE_HASH,
        ..Default::default()
    };
    let store: DynVmDatabase = Box::new(StoreVmDatabase::new(in_memory_db, header).unwrap());

    let mut cache = FxHashMap::default();
    cache.insert(
        Address::from_low_u64_be(CONTRACT_ADDRESS),
        Account::new(
            U256::MAX,
            Code::from_bytecode(bytecode),
            0,
            FxHashMap::default(),
        ),
    );
    cache.insert(
        Address::from_low_u64_be(SENDER_ADDRESS),
        Account::new(
            U256::MAX,
            Code::from_bytecode(Bytes::new()),
            0,
            FxHashMap::default(),
        ),
    );

    GeneralizedDatabase::new_with_account_state(Arc::new(store), cache)
}

fn init_vm(db: &mut GeneralizedDatabase, calldata: Bytes) -> VM<'_> {
    let env = Environment {
        origin: Address::from_low_u64_be(SENDER_ADDRESS),
        tx_nonce: 0,
        gas_limit: (i64::MAX - 1) as u64,
        block_gas_limit: (i64::MAX - 1) as u64,
        ..Default::default()
    };

    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(Address::from_low_u64_be(CONTRACT_ADDRESS)),
        data: calldata,
        ..Default::default()
    });

    VM::new(
        env,
        db,
        &tx,
        LevmCallTracer::disabled(),
        VMType::L1,
        &NativeCrypto,
    )
    .unwrap()
}
