//! Cached basic-block pre-decoded EVM IR (`CompiledCode`) for LEVM.
//!
//! Compiles deployed contract bytecode once per `(code_hash, has_push0)` into compact
//! 8-byte instructions grouped into straight-line basic blocks:
//! - Hoists per-opcode gas, stack-underflow, and stack-overflow checks to a single
//!   basic-block entry check (`BlockCheck`). On failure, control transfers seamlessly
//!   to canonical step-by-step execution at the exact block-entry `evm_pc`.
//! - Pre-decodes all `PUSH1..PUSH32` immediates into native little-endian words/constants.
//! - Pre-validates static `PUSH1/2 + JUMP/JUMPI` targets directly to IR indices.
//! - Fuses hot Solidity idioms at compile time (`DUP1+PUSH4+EQ+PUSH2+JUMPI`,
//!   `ISZERO/EQ/LT/GT + PUSH1/2 + JUMPI`, `PUSH1..4 + ADD`, `PUSH1 + SHL/SHR`, `SWAP1 + POP`).

#![cfg(not(any(
    feature = "perf_opcode_timings",
    feature = "sp1",
    feature = "risc0",
    feature = "zisk",
    feature = "openvm"
)))]

use crate::{
    constants::STACK_LIMIT,
    errors::{ContextResult, ExceptionalHalt, OpcodeResult, VMError},
    opcode_handlers::arithmetic::wrapping_mul,
    opcode_handlers::bitwise_comparison::{shift_left, shift_right},
    vm::VM,
};
use ethrex_common::{
    H256, U256,
    types::{BYTECODE_PADDING, Code, Fork},
};
use rustc_hash::FxHashMap;
use std::{
    cell::{OnceCell, RefCell},
    sync::{Arc, LazyLock, RwLock},
};

/// Maximum contract bytecode length compiled to IR (matches EIP-170 24 KiB limit,
/// which also guarantees IR instruction indices fit in `u16`).
const MAX_COMPILED_CODE_LEN: usize = 24_576;
const MAX_CACHED_CONTRACTS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum IrTag {
    BlockCheck = 0,
    Stop = 1,
    Add = 2,
    Mul = 3,
    Sub = 4,
    Lt = 5,
    Gt = 6,
    Slt = 7,
    Sgt = 8,
    Eq = 9,
    IsZero = 10,
    And = 11,
    Or = 12,
    Xor = 13,
    Not = 14,
    Shl = 15,
    Shr = 16,
    Caller = 17,
    CallValue = 18,
    CallDataLoad = 19,
    CallDataSize = 20,
    Pop = 21,
    Push0 = 22,
    PushU32 = 23,
    PushU64 = 24,
    PushU256 = 25,
    Dup = 26,
    Swap = 27,
    JumpImm = 28,
    JumpiImm = 29,
    JumpDyn = 30,
    JumpiDyn = 31,
    // Compile-time fused superinstructions
    Dup1Push4EqJumpi = 32,
    IsZeroJumpi = 33,
    EqJumpi = 34,
    LtJumpi = 35,
    GtJumpi = 36,
    PushU32Add = 37,
    PushU8Shr = 38,
    PushU8Shl = 39,
    Swap1Pop = 40,
    // Dynamic-gas / stateful instructions (outside unchecked gas blocks)
    MLoad = 41,
    MStore = 42,
    Fallback = 43,
}

#[derive(Clone, Copy, Debug)]
#[repr(C)]
struct IrOp {
    tag: IrTag,
    aux: u8,
    aux2: u16,
    data: u32,
}

const _: () = assert!(size_of::<IrOp>() == 8);

impl IrOp {
    #[inline(always)]
    const fn simple(tag: IrTag) -> Self {
        Self {
            tag,
            aux: 0,
            aux2: 0,
            data: 0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct BlockMeta {
    gas: i64,
    min_stack: u16,
    max_growth: u16,
    evm_pc: u32,
}

pub(crate) struct CompiledCode {
    ops: Vec<IrOp>,
    blocks: Vec<BlockMeta>,
    const_u64: Vec<u64>,
    const_u256: Vec<U256>,
    /// Valid `JUMPDEST` byte offset -> IR instruction index (`u32::MAX` if not a valid jumpdest).
    jumpdest_to_ir: Vec<u32>,
    /// Post-CALL/CREATE resume byte offset -> IR instruction index.
    resume_to_ir: FxHashMap<u32, u32>,
}

type CompiledCodeCache = FxHashMap<(H256, bool), Arc<CompiledCode>>;

/// Global cache of compiled contracts keyed by `(code_hash, has_push0)`.
static IR_CACHE: LazyLock<RwLock<CompiledCodeCache>> =
    LazyLock::new(|| RwLock::new(FxHashMap::default()));

thread_local! {
    /// 1-slot per-thread MRU cache so repeated calls to the same hot contract (e.g. an ERC-20
    /// token) resolve with a single `H256` compare and zero `RwLock` traffic.
    static TLS_LAST_COMPILED: RefCell<Option<(H256, bool, Arc<CompiledCode>)>> =
        const { RefCell::new(None) };
}

fn get_or_compile(code: &Code, has_push0: bool) -> Option<Arc<CompiledCode>> {
    let hash = code.hash;
    // Never cache unhashed CREATE initcode (`H256::zero()`), empty code, or oversized code.
    if hash == H256::zero() || code.is_empty() || code.len() > MAX_COMPILED_CODE_LEN {
        return None;
    }

    // Fast path: thread-local 1-slot MRU hit (zero synchronization).
    if let Some(hit) = TLS_LAST_COMPILED.with(|cell| {
        let borrow = cell.borrow();
        if let Some((cached_hash, cached_push0, ref compiled)) = *borrow
            && cached_hash == hash
            && cached_push0 == has_push0
        {
            Some(Arc::clone(compiled))
        } else {
            None
        }
    }) {
        return Some(hit);
    }

    // Shared cache lookup.
    if let Ok(guard) = IR_CACHE.read()
        && let Some(compiled) = guard.get(&(hash, has_push0))
    {
        let arc = Arc::clone(compiled);
        TLS_LAST_COMPILED.with(|cell| {
            *cell.borrow_mut() = Some((hash, has_push0, Arc::clone(&arc)));
        });
        return Some(arc);
    }

    let compiled = Arc::new(compile_bytecode(code, has_push0));
    if let Ok(mut guard) = IR_CACHE.write() {
        if guard.len() >= MAX_CACHED_CONTRACTS {
            guard.clear();
        }
        guard.insert((hash, has_push0), Arc::clone(&compiled));
    }
    TLS_LAST_COMPILED.with(|cell| {
        *cell.borrow_mut() = Some((hash, has_push0, Arc::clone(&compiled)));
    });
    Some(compiled)
}

#[derive(Clone, Copy)]
struct PendingPatch {
    ir_idx: usize,
    dest_pc: usize,
    /// If true, patch `aux2` (u16); otherwise patch `data` (u32).
    patch_aux2: bool,
}

struct BlockBuilder {
    active: bool,
    block_op_idx: usize,
    meta_idx: usize,
    evm_pc: u32,
    gas: i64,
    depth: i32,
    min_stack: i32,
    max_growth: i32,
}

impl BlockBuilder {
    const fn new() -> Self {
        Self {
            active: false,
            block_op_idx: 0,
            meta_idx: 0,
            evm_pc: 0,
            gas: 0,
            depth: 0,
            min_stack: 0,
            max_growth: 0,
        }
    }

    #[expect(
        clippy::as_conversions,
        reason = "block counts and evm_pc are bounded by MAX_COMPILED_CODE_LEN"
    )]
    fn ensure_open(&mut self, ops: &mut Vec<IrOp>, blocks: &mut Vec<BlockMeta>, evm_pc: usize) {
        if !self.active {
            self.active = true;
            self.block_op_idx = ops.len();
            self.meta_idx = blocks.len();
            self.evm_pc = evm_pc as u32;
            self.gas = 0;
            self.depth = 0;
            self.min_stack = 0;
            self.max_growth = 0;
            ops.push(IrOp {
                tag: IrTag::BlockCheck,
                aux: 0,
                aux2: 0,
                data: self.meta_idx as u32,
            });
            blocks.push(BlockMeta {
                gas: 0,
                min_stack: 0,
                max_growth: 0,
                evm_pc: evm_pc as u32,
            });
        }
    }

    #[expect(
        clippy::arithmetic_side_effects,
        reason = "stack requirements and gas are bounded by block length <= 65535"
    )]
    fn record_op(&mut self, gas: i64, min_req: i32, peak_growth: i32, net_delta: i32) {
        self.gas += gas;
        let needed = min_req - self.depth;
        if needed > self.min_stack {
            self.min_stack = needed;
        }
        let peak = self.depth + peak_growth;
        if peak > self.max_growth {
            self.max_growth = peak;
        }
        self.depth += net_delta;
    }

    #[expect(
        clippy::as_conversions,
        clippy::indexing_slicing,
        reason = "meta_idx is always valid when active; stack bounds fit in u16"
    )]
    fn close(&mut self, blocks: &mut [BlockMeta]) {
        if self.active {
            blocks[self.meta_idx] = BlockMeta {
                gas: self.gas,
                min_stack: self.min_stack.max(0) as u16,
                max_growth: self.max_growth.max(0) as u16,
                evm_pc: self.evm_pc,
            };
            self.active = false;
        }
    }
}

/// Reads a static `PUSH1` or `PUSH2` immediate at `pc` followed by `expected_next` (`0x56` JUMP
/// or `0x57` JUMPI). Returns `(dest_pc, bytes_consumed)` if the target is a valid `JUMPDEST`.
#[expect(
    clippy::indexing_slicing,
    clippy::as_conversions,
    clippy::arithmetic_side_effects,
    reason = "buf has BYTECODE_PADDING > 32 trailing zero bytes"
)]
fn match_static_jump(
    buf: &[u8],
    code: &Code,
    pc: usize,
    expected_next: u8,
) -> Option<(usize, usize)> {
    match buf[pc] {
        0x60 if buf[pc + 2] == expected_next => {
            let dest = buf[pc + 1] as usize;
            code.is_valid_jumpdest(dest).then_some((dest, 3))
        }
        0x61 if buf[pc + 3] == expected_next => {
            let dest = u16::from_be_bytes([buf[pc + 1], buf[pc + 2]]) as usize;
            code.is_valid_jumpdest(dest).then_some((dest, 4))
        }
        _ => None,
    }
}

#[expect(
    clippy::indexing_slicing,
    clippy::as_conversions,
    clippy::arithmetic_side_effects,
    reason = "buf has BYTECODE_PADDING > 32 trailing zero bytes and code_len <= u16::MAX"
)]
fn compile_bytecode(code: &Code, has_push0: bool) -> CompiledCode {
    const { assert!(BYTECODE_PADDING > 32) };

    let buf = code.dispatch_buf();
    let code_len = code.len();
    let mut ops = Vec::with_capacity(code_len);
    let mut blocks = Vec::with_capacity(code_len / 8 + 4);
    let mut const_u64 = Vec::new();
    let mut const_u256 = Vec::new();
    let mut jumpdest_to_ir = vec![u32::MAX; code_len];
    let mut resume_to_ir = FxHashMap::default();
    let mut patches: Vec<PendingPatch> = Vec::new();
    let mut bb = BlockBuilder::new();

    let mut pc = 0usize;
    while pc < code_len {
        let op_pc = pc;
        let opcode = buf[pc];

        // JUMPDEST always opens a fresh basic block so both fallthrough and jumps
        // land on its `BlockCheck` and account for the 1 gas of `JUMPDEST`.
        if opcode == 0x5B {
            bb.close(&mut blocks);
            let ir_entry = ops.len() as u32;
            jumpdest_to_ir[op_pc] = ir_entry;
            bb.ensure_open(&mut ops, &mut blocks, op_pc);
            bb.record_op(1, 0, 0, 0);
            pc += 1;
            continue;
        }

        // STOP (0x00)
        if opcode == 0x00 {
            bb.close(&mut blocks);
            ops.push(IrOp::simple(IrTag::Stop));
            pc += 1;
            continue;
        }

        // Try compile-time superinstruction fusions first.

        // 1) Solidity selector dispatch: `DUP1; PUSH4 sel; EQ; PUSH2 dest; JUMPI`
        if opcode == 0x80
            && buf[pc + 1] == 0x63
            && buf[pc + 6] == 0x14
            && buf[pc + 7] == 0x61
            && buf[pc + 10] == 0x57
        {
            let dest = u16::from_be_bytes([buf[pc + 8], buf[pc + 9]]) as usize;
            if code.is_valid_jumpdest(dest) {
                let sel = u32::from_be_bytes([buf[pc + 2], buf[pc + 3], buf[pc + 4], buf[pc + 5]]);
                bb.ensure_open(&mut ops, &mut blocks, op_pc);
                bb.record_op(22, 1, 2, 0);
                let ir_idx = ops.len();
                ops.push(IrOp {
                    tag: IrTag::Dup1Push4EqJumpi,
                    aux: 0,
                    aux2: 0,
                    data: sel,
                });
                patches.push(PendingPatch {
                    ir_idx,
                    dest_pc: dest,
                    patch_aux2: true,
                });
                bb.close(&mut blocks);
                pc += 11;
                continue;
            }
        }

        // 2) `ISZERO + PUSH1/2 + JUMPI`
        if opcode == 0x15
            && let Some((dest, jlen)) = match_static_jump(buf, code, pc + 1, 0x57)
        {
            bb.ensure_open(&mut ops, &mut blocks, op_pc);
            bb.record_op(16, 1, 1, -1);
            let ir_idx = ops.len();
            ops.push(IrOp {
                tag: IrTag::IsZeroJumpi,
                aux: 0,
                aux2: 0,
                data: 0,
            });
            patches.push(PendingPatch {
                ir_idx,
                dest_pc: dest,
                patch_aux2: false,
            });
            bb.close(&mut blocks);
            pc += 1 + jlen;
            continue;
        }

        // 3) `EQ / LT / GT + PUSH1/2 + JUMPI`
        if matches!(opcode, 0x10 | 0x11 | 0x14)
            && let Some((dest, jlen)) = match_static_jump(buf, code, pc + 1, 0x57)
        {
            let tag = match opcode {
                0x10 => IrTag::LtJumpi,
                0x11 => IrTag::GtJumpi,
                _ => IrTag::EqJumpi,
            };
            bb.ensure_open(&mut ops, &mut blocks, op_pc);
            bb.record_op(16, 2, 0, -2);
            let ir_idx = ops.len();
            ops.push(IrOp {
                tag,
                aux: 0,
                aux2: 0,
                data: 0,
            });
            patches.push(PendingPatch {
                ir_idx,
                dest_pc: dest,
                patch_aux2: false,
            });
            bb.close(&mut blocks);
            pc += 1 + jlen;
            continue;
        }

        // 4) `PUSH1/2 + JUMP` or `PUSH1/2 + JUMPI` (pre-validated jumpdest)
        if matches!(opcode, 0x60 | 0x61) {
            if let Some((dest, jlen)) = match_static_jump(buf, code, pc, 0x56) {
                bb.ensure_open(&mut ops, &mut blocks, op_pc);
                bb.record_op(11, 0, 1, 0);
                let ir_idx = ops.len();
                ops.push(IrOp {
                    tag: IrTag::JumpImm,
                    aux: 0,
                    aux2: 0,
                    data: 0,
                });
                patches.push(PendingPatch {
                    ir_idx,
                    dest_pc: dest,
                    patch_aux2: false,
                });
                bb.close(&mut blocks);
                pc += jlen;
                continue;
            }
            if let Some((dest, jlen)) = match_static_jump(buf, code, pc, 0x57) {
                bb.ensure_open(&mut ops, &mut blocks, op_pc);
                bb.record_op(13, 1, 1, -1);
                let ir_idx = ops.len();
                ops.push(IrOp {
                    tag: IrTag::JumpiImm,
                    aux: 0,
                    aux2: 0,
                    data: 0,
                });
                patches.push(PendingPatch {
                    ir_idx,
                    dest_pc: dest,
                    patch_aux2: false,
                });
                bb.close(&mut blocks);
                pc += jlen;
                continue;
            }
        }

        // 5) `PUSH1 imm + SHR / SHL`
        if opcode == 0x60 && matches!(buf[pc + 2], 0x1B | 0x1C) {
            let shift = buf[pc + 1];
            let tag = if buf[pc + 2] == 0x1B {
                IrTag::PushU8Shl
            } else {
                IrTag::PushU8Shr
            };
            bb.ensure_open(&mut ops, &mut blocks, op_pc);
            bb.record_op(6, 1, 1, 0);
            ops.push(IrOp {
                tag,
                aux: shift,
                aux2: 0,
                data: 0,
            });
            pc += 3;
            continue;
        }

        // 6) `PUSH1..PUSH4 + ADD`
        if matches!(opcode, 0x60..=0x63) {
            let n = (opcode - 0x5F) as usize;
            if buf[pc + 1 + n] == 0x01 {
                let mut imm = 0u32;
                for i in 0..n {
                    imm = (imm << 8) | (buf[pc + 1 + i] as u32);
                }
                bb.ensure_open(&mut ops, &mut blocks, op_pc);
                bb.record_op(6, 1, 1, 0);
                ops.push(IrOp {
                    tag: IrTag::PushU32Add,
                    aux: 0,
                    aux2: 0,
                    data: imm,
                });
                pc += 2 + n;
                continue;
            }
        }

        // 7) `SWAP1 + POP`
        if opcode == 0x90 && buf[pc + 1] == 0x50 {
            bb.ensure_open(&mut ops, &mut blocks, op_pc);
            bb.record_op(5, 2, 0, -1);
            ops.push(IrOp::simple(IrTag::Swap1Pop));
            pc += 2;
            continue;
        }

        // Regular constant-gas pure EVM opcodes inside the current basic block.
        match opcode {
            0x01 => {
                bb.ensure_open(&mut ops, &mut blocks, op_pc);
                bb.record_op(3, 2, 0, -1);
                ops.push(IrOp::simple(IrTag::Add));
                pc += 1;
            }
            0x02 => {
                bb.ensure_open(&mut ops, &mut blocks, op_pc);
                bb.record_op(5, 2, 0, -1);
                ops.push(IrOp::simple(IrTag::Mul));
                pc += 1;
            }
            0x03 => {
                bb.ensure_open(&mut ops, &mut blocks, op_pc);
                bb.record_op(3, 2, 0, -1);
                ops.push(IrOp::simple(IrTag::Sub));
                pc += 1;
            }
            0x10 => {
                bb.ensure_open(&mut ops, &mut blocks, op_pc);
                bb.record_op(3, 2, 0, -1);
                ops.push(IrOp::simple(IrTag::Lt));
                pc += 1;
            }
            0x11 => {
                bb.ensure_open(&mut ops, &mut blocks, op_pc);
                bb.record_op(3, 2, 0, -1);
                ops.push(IrOp::simple(IrTag::Gt));
                pc += 1;
            }
            0x12 => {
                bb.ensure_open(&mut ops, &mut blocks, op_pc);
                bb.record_op(3, 2, 0, -1);
                ops.push(IrOp::simple(IrTag::Slt));
                pc += 1;
            }
            0x13 => {
                bb.ensure_open(&mut ops, &mut blocks, op_pc);
                bb.record_op(3, 2, 0, -1);
                ops.push(IrOp::simple(IrTag::Sgt));
                pc += 1;
            }
            0x14 => {
                bb.ensure_open(&mut ops, &mut blocks, op_pc);
                bb.record_op(3, 2, 0, -1);
                ops.push(IrOp::simple(IrTag::Eq));
                pc += 1;
            }
            0x15 => {
                bb.ensure_open(&mut ops, &mut blocks, op_pc);
                bb.record_op(3, 1, 0, 0);
                ops.push(IrOp::simple(IrTag::IsZero));
                pc += 1;
            }
            0x16 => {
                bb.ensure_open(&mut ops, &mut blocks, op_pc);
                bb.record_op(3, 2, 0, -1);
                ops.push(IrOp::simple(IrTag::And));
                pc += 1;
            }
            0x17 => {
                bb.ensure_open(&mut ops, &mut blocks, op_pc);
                bb.record_op(3, 2, 0, -1);
                ops.push(IrOp::simple(IrTag::Or));
                pc += 1;
            }
            0x18 => {
                bb.ensure_open(&mut ops, &mut blocks, op_pc);
                bb.record_op(3, 2, 0, -1);
                ops.push(IrOp::simple(IrTag::Xor));
                pc += 1;
            }
            0x19 => {
                bb.ensure_open(&mut ops, &mut blocks, op_pc);
                bb.record_op(3, 1, 0, 0);
                ops.push(IrOp::simple(IrTag::Not));
                pc += 1;
            }
            0x1B => {
                bb.ensure_open(&mut ops, &mut blocks, op_pc);
                bb.record_op(3, 2, 0, -1);
                ops.push(IrOp::simple(IrTag::Shl));
                pc += 1;
            }
            0x1C => {
                bb.ensure_open(&mut ops, &mut blocks, op_pc);
                bb.record_op(3, 2, 0, -1);
                ops.push(IrOp::simple(IrTag::Shr));
                pc += 1;
            }
            0x33 => {
                bb.ensure_open(&mut ops, &mut blocks, op_pc);
                bb.record_op(2, 0, 1, 1);
                ops.push(IrOp::simple(IrTag::Caller));
                pc += 1;
            }
            0x34 => {
                bb.ensure_open(&mut ops, &mut blocks, op_pc);
                bb.record_op(2, 0, 1, 1);
                ops.push(IrOp::simple(IrTag::CallValue));
                pc += 1;
            }
            0x35 => {
                bb.ensure_open(&mut ops, &mut blocks, op_pc);
                bb.record_op(3, 1, 0, 0);
                ops.push(IrOp::simple(IrTag::CallDataLoad));
                pc += 1;
            }
            0x36 => {
                bb.ensure_open(&mut ops, &mut blocks, op_pc);
                bb.record_op(2, 0, 1, 1);
                ops.push(IrOp::simple(IrTag::CallDataSize));
                pc += 1;
            }
            0x50 => {
                bb.ensure_open(&mut ops, &mut blocks, op_pc);
                bb.record_op(2, 1, 0, -1);
                ops.push(IrOp::simple(IrTag::Pop));
                pc += 1;
            }
            0x51 => {
                bb.close(&mut blocks);
                ops.push(IrOp {
                    tag: IrTag::MLoad,
                    aux: 0,
                    aux2: 0,
                    data: (pc + 1) as u32,
                });
                pc += 1;
            }
            0x52 => {
                bb.close(&mut blocks);
                ops.push(IrOp {
                    tag: IrTag::MStore,
                    aux: 0,
                    aux2: 0,
                    data: (pc + 1) as u32,
                });
                pc += 1;
            }
            0x56 => {
                bb.ensure_open(&mut ops, &mut blocks, op_pc);
                bb.record_op(8, 1, 0, -1);
                ops.push(IrOp {
                    tag: IrTag::JumpDyn,
                    aux: 0,
                    aux2: 0,
                    data: op_pc as u32,
                });
                bb.close(&mut blocks);
                pc += 1;
            }
            0x57 => {
                bb.ensure_open(&mut ops, &mut blocks, op_pc);
                bb.record_op(10, 2, 0, -2);
                ops.push(IrOp {
                    tag: IrTag::JumpiDyn,
                    aux: 0,
                    aux2: 0,
                    data: op_pc as u32,
                });
                bb.close(&mut blocks);
                pc += 1;
            }
            0x5F if has_push0 => {
                bb.ensure_open(&mut ops, &mut blocks, op_pc);
                bb.record_op(2, 0, 1, 1);
                ops.push(IrOp::simple(IrTag::Push0));
                pc += 1;
            }
            0x60..=0x63 => {
                let n = (opcode - 0x5F) as usize;
                let mut imm = 0u32;
                for i in 0..n {
                    imm = (imm << 8) | (buf[pc + 1 + i] as u32);
                }
                bb.ensure_open(&mut ops, &mut blocks, op_pc);
                bb.record_op(3, 0, 1, 1);
                ops.push(IrOp {
                    tag: IrTag::PushU32,
                    aux: 0,
                    aux2: 0,
                    data: imm,
                });
                pc += 1 + n;
            }
            0x64..=0x67 => {
                let n = (opcode - 0x5F) as usize;
                let mut imm = 0u64;
                for i in 0..n {
                    imm = (imm << 8) | (buf[pc + 1 + i] as u64);
                }
                let idx = const_u64.len() as u32;
                const_u64.push(imm);
                bb.ensure_open(&mut ops, &mut blocks, op_pc);
                bb.record_op(3, 0, 1, 1);
                ops.push(IrOp {
                    tag: IrTag::PushU64,
                    aux: 0,
                    aux2: 0,
                    data: idx,
                });
                pc += 1 + n;
            }
            0x68..=0x7F => {
                let n = (opcode - 0x5F) as usize;
                let mut bytes = [0u8; 32];
                bytes[32 - n..32].copy_from_slice(&buf[pc + 1..pc + 1 + n]);
                let val = U256::from_big_endian(&bytes);
                let idx = const_u256.len() as u32;
                const_u256.push(val);
                bb.ensure_open(&mut ops, &mut blocks, op_pc);
                bb.record_op(3, 0, 1, 1);
                ops.push(IrOp {
                    tag: IrTag::PushU256,
                    aux: 0,
                    aux2: 0,
                    data: idx,
                });
                pc += 1 + n;
            }
            0x80..=0x8F => {
                let depth_idx = opcode - 0x80;
                bb.ensure_open(&mut ops, &mut blocks, op_pc);
                bb.record_op(3, (depth_idx as i32) + 1, 1, 1);
                ops.push(IrOp {
                    tag: IrTag::Dup,
                    aux: depth_idx,
                    aux2: 0,
                    data: 0,
                });
                pc += 1;
            }
            0x90..=0x9F => {
                let depth_idx = opcode - 0x8F;
                bb.ensure_open(&mut ops, &mut blocks, op_pc);
                bb.record_op(3, (depth_idx as i32) + 1, 0, 0);
                ops.push(IrOp {
                    tag: IrTag::Swap,
                    aux: depth_idx,
                    aux2: 0,
                    data: 0,
                });
                pc += 1;
            }
            _ => {
                bb.close(&mut blocks);
                let next_pc = (pc + 1) as u32;
                ops.push(IrOp {
                    tag: IrTag::Fallback,
                    aux: opcode,
                    aux2: 0,
                    data: next_pc,
                });
                // Record resume point for CALL/CREATE family opcodes so returning from a child
                // call frame continues seamlessly inside the compiled IR loop.
                if matches!(opcode, 0xF0..=0xF5 | 0xFA) {
                    resume_to_ir.insert(next_pc, ops.len() as u32);
                }
                pc += 1;
            }
        }
    }

    bb.close(&mut blocks);
    // Sentinel `Stop` so falling off the end of bytecode halts cleanly.
    ops.push(IrOp::simple(IrTag::Stop));

    // Patch all static jump targets from EVM byte PC -> IR instruction index.
    for patch in patches {
        let target_ir = jumpdest_to_ir[patch.dest_pc];
        debug_assert_ne!(target_ir, u32::MAX, "validated jumpdest missing IR target");
        if patch.patch_aux2 {
            // If IR exceeded u16::MAX (impossible for <= 24KB contracts unless extreme),
            // clamp safety is guaranteed by checking fit.
            if let Ok(t16) = u16::try_from(target_ir) {
                ops[patch.ir_idx].aux2 = t16;
            } else {
                // Safe fallback: convert to BlockCheck-less fallback at runtime (unreachable
                // for valid <= 24KB Ethereum contracts, where ops.len() < 35000).
                ops[patch.ir_idx] = IrOp::simple(IrTag::Stop);
            }
        } else {
            ops[patch.ir_idx].data = target_ir;
        }
    }

    CompiledCode {
        ops,
        blocks,
        const_u64,
        const_u256,
        jumpdest_to_ir,
        resume_to_ir,
    }
}

impl<'a> VM<'a> {
    /// Executes the current call-frame stack using cached basic-block compiled IR when
    /// applicable, falling back seamlessly to `run_dispatch_fast` on initcode, empty code,
    /// pre-Constantinople forks, or basic-block gas/stack boundary errors.
    #[expect(
        unsafe_code,
        clippy::indexing_slicing,
        clippy::as_conversions,
        clippy::arithmetic_side_effects,
        reason = "basic-block entry checks guarantee gas and stack bounds for all inner IR ops"
    )]
    pub(crate) fn run_dispatch_ir(&mut self) -> Result<ContextResult, VMError> {
        let fork = self.env.config.fork;
        if fork < Fork::Constantinople {
            return self.run_dispatch_fast();
        }
        let has_push0 = fork >= Fork::Shanghai;
        let opcode_table = self.opcode_table;
        let debug_enabled = self.debug_mode.enabled;
        let mut error = OnceCell::<VMError>::new();

        enum IrExit {
            Stop,
            Error(VMError),
            ReloadFrame,
            FallbackAtPc(usize),
        }

        loop {
            let Some(compiled) = get_or_compile(&self.current_call_frame.bytecode, has_push0)
            else {
                return self.run_dispatch_fast();
            };

            let start_pc = self.current_call_frame.pc;
            let mut ip: usize = if start_pc == 0 {
                0
            } else if let Some(&resume_ip) = compiled.resume_to_ir.get(&(start_pc as u32)) {
                resume_ip as usize
            } else if let Some(&jd_ip) = compiled.jumpdest_to_ir.get(start_pc)
                && jd_ip != u32::MAX
            {
                jd_ip as usize
            } else {
                return self.run_dispatch_fast();
            };

            let ops_ptr = compiled.ops.as_ptr();
            let blocks_ptr = compiled.blocks.as_ptr();
            let const_u64_ptr = compiled.const_u64.as_ptr();
            let const_u256_ptr = compiled.const_u256.as_ptr();
            let jumpdest_slice = compiled.jumpdest_to_ir.as_slice();
            let entry_depth = self.call_frames.len();

            let mut gas = self.current_call_frame.gas_remaining;
            let mut sp = self.current_call_frame.stack.offset;
            let stack_ptr = self.current_call_frame.stack.values.as_mut_ptr();

            let exit: IrExit = loop {
                let op = unsafe { *ops_ptr.add(ip) };
                ip += 1;

                match op.tag {
                    IrTag::BlockCheck => {
                        let meta = unsafe { *blocks_ptr.add(op.data as usize) };
                        if gas < meta.gas
                            || sp > STACK_LIMIT - (meta.min_stack as usize)
                            || sp < (meta.max_growth as usize)
                        {
                            break IrExit::FallbackAtPc(meta.evm_pc as usize);
                        }
                        gas -= meta.gas;
                    }

                    IrTag::Stop => break IrExit::Stop,

                    IrTag::Add => {
                        let new_sp = sp + 1;
                        unsafe {
                            let a = *stack_ptr.add(sp);
                            let b = &mut *stack_ptr.add(new_sp);
                            *b = a.overflowing_add(*b).0;
                        }
                        sp = new_sp;
                    }

                    IrTag::Mul => {
                        let new_sp = sp + 1;
                        unsafe {
                            let a = &*stack_ptr.add(sp);
                            let b = &mut *stack_ptr.add(new_sp);
                            *b = wrapping_mul(a, b);
                        }
                        sp = new_sp;
                    }

                    IrTag::Sub => {
                        let new_sp = sp + 1;
                        unsafe {
                            let a = *stack_ptr.add(sp);
                            let b = &mut *stack_ptr.add(new_sp);
                            *b = a.overflowing_sub(*b).0;
                        }
                        sp = new_sp;
                    }

                    IrTag::Lt => {
                        let new_sp = sp + 1;
                        unsafe {
                            let a = *stack_ptr.add(sp);
                            let b = &mut *stack_ptr.add(new_sp);
                            *b = U256::from((a < *b) as u64);
                        }
                        sp = new_sp;
                    }

                    IrTag::Gt => {
                        let new_sp = sp + 1;
                        unsafe {
                            let a = *stack_ptr.add(sp);
                            let b = &mut *stack_ptr.add(new_sp);
                            *b = U256::from((a > *b) as u64);
                        }
                        sp = new_sp;
                    }

                    IrTag::Slt => {
                        let new_sp = sp + 1;
                        unsafe {
                            let a = *stack_ptr.add(sp);
                            let b = &mut *stack_ptr.add(new_sp);
                            let a_neg = (a.0[3] >> 63) != 0;
                            let b_neg = (b.0[3] >> 63) != 0;
                            let res = if a_neg != b_neg { a_neg } else { a < *b };
                            *b = U256::from(res as u64);
                        }
                        sp = new_sp;
                    }

                    IrTag::Sgt => {
                        let new_sp = sp + 1;
                        unsafe {
                            let a = *stack_ptr.add(sp);
                            let b = &mut *stack_ptr.add(new_sp);
                            let a_neg = (a.0[3] >> 63) != 0;
                            let b_neg = (b.0[3] >> 63) != 0;
                            let res = if a_neg != b_neg { b_neg } else { a > *b };
                            *b = U256::from(res as u64);
                        }
                        sp = new_sp;
                    }

                    IrTag::Eq => {
                        let new_sp = sp + 1;
                        unsafe {
                            let a = *stack_ptr.add(sp);
                            let b = &mut *stack_ptr.add(new_sp);
                            *b = U256::from((a == *b) as u64);
                        }
                        sp = new_sp;
                    }

                    IrTag::IsZero => unsafe {
                        let slot = &mut *stack_ptr.add(sp);
                        *slot = U256::from(slot.is_zero() as u64);
                    },

                    IrTag::And => {
                        let new_sp = sp + 1;
                        unsafe {
                            let a = *stack_ptr.add(sp);
                            let b = &mut *stack_ptr.add(new_sp);
                            b.0[0] &= a.0[0];
                            b.0[1] &= a.0[1];
                            b.0[2] &= a.0[2];
                            b.0[3] &= a.0[3];
                        }
                        sp = new_sp;
                    }

                    IrTag::Or => {
                        let new_sp = sp + 1;
                        unsafe {
                            let a = *stack_ptr.add(sp);
                            let b = &mut *stack_ptr.add(new_sp);
                            b.0[0] |= a.0[0];
                            b.0[1] |= a.0[1];
                            b.0[2] |= a.0[2];
                            b.0[3] |= a.0[3];
                        }
                        sp = new_sp;
                    }

                    IrTag::Xor => {
                        let new_sp = sp + 1;
                        unsafe {
                            let a = *stack_ptr.add(sp);
                            let b = &mut *stack_ptr.add(new_sp);
                            b.0[0] ^= a.0[0];
                            b.0[1] ^= a.0[1];
                            b.0[2] ^= a.0[2];
                            b.0[3] ^= a.0[3];
                        }
                        sp = new_sp;
                    }

                    IrTag::Not => unsafe {
                        let slot = &mut *stack_ptr.add(sp);
                        slot.0[0] = !slot.0[0];
                        slot.0[1] = !slot.0[1];
                        slot.0[2] = !slot.0[2];
                        slot.0[3] = !slot.0[3];
                    },

                    IrTag::Shl => {
                        let new_sp = sp + 1;
                        unsafe {
                            let shift_amount = *stack_ptr.add(sp);
                            let slot = &mut *stack_ptr.add(new_sp);
                            *slot = match u8::try_from(shift_amount) {
                                Ok(s) => shift_left(slot, s),
                                Err(_) => U256::zero(),
                            };
                        }
                        sp = new_sp;
                    }

                    IrTag::Shr => {
                        let new_sp = sp + 1;
                        unsafe {
                            let shift_amount = *stack_ptr.add(sp);
                            let slot = &mut *stack_ptr.add(new_sp);
                            *slot = match u8::try_from(shift_amount) {
                                Ok(s) => shift_right(slot, s, 0),
                                Err(_) => U256::zero(),
                            };
                        }
                        sp = new_sp;
                    }

                    IrTag::Caller => {
                        sp -= 1;
                        unsafe {
                            *stack_ptr.add(sp) =
                                crate::utils::address_to_word(self.current_call_frame.msg_sender);
                        }
                    }

                    IrTag::CallValue => {
                        sp -= 1;
                        unsafe {
                            *stack_ptr.add(sp) = self.current_call_frame.msg_value;
                        }
                    }

                    IrTag::CallDataLoad => {
                        let offset_word = unsafe { *stack_ptr.add(sp) };
                        let value = match usize::try_from(offset_word) {
                            Ok(offset) => {
                                match self.current_call_frame.calldata.load_word(offset) {
                                    Ok(v) => v,
                                    Err(e) => {
                                        self.current_call_frame.gas_remaining = gas;
                                        self.current_call_frame.stack.offset = sp;
                                        return Err(e);
                                    }
                                }
                            }
                            Err(_) => U256::zero(),
                        };
                        unsafe {
                            *stack_ptr.add(sp) = value;
                        }
                    }

                    IrTag::CallDataSize => {
                        sp -= 1;
                        unsafe {
                            *stack_ptr.add(sp) = U256::from(self.current_call_frame.calldata.len());
                        }
                    }

                    IrTag::Pop => {
                        sp += 1;
                    }

                    IrTag::Push0 => {
                        sp -= 1;
                        unsafe {
                            *stack_ptr.add(sp) = U256::zero();
                        }
                    }

                    IrTag::PushU32 => {
                        sp -= 1;
                        unsafe {
                            let slot = &mut *stack_ptr.add(sp);
                            slot.0[0] = op.data as u64;
                            slot.0[1] = 0;
                            slot.0[2] = 0;
                            slot.0[3] = 0;
                        }
                    }

                    IrTag::PushU64 => {
                        sp -= 1;
                        unsafe {
                            let v = *const_u64_ptr.add(op.data as usize);
                            let slot = &mut *stack_ptr.add(sp);
                            slot.0[0] = v;
                            slot.0[1] = 0;
                            slot.0[2] = 0;
                            slot.0[3] = 0;
                        }
                    }

                    IrTag::PushU256 => {
                        sp -= 1;
                        unsafe {
                            *stack_ptr.add(sp) = *const_u256_ptr.add(op.data as usize);
                        }
                    }

                    IrTag::Dup => {
                        let src = sp + (op.aux as usize);
                        sp -= 1;
                        unsafe {
                            *stack_ptr.add(sp) = *stack_ptr.add(src);
                        }
                    }

                    IrTag::Swap => {
                        let dst = sp + (op.aux as usize);
                        unsafe {
                            std::ptr::swap(stack_ptr.add(sp), stack_ptr.add(dst));
                        }
                    }

                    IrTag::JumpImm => {
                        ip = op.data as usize;
                    }

                    IrTag::JumpiImm => {
                        let cond = unsafe { *stack_ptr.add(sp) };
                        sp += 1;
                        if !cond.is_zero() {
                            ip = op.data as usize;
                        }
                    }

                    IrTag::JumpDyn => {
                        let dest_word = unsafe { *stack_ptr.add(sp) };
                        sp += 1;
                        if dest_word.0[1] == 0 && dest_word.0[2] == 0 && dest_word.0[3] == 0 {
                            let dest = dest_word.0[0] as usize;
                            if let Some(&target_ir) = jumpdest_slice.get(dest)
                                && target_ir != u32::MAX
                            {
                                ip = target_ir as usize;
                                continue;
                            }
                        }
                        self.current_call_frame.pc = (op.data as usize) + 1;
                        break IrExit::Error(ExceptionalHalt::InvalidJump.into());
                    }

                    IrTag::JumpiDyn => {
                        let dest_word = unsafe { *stack_ptr.add(sp) };
                        let cond = unsafe { *stack_ptr.add(sp + 1) };
                        sp += 2;
                        if !cond.is_zero() {
                            if dest_word.0[1] == 0 && dest_word.0[2] == 0 && dest_word.0[3] == 0 {
                                let dest = dest_word.0[0] as usize;
                                if let Some(&target_ir) = jumpdest_slice.get(dest)
                                    && target_ir != u32::MAX
                                {
                                    ip = target_ir as usize;
                                    continue;
                                }
                            }
                            self.current_call_frame.pc = (op.data as usize) + 1;
                            break IrExit::Error(ExceptionalHalt::InvalidJump.into());
                        }
                    }

                    // Fused superinstructions
                    IrTag::Dup1Push4EqJumpi => {
                        let top = unsafe { *stack_ptr.add(sp) };
                        if top.0[0] == (op.data as u64)
                            && top.0[1] == 0
                            && top.0[2] == 0
                            && top.0[3] == 0
                        {
                            ip = op.aux2 as usize;
                        }
                    }

                    IrTag::IsZeroJumpi => {
                        let v = unsafe { *stack_ptr.add(sp) };
                        sp += 1;
                        if v.is_zero() {
                            ip = op.data as usize;
                        }
                    }

                    IrTag::EqJumpi => {
                        let a = unsafe { *stack_ptr.add(sp) };
                        let b = unsafe { *stack_ptr.add(sp + 1) };
                        sp += 2;
                        if a == b {
                            ip = op.data as usize;
                        }
                    }

                    IrTag::LtJumpi => {
                        let a = unsafe { *stack_ptr.add(sp) };
                        let b = unsafe { *stack_ptr.add(sp + 1) };
                        sp += 2;
                        if a < b {
                            ip = op.data as usize;
                        }
                    }

                    IrTag::GtJumpi => {
                        let a = unsafe { *stack_ptr.add(sp) };
                        let b = unsafe { *stack_ptr.add(sp + 1) };
                        sp += 2;
                        if a > b {
                            ip = op.data as usize;
                        }
                    }

                    IrTag::PushU32Add => unsafe {
                        let slot = &mut *stack_ptr.add(sp);
                        *slot = slot.overflowing_add(U256::from(op.data as u64)).0;
                    },

                    IrTag::PushU8Shr => unsafe {
                        let slot = &mut *stack_ptr.add(sp);
                        *slot = shift_right(slot, op.aux, 0);
                    },

                    IrTag::PushU8Shl => unsafe {
                        let slot = &mut *stack_ptr.add(sp);
                        *slot = shift_left(slot, op.aux);
                    },

                    IrTag::Swap1Pop => {
                        unsafe {
                            *stack_ptr.add(sp + 1) = *stack_ptr.add(sp);
                        }
                        sp += 1;
                    }

                    // Dynamic-gas warm-memory fast paths
                    IrTag::MLoad => {
                        if sp < STACK_LIMIT {
                            let offset_word = unsafe { *stack_ptr.add(sp) };
                            let mem_len = self.current_call_frame.memory.len();
                            if offset_word.0[1] == 0
                                && offset_word.0[2] == 0
                                && offset_word.0[3] == 0
                                && (offset_word.0[0] as usize).saturating_add(32) <= mem_len
                            {
                                gas -= 3;
                                if gas < 0 {
                                    self.current_call_frame.pc = op.data as usize;
                                    break IrExit::Error(ExceptionalHalt::OutOfGas.into());
                                }
                                let offset = offset_word.0[0] as usize;
                                if let Ok(word) = self
                                    .current_call_frame
                                    .memory
                                    .load_word_prerounded(offset, mem_len)
                                {
                                    unsafe {
                                        *stack_ptr.add(sp) = word;
                                    }
                                    continue;
                                }
                            }
                        }
                        self.current_call_frame.pc = op.data as usize;
                        self.current_call_frame.gas_remaining = gas;
                        self.current_call_frame.stack.offset = sp;
                        let op_result = opcode_table[0x51].call(self, &mut error);
                        if let OpcodeResult::Halt = op_result {
                            match error.take() {
                                None => break IrExit::Stop,
                                Some(err) => break IrExit::Error(err),
                            }
                        }
                        gas = self.current_call_frame.gas_remaining;
                        sp = self.current_call_frame.stack.offset;
                    }

                    IrTag::MStore => {
                        if !debug_enabled && sp + 1 < STACK_LIMIT {
                            let offset_word = unsafe { *stack_ptr.add(sp) };
                            let mem_len = self.current_call_frame.memory.len();
                            if offset_word.0[1] == 0
                                && offset_word.0[2] == 0
                                && offset_word.0[3] == 0
                                && (offset_word.0[0] as usize).saturating_add(32) <= mem_len
                            {
                                gas -= 3;
                                if gas < 0 {
                                    self.current_call_frame.pc = op.data as usize;
                                    break IrExit::Error(ExceptionalHalt::OutOfGas.into());
                                }
                                let offset = offset_word.0[0] as usize;
                                let value = unsafe { *stack_ptr.add(sp + 1) };
                                if self
                                    .current_call_frame
                                    .memory
                                    .store_word_prerounded(offset, value, mem_len)
                                    .is_ok()
                                {
                                    sp += 2;
                                    continue;
                                }
                            }
                        }
                        self.current_call_frame.pc = op.data as usize;
                        self.current_call_frame.gas_remaining = gas;
                        self.current_call_frame.stack.offset = sp;
                        let op_result = opcode_table[0x52].call(self, &mut error);
                        if let OpcodeResult::Halt = op_result {
                            match error.take() {
                                None => break IrExit::Stop,
                                Some(err) => break IrExit::Error(err),
                            }
                        }
                        gas = self.current_call_frame.gas_remaining;
                        sp = self.current_call_frame.stack.offset;
                    }

                    IrTag::Fallback => {
                        self.current_call_frame.pc = op.data as usize;
                        self.current_call_frame.gas_remaining = gas;
                        self.current_call_frame.stack.offset = sp;
                        let op_result = opcode_table[op.aux as usize].call(self, &mut error);
                        match op_result {
                            OpcodeResult::Continue => {
                                if self.call_frames.len() != entry_depth {
                                    break IrExit::ReloadFrame;
                                }
                                gas = self.current_call_frame.gas_remaining;
                                sp = self.current_call_frame.stack.offset;
                            }
                            OpcodeResult::Halt => match error.take() {
                                None => break IrExit::Stop,
                                Some(err) => break IrExit::Error(err),
                            },
                        }
                    }
                }
            };

            match exit {
                IrExit::ReloadFrame => continue,
                IrExit::FallbackAtPc(evm_pc) => {
                    self.current_call_frame.pc = evm_pc;
                    self.current_call_frame.gas_remaining = gas;
                    self.current_call_frame.stack.offset = sp;
                    return self.run_dispatch_fast();
                }
                IrExit::Stop => {
                    self.current_call_frame.gas_remaining = gas;
                    self.current_call_frame.stack.offset = sp;
                    let result = self.handle_opcode_result()?;
                    if self.is_initial_call_frame() {
                        self.handle_state_backup(&result, !self.preserve_top_level_backup)?;
                        return Ok(result);
                    }
                    self.handle_return(&result)?;
                }
                IrExit::Error(err) => {
                    self.current_call_frame.gas_remaining = gas;
                    self.current_call_frame.stack.offset = sp;
                    let result = self.handle_opcode_error(err)?;
                    if self.is_initial_call_frame() {
                        self.handle_state_backup(&result, !self.preserve_top_level_backup)?;
                        return Ok(result);
                    }
                    self.handle_return(&result)?;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use ethrex_crypto::NativeCrypto;

    #[test]
    fn compiles_solidity_patterns_and_computes_exact_block_bounds() {
        // DUP1; PUSH4 0xa9059cbb; EQ; PUSH2 0x000d; JUMPI; STOP; JUMPDEST; PUSH1 4; ADD; STOP
        let bytecode = Bytes::from_static(&[
            0x80, 0x63, 0xa9, 0x05, 0x9c, 0xbb, 0x14, 0x61, 0x00, 0x0c, 0x57, 0x00, 0x5b, 0x60,
            0x04, 0x01, 0x00,
        ]);
        let code = Code::from_bytecode(bytecode, &NativeCrypto);
        let compiled = compile_bytecode(&code, true);

        // First block must contain BlockCheck + fused Dup1Push4EqJumpi targeting the JUMPDEST at pc=12.
        assert_eq!(compiled.ops[0].tag, IrTag::BlockCheck);
        assert_eq!(compiled.ops[1].tag, IrTag::Dup1Push4EqJumpi);
        assert_eq!(compiled.ops[1].data, 0xa905_9cbb);
        let jd_ir = compiled.jumpdest_to_ir[12];
        assert_ne!(jd_ir, u32::MAX);
        assert_eq!(u32::from(compiled.ops[1].aux2), jd_ir);

        // First block gas = DUP1(3)+PUSH4(3)+EQ(3)+PUSH2(3)+JUMPI(10) = 22, min_stack=1, max_growth=2.
        assert_eq!(compiled.blocks[0].gas, 22);
        assert_eq!(compiled.blocks[0].min_stack, 1);
        assert_eq!(compiled.blocks[0].max_growth, 2);

        // Second block at JUMPDEST (pc=12): JUMPDEST(1) + PUSH1(3) + ADD(3) = 7, fused PushU32Add.
        let jd_idx = usize::try_from(jd_ir).unwrap_or(0);
        assert_eq!(compiled.ops[jd_idx].tag, IrTag::BlockCheck);
        assert_eq!(compiled.ops[jd_idx + 1].tag, IrTag::PushU32Add);
        assert_eq!(compiled.ops[jd_idx + 1].data, 4);
        let b1 = usize::try_from(compiled.ops[jd_idx].data).unwrap_or(0);
        assert_eq!(compiled.blocks[b1].gas, 7);
        assert_eq!(compiled.blocks[b1].min_stack, 1);
        assert_eq!(compiled.blocks[b1].max_growth, 1);
    }

    #[test]
    fn compiles_comparison_and_shift_fusions() {
        // PUSH1 0xE0; SHR; ISZERO; PUSH1 0x09; JUMPI; SWAP1; POP; STOP; STOP; JUMPDEST; STOP
        let bytecode = Bytes::from_static(&[
            0x60, 0xe0, 0x1c, 0x15, 0x60, 0x09, 0x57, 0x90, 0x50, 0x5b, 0x00,
        ]);
        let code = Code::from_bytecode(bytecode, &NativeCrypto);
        let compiled = compile_bytecode(&code, true);
        assert_eq!(compiled.ops[1].tag, IrTag::PushU8Shr);
        assert_eq!(compiled.ops[1].aux, 0xe0);
        assert_eq!(compiled.ops[2].tag, IrTag::IsZeroJumpi);
        assert_eq!(compiled.ops[2].data, compiled.jumpdest_to_ir[9]);
        // Gas for block 0: PUSH1(3)+SHR(3)+ISZERO(3)+PUSH1(3)+JUMPI(10) = 22
        assert_eq!(compiled.blocks[0].gas, 22);
    }
}
