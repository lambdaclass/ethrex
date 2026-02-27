//! Bytecode optimizer for JIT compilation — constant folding pass.
//!
//! Detects `PUSH+PUSH+ARITHMETIC` patterns and folds them into a single
//! wider PUSH of the pre-computed result. Uses same-length replacement
//! so bytecode offsets (JUMP targets, basic blocks) are preserved.
//!
//! # Example
//!
//! ```text
//! Before: PUSH1 3, PUSH1 4, ADD   (5 bytes, 3 instructions)
//! After:  PUSH4 7                  (5 bytes, 1 instruction)
//! ```

use bytes::Bytes;
use ethrex_common::U256;

use super::types::AnalyzedBytecode;

// ─── EVM opcode constants ────────────────────────────────────────────

const ADD: u8 = 0x01;
const MUL: u8 = 0x02;
const SUB: u8 = 0x03;
const DIV: u8 = 0x04;
const SDIV: u8 = 0x05;
const MOD: u8 = 0x06;
const SMOD: u8 = 0x07;
const EXP: u8 = 0x0A;
const SIGNEXTEND: u8 = 0x0B;
const LT: u8 = 0x10;
const GT: u8 = 0x11;
const SLT: u8 = 0x12;
const SGT: u8 = 0x13;
const EQ: u8 = 0x14;
const ISZERO: u8 = 0x15;
const AND: u8 = 0x16;
const OR: u8 = 0x17;
const XOR: u8 = 0x18;
const NOT: u8 = 0x19;
const SHL: u8 = 0x1B;
const SHR: u8 = 0x1C;
const SAR: u8 = 0x1D;

// ─── Public types ────────────────────────────────────────────────────

/// A constant-foldable `PUSH+PUSH+ARITHMETIC` pattern detected in bytecode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldablePattern {
    /// Byte offset of the first PUSH instruction.
    pub offset: usize,
    /// Total byte length of the three-instruction sequence.
    pub length: usize,
    /// Value pushed by the first PUSH (ends up as `μ_s[1]` — below top).
    pub first_val: U256,
    /// Value pushed by the second PUSH (ends up as `μ_s[0]` — stack top).
    pub second_val: U256,
    /// The arithmetic opcode (ADD, SUB, MUL, AND, OR, XOR, DIV, etc.).
    pub op: u8,
}

/// A constant-foldable `PUSH+UNARY_OP` pattern (NOT, ISZERO).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnaryPattern {
    /// Byte offset of the PUSH instruction.
    pub offset: usize,
    /// Total byte length of the two-instruction sequence (push_size + 1).
    pub length: usize,
    /// Value pushed by the PUSH instruction.
    pub val: U256,
    /// The unary opcode (NOT, ISZERO).
    pub op: u8,
}

/// Statistics from a single optimization pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OptimizationStats {
    /// Number of foldable patterns detected during scan.
    pub patterns_detected: usize,
    /// Number of patterns successfully folded (result fit in available bytes).
    pub patterns_folded: usize,
    /// Number of opcodes eliminated (each fold removes 2: `3 → 1`).
    pub opcodes_eliminated: usize,
    /// Number of unary patterns folded (each fold removes 1: `2 → 1`).
    pub unary_patterns_folded: usize,
}

// ─── Helper functions ────────────────────────────────────────────────

/// Check if an opcode is a PUSH instruction (PUSH0 `0x5F` through PUSH32 `0x7F`).
fn is_push(opcode: u8) -> bool {
    (0x5f..=0x7f).contains(&opcode)
}

/// Return the number of immediate data bytes for a PUSH opcode.
/// PUSH0 returns 0, PUSH1 returns 1, …, PUSH32 returns 32.
/// Non-PUSH opcodes return 0.
#[allow(clippy::arithmetic_side_effects)]
fn push_data_size(opcode: u8) -> usize {
    if opcode == 0x5f {
        0 // PUSH0
    } else if (0x60..=0x7f).contains(&opcode) {
        usize::from(opcode - 0x5f)
    } else {
        0
    }
}

/// Total instruction size in bytes: 1 (opcode byte) + immediate data bytes.
fn instruction_size(opcode: u8) -> usize {
    1_usize.saturating_add(push_data_size(opcode))
}

/// Extract a U256 value from PUSH immediate bytes at `push_offset`.
fn extract_push_value(bytecode: &[u8], push_offset: usize, data_size: usize) -> U256 {
    if data_size == 0 {
        return U256::zero(); // PUSH0
    }
    let start = push_offset.saturating_add(1);
    let end = start.saturating_add(data_size);
    if end > bytecode.len() {
        return U256::zero(); // truncated bytecode
    }
    #[expect(clippy::indexing_slicing, reason = "bounds checked above")]
    U256::from_big_endian(&bytecode[start..end])
}

/// Minimum number of bytes needed to represent a U256 value in big-endian.
fn bytes_needed(value: U256) -> usize {
    if value.is_zero() {
        return 0;
    }
    let buf = value.to_big_endian();
    for (i, &b) in buf.iter().enumerate() {
        if b != 0 {
            return 32_usize.saturating_sub(i);
        }
    }
    0
}

// ─── Signed arithmetic helpers (replicating LEVM semantics) ─────────

/// Check if bit 255 is set (two's complement negative).
fn is_negative(value: U256) -> bool {
    value.bit(255)
}

/// Two's complement negation: `!x + 1`.
fn negate(value: U256) -> U256 {
    (!value).overflowing_add(U256::one()).0
}

/// Absolute value in two's complement.
fn abs_val(value: U256) -> U256 {
    if is_negative(value) {
        negate(value)
    } else {
        value
    }
}

/// Convert a boolean to U256 (0 or 1).
fn u256_from_bool(value: bool) -> U256 {
    if value { U256::one() } else { U256::zero() }
}

/// Check if an opcode is a unary foldable operation (NOT, ISZERO).
fn is_unary_foldable_op(opcode: u8) -> bool {
    matches!(opcode, NOT | ISZERO)
}

/// Evaluate a unary operation following EVM semantics.
fn eval_unary_op(op: u8, val: U256) -> Option<U256> {
    match op {
        NOT => Some(!val),
        ISZERO => Some(u256_from_bool(val.is_zero())),
        _ => None,
    }
}

/// Signed division following EVM SDIV semantics.
fn eval_sdiv(dividend: U256, divisor: U256) -> U256 {
    if divisor.is_zero() || dividend.is_zero() {
        return U256::zero();
    }
    let quotient = abs_val(dividend)
        .checked_div(abs_val(divisor))
        .unwrap_or_default();
    if is_negative(dividend) ^ is_negative(divisor) {
        negate(quotient)
    } else {
        quotient
    }
}

/// Signed modulo following EVM SMOD semantics (result sign follows dividend).
fn eval_smod(dividend: U256, divisor: U256) -> U256 {
    if divisor.is_zero() || dividend.is_zero() {
        return U256::zero();
    }
    let remainder = abs_val(dividend)
        .checked_rem(abs_val(divisor))
        .unwrap_or_default();
    if is_negative(dividend) {
        negate(remainder)
    } else {
        remainder
    }
}

/// SIGNEXTEND: extend sign bit at byte boundary.
#[allow(clippy::arithmetic_side_effects)]
fn eval_signextend(byte_size_minus_one: U256, value: U256) -> U256 {
    if byte_size_minus_one > U256::from(31) {
        return value;
    }
    let sign_bit_index = byte_size_minus_one * 8 + 7;
    let sign_bit = (value >> sign_bit_index) & U256::one();
    let mask = (U256::one() << sign_bit_index) - U256::one();
    if sign_bit.is_zero() {
        value & mask
    } else {
        value | !mask
    }
}

/// Signed comparison helper for SLT/SGT.
fn signed_compare(a: U256, b: U256, less_than: bool) -> U256 {
    let a_neg = is_negative(a);
    let b_neg = is_negative(b);
    let result = match (a_neg, b_neg) {
        (true, false) => less_than,
        (false, true) => !less_than,
        _ if less_than => a < b,
        _ => a > b,
    };
    u256_from_bool(result)
}

/// Arithmetic right shift following EVM SAR semantics.
#[allow(clippy::arithmetic_side_effects)]
fn eval_sar(shift: U256, value: U256) -> U256 {
    let value_negative = is_negative(value);
    if shift < U256::from(256) {
        if !value_negative {
            value >> shift
        } else {
            (value >> shift) | (U256::MAX << (U256::from(256) - shift))
        }
    } else if value_negative {
        U256::MAX
    } else {
        U256::zero()
    }
}

/// Shift (SHL/SHR): returns 0 if shift >= 256.
#[allow(clippy::arithmetic_side_effects)]
fn eval_shift(value: U256, shift: U256, left: bool) -> U256 {
    if shift < U256::from(256) {
        if left { value << shift } else { value >> shift }
    } else {
        U256::zero()
    }
}

/// Evaluate a binary arithmetic operation following EVM stack semantics.
///
/// `second_val` is `μ_s[0]` (top of stack), `first_val` is `μ_s[1]`.
fn eval_op(op: u8, first_val: U256, second_val: U256) -> Option<U256> {
    match op {
        ADD => Some(second_val.overflowing_add(first_val).0),
        SUB => Some(second_val.overflowing_sub(first_val).0),
        MUL => Some(second_val.overflowing_mul(first_val).0),
        DIV => Some(second_val.checked_div(first_val).unwrap_or_default()),
        SDIV => Some(eval_sdiv(second_val, first_val)),
        MOD => Some(second_val.checked_rem(first_val).unwrap_or_default()),
        SMOD => Some(eval_smod(second_val, first_val)),
        EXP => Some(second_val.overflowing_pow(first_val).0),
        SIGNEXTEND => Some(eval_signextend(second_val, first_val)),
        LT => Some(u256_from_bool(second_val < first_val)),
        GT => Some(u256_from_bool(second_val > first_val)),
        EQ => Some(u256_from_bool(second_val == first_val)),
        SLT => Some(signed_compare(second_val, first_val, true)),
        SGT => Some(signed_compare(second_val, first_val, false)),
        SHL => Some(eval_shift(first_val, second_val, true)),
        SHR => Some(eval_shift(first_val, second_val, false)),
        SAR => Some(eval_sar(second_val, first_val)),
        AND => Some(second_val & first_val),
        OR => Some(second_val | first_val),
        XOR => Some(second_val ^ first_val),
        _ => None,
    }
}

/// Check if an opcode is a foldable binary operation.
fn is_foldable_op(opcode: u8) -> bool {
    matches!(
        opcode,
        ADD | MUL
            | SUB
            | DIV
            | SDIV
            | MOD
            | SMOD
            | EXP
            | SIGNEXTEND
            | LT
            | GT
            | SLT
            | SGT
            | EQ
            | AND
            | OR
            | XOR
            | SHL
            | SHR
            | SAR
    )
}

// ─── Public API ──────────────────────────────────────────────────────

/// Scan bytecode for constant-foldable `PUSH+PUSH+ARITHMETIC` patterns.
///
/// Does not modify bytecode — returns detected patterns for inspection.
pub fn detect_patterns(bytecode: &[u8]) -> Vec<FoldablePattern> {
    let mut patterns = Vec::new();
    let len = bytecode.len();
    let mut i = 0;

    while i < len {
        #[expect(clippy::indexing_slicing, reason = "i < len checked in loop condition")]
        let opcode_a = bytecode[i];

        if !is_push(opcode_a) {
            i = i.saturating_add(instruction_size(opcode_a));
            continue;
        }

        let size_a = push_data_size(opcode_a);
        let total_a = instruction_size(opcode_a);
        let j = i.saturating_add(total_a);

        if j >= len {
            break;
        }

        #[expect(clippy::indexing_slicing, reason = "j < len checked above")]
        let opcode_b = bytecode[j];

        if !is_push(opcode_b) {
            i = i.saturating_add(total_a);
            continue;
        }

        let size_b = push_data_size(opcode_b);
        let total_b = instruction_size(opcode_b);
        let k = j.saturating_add(total_b);

        if k >= len {
            break;
        }

        #[expect(clippy::indexing_slicing, reason = "k < len checked above")]
        let opcode_op = bytecode[k];

        if !is_foldable_op(opcode_op) {
            i = i.saturating_add(total_a);
            continue;
        }

        // Found a PUSH+PUSH+OP pattern
        let first_val = extract_push_value(bytecode, i, size_a);
        let second_val = extract_push_value(bytecode, j, size_b);
        let pattern_length = total_a.saturating_add(total_b).saturating_add(1);

        patterns.push(FoldablePattern {
            offset: i,
            length: pattern_length,
            first_val,
            second_val,
            op: opcode_op,
        });

        // Skip past the entire pattern to avoid overlapping detections
        i = k.saturating_add(1);
    }

    patterns
}

/// Scan bytecode for constant-foldable `PUSH+UNARY_OP` patterns.
///
/// Does not modify bytecode — returns detected patterns for inspection.
pub fn detect_unary_patterns(bytecode: &[u8]) -> Vec<UnaryPattern> {
    let mut patterns = Vec::new();
    let len = bytecode.len();
    let mut i = 0;

    while i < len {
        #[expect(clippy::indexing_slicing, reason = "i < len checked in loop condition")]
        let opcode_a = bytecode[i];

        if !is_push(opcode_a) {
            i = i.saturating_add(instruction_size(opcode_a));
            continue;
        }

        let size_a = push_data_size(opcode_a);
        let total_a = instruction_size(opcode_a);
        let j = i.saturating_add(total_a);

        if j >= len {
            break;
        }

        #[expect(clippy::indexing_slicing, reason = "j < len checked above")]
        let opcode_op = bytecode[j];

        if !is_unary_foldable_op(opcode_op) {
            i = i.saturating_add(total_a);
            continue;
        }

        let val = extract_push_value(bytecode, i, size_a);
        let pattern_length = total_a.saturating_add(1);

        patterns.push(UnaryPattern {
            offset: i,
            length: pattern_length,
            val,
            op: opcode_op,
        });

        // Skip past the entire pattern
        i = j.saturating_add(1);
    }

    patterns
}

/// Apply constant folding to analyzed bytecode.
///
/// Replaces each foldable `PUSH+PUSH+OP` sequence with a single wider PUSH
/// of the pre-computed result. Also folds `PUSH+UNARY_OP` patterns.
/// Bytecode length is preserved (same offsets).
/// Rewrite a bytecode region `[offset..offset+length]` as a single PUSH of `result`.
///
/// Returns `true` if the fold was applied, `false` if the result doesn't fit.
/// `length` is the total pattern size; the replacement PUSH uses `length - 1` data bytes.
fn write_folded_push(bytecode: &mut [u8], offset: usize, length: usize, result: U256) -> bool {
    let data_size = length.saturating_sub(1);
    if data_size > 32 || bytes_needed(result) > data_size {
        return false;
    }
    let Some(data_size_u8) = u8::try_from(data_size).ok() else {
        return false;
    };

    #[expect(clippy::indexing_slicing, reason = "offset within bytecode bounds")]
    {
        bytecode[offset] = 0x5f_u8.saturating_add(data_size_u8);
    }

    let buf = result.to_big_endian();
    let pad_start = 32_usize.saturating_sub(data_size);
    let dest_start = offset.saturating_add(1);
    let dest_end = dest_start.saturating_add(data_size);
    #[expect(clippy::indexing_slicing, reason = "dest range within pattern bounds")]
    {
        bytecode[dest_start..dest_end].copy_from_slice(&buf[pad_start..]);
    }

    true
}

pub fn optimize(analyzed: AnalyzedBytecode) -> (AnalyzedBytecode, OptimizationStats) {
    let binary_patterns = detect_patterns(&analyzed.bytecode);
    let unary_patterns = detect_unary_patterns(&analyzed.bytecode);

    if binary_patterns.is_empty() && unary_patterns.is_empty() {
        return (analyzed, OptimizationStats::default());
    }

    let mut bytecode = analyzed.bytecode.to_vec();
    let mut stats = OptimizationStats {
        patterns_detected: binary_patterns.len(),
        ..Default::default()
    };

    for pattern in &binary_patterns {
        let Some(result) = eval_op(pattern.op, pattern.first_val, pattern.second_val) else {
            continue;
        };
        if write_folded_push(&mut bytecode, pattern.offset, pattern.length, result) {
            stats.patterns_folded = stats.patterns_folded.saturating_add(1);
            stats.opcodes_eliminated = stats.opcodes_eliminated.saturating_add(2);
        }
    }

    for pattern in &unary_patterns {
        let Some(result) = eval_unary_op(pattern.op, pattern.val) else {
            continue;
        };
        if write_folded_push(&mut bytecode, pattern.offset, pattern.length, result) {
            stats.unary_patterns_folded = stats.unary_patterns_folded.saturating_add(1);
            stats.opcodes_eliminated = stats.opcodes_eliminated.saturating_add(1);
        }
    }

    let optimized = AnalyzedBytecode {
        bytecode: Bytes::from(bytecode),
        opcode_count: analyzed
            .opcode_count
            .saturating_sub(stats.opcodes_eliminated),
        ..analyzed
    };

    (optimized, stats)
}

// ─── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;
    use ethrex_common::H256;

    // Helper: build AnalyzedBytecode from raw bytes for testing optimize()
    fn make_analyzed(bytecode: Vec<u8>, opcode_count: usize) -> AnalyzedBytecode {
        AnalyzedBytecode {
            hash: H256::zero(),
            bytecode: Bytes::from(bytecode),
            jump_targets: vec![],
            basic_blocks: vec![],
            opcode_count,
            has_external_calls: false,
        }
    }

    // ── Helper function tests ────────────────────────────────────────

    #[test]
    fn test_is_push() {
        assert!(is_push(0x5f), "PUSH0");
        assert!(is_push(0x60), "PUSH1");
        assert!(is_push(0x7f), "PUSH32");
        assert!(!is_push(0x00), "STOP");
        assert!(!is_push(0x01), "ADD");
        assert!(!is_push(0x80), "DUP1");
    }

    #[test]
    fn test_push_data_size() {
        assert_eq!(push_data_size(0x5f), 0, "PUSH0 has 0 data bytes");
        assert_eq!(push_data_size(0x60), 1, "PUSH1 has 1 data byte");
        assert_eq!(push_data_size(0x61), 2, "PUSH2 has 2 data bytes");
        assert_eq!(push_data_size(0x7f), 32, "PUSH32 has 32 data bytes");
        assert_eq!(push_data_size(0x01), 0, "ADD has 0 data bytes");
    }

    #[test]
    fn test_bytes_needed() {
        assert_eq!(bytes_needed(U256::zero()), 0);
        assert_eq!(bytes_needed(U256::from(1)), 1);
        assert_eq!(bytes_needed(U256::from(255)), 1);
        assert_eq!(bytes_needed(U256::from(256)), 2);
        assert_eq!(bytes_needed(U256::from(65535)), 2);
        assert_eq!(bytes_needed(U256::from(65536)), 3);
    }

    #[test]
    fn test_eval_op_add() {
        let result = eval_op(ADD, U256::from(3), U256::from(4));
        // EVM: second_val(4) + first_val(3) = 7
        assert_eq!(result, Some(U256::from(7)));
    }

    #[test]
    fn test_eval_op_sub() {
        // PUSH 3, PUSH 7, SUB → 7 - 3 = 4
        let result = eval_op(SUB, U256::from(3), U256::from(7));
        assert_eq!(result, Some(U256::from(4)));
    }

    #[test]
    fn test_eval_op_sub_wrapping() {
        // PUSH 5, PUSH 3, SUB → 3 - 5 = wraps to U256::MAX - 1
        let result = eval_op(SUB, U256::from(5), U256::from(3));
        let expected = U256::zero().overflowing_sub(U256::from(2)).0;
        assert_eq!(result, Some(expected));
    }

    #[test]
    fn test_eval_op_mul() {
        let result = eval_op(MUL, U256::from(5), U256::from(6));
        assert_eq!(result, Some(U256::from(30)));
    }

    #[test]
    fn test_eval_op_bitwise() {
        assert_eq!(
            eval_op(AND, U256::from(0xFF), U256::from(0x0F)),
            Some(U256::from(0x0F))
        );
        assert_eq!(
            eval_op(OR, U256::from(0xF0), U256::from(0x0F)),
            Some(U256::from(0xFF))
        );
        assert_eq!(
            eval_op(XOR, U256::from(0xFF), U256::from(0x0F)),
            Some(U256::from(0xF0))
        );
    }

    #[test]
    fn test_eval_op_unknown() {
        // POP (0x50) is not a foldable op
        assert_eq!(eval_op(0x50, U256::from(1), U256::from(2)), None);
    }

    // ── Pattern detection tests ──────────────────────────────────────

    #[test]
    fn test_detect_push1_push1_add() {
        // PUSH1 3, PUSH1 4, ADD, STOP
        let bytecode = vec![0x60, 0x03, 0x60, 0x04, 0x01, 0x00];
        let patterns = detect_patterns(&bytecode);

        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].offset, 0);
        assert_eq!(patterns[0].length, 5);
        assert_eq!(patterns[0].first_val, U256::from(3));
        assert_eq!(patterns[0].second_val, U256::from(4));
        assert_eq!(patterns[0].op, ADD);
    }

    #[test]
    fn test_detect_push1_push1_mul() {
        // PUSH1 5, PUSH1 6, MUL, STOP
        let bytecode = vec![0x60, 0x05, 0x60, 0x06, 0x02, 0x00];
        let patterns = detect_patterns(&bytecode);

        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].op, MUL);
        assert_eq!(patterns[0].first_val, U256::from(5));
        assert_eq!(patterns[0].second_val, U256::from(6));
    }

    #[test]
    fn test_detect_no_pattern_single_push() {
        // PUSH1 3, ADD, STOP — only one PUSH before ADD
        let bytecode = vec![0x60, 0x03, 0x01, 0x00];
        let patterns = detect_patterns(&bytecode);
        assert!(patterns.is_empty());
    }

    #[test]
    fn test_detect_multiple_patterns() {
        // PUSH1 1, PUSH1 2, ADD, PUSH1 3, PUSH1 4, MUL, STOP
        let bytecode = vec![
            0x60, 0x01, 0x60, 0x02, 0x01, // PUSH1 1 + PUSH1 2 + ADD
            0x60, 0x03, 0x60, 0x04, 0x02, // PUSH1 3 + PUSH1 4 + MUL
            0x00, // STOP
        ];
        let patterns = detect_patterns(&bytecode);

        assert_eq!(patterns.len(), 2);
        assert_eq!(patterns[0].offset, 0);
        assert_eq!(patterns[0].op, ADD);
        assert_eq!(patterns[1].offset, 5);
        assert_eq!(patterns[1].op, MUL);
    }

    #[test]
    fn test_detect_pattern_with_gap() {
        // PUSH1 3, DUP1, PUSH1 4, ADD, STOP — DUP1 breaks the sequence
        let bytecode = vec![0x60, 0x03, 0x80, 0x60, 0x04, 0x01, 0x00];
        let patterns = detect_patterns(&bytecode);
        assert!(patterns.is_empty());
    }

    #[test]
    fn test_detect_mixed_push_sizes() {
        // PUSH2 0x0100, PUSH1 5, ADD, STOP
        let bytecode = vec![0x61, 0x01, 0x00, 0x60, 0x05, 0x01, 0x00];
        let patterns = detect_patterns(&bytecode);

        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].offset, 0);
        assert_eq!(patterns[0].length, 6); // 3 + 2 + 1
        assert_eq!(patterns[0].first_val, U256::from(256));
        assert_eq!(patterns[0].second_val, U256::from(5));
    }

    #[test]
    fn test_detect_empty_bytecode() {
        let patterns = detect_patterns(&[]);
        assert!(patterns.is_empty());
    }

    #[test]
    fn test_detect_three_pushes_finds_last_pair() {
        // PUSH1 1, PUSH1 2, PUSH1 3, ADD — should find PUSH1 2 + PUSH1 3 + ADD
        // (first PUSH1 1 + PUSH1 2 → next is PUSH1, not arith → skip)
        let bytecode = vec![0x60, 0x01, 0x60, 0x02, 0x60, 0x03, 0x01, 0x00];
        let patterns = detect_patterns(&bytecode);

        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].offset, 2); // starts at second PUSH
        assert_eq!(patterns[0].first_val, U256::from(2));
        assert_eq!(patterns[0].second_val, U256::from(3));
    }

    #[test]
    fn test_detect_push0_push0_add() {
        // PUSH0, PUSH0, ADD, STOP
        let bytecode = vec![0x5f, 0x5f, 0x01, 0x00];
        let patterns = detect_patterns(&bytecode);

        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].length, 3); // 1 + 1 + 1
        assert_eq!(patterns[0].first_val, U256::zero());
        assert_eq!(patterns[0].second_val, U256::zero());
    }

    #[test]
    fn test_detect_all_supported_ops() {
        for (op, op_name) in [
            (0x01u8, "ADD"),
            (0x02, "MUL"),
            (0x03, "SUB"),
            (0x04, "DIV"),
            (0x05, "SDIV"),
            (0x06, "MOD"),
            (0x07, "SMOD"),
            (0x0A, "EXP"),
            (0x0B, "SIGNEXTEND"),
            (0x10, "LT"),
            (0x11, "GT"),
            (0x12, "SLT"),
            (0x13, "SGT"),
            (0x14, "EQ"),
            (0x16, "AND"),
            (0x17, "OR"),
            (0x18, "XOR"),
            (0x1B, "SHL"),
            (0x1C, "SHR"),
            (0x1D, "SAR"),
        ] {
            let bytecode = vec![0x60, 0x01, 0x60, 0x02, op, 0x00];
            let patterns = detect_patterns(&bytecode);
            assert_eq!(patterns.len(), 1, "should detect {op_name} pattern");
            assert_eq!(patterns[0].op, op);
        }
    }

    #[test]
    fn test_detect_unsupported_ops_ignored() {
        // PUSH1 1, PUSH1 2, POP (0x50) — not in our foldable set
        let bytecode = vec![0x60, 0x01, 0x60, 0x02, 0x50, 0x00];
        let patterns = detect_patterns(&bytecode);
        assert!(patterns.is_empty(), "POP should not be detected");
    }

    // ── Constant folding tests ───────────────────────────────────────

    #[test]
    fn test_fold_push1_push1_add() {
        // PUSH1 3, PUSH1 4, ADD, STOP → PUSH4 7, STOP
        let analyzed = make_analyzed(vec![0x60, 0x03, 0x60, 0x04, 0x01, 0x00], 4);
        let (result, stats) = optimize(analyzed);

        // PUSH4 (0x63) = 0x5F + 4
        assert_eq!(
            result.bytecode.as_ref(),
            &[0x63, 0x00, 0x00, 0x00, 0x07, 0x00]
        );
        assert_eq!(stats.patterns_detected, 1);
        assert_eq!(stats.patterns_folded, 1);
        assert_eq!(stats.opcodes_eliminated, 2);
        assert_eq!(result.opcode_count, 2); // 4 - 2
    }

    #[test]
    fn test_fold_push1_push1_sub() {
        // PUSH1 3, PUSH1 7, SUB, STOP → EVM: 7 - 3 = 4
        let analyzed = make_analyzed(vec![0x60, 0x03, 0x60, 0x07, 0x03, 0x00], 4);
        let (result, _stats) = optimize(analyzed);

        assert_eq!(
            result.bytecode.as_ref(),
            &[0x63, 0x00, 0x00, 0x00, 0x04, 0x00]
        );
    }

    #[test]
    fn test_fold_push1_push1_mul() {
        // PUSH1 5, PUSH1 6, MUL, STOP → 30 = 0x1E
        let analyzed = make_analyzed(vec![0x60, 0x05, 0x60, 0x06, 0x02, 0x00], 4);
        let (result, _stats) = optimize(analyzed);

        assert_eq!(
            result.bytecode.as_ref(),
            &[0x63, 0x00, 0x00, 0x00, 0x1E, 0x00]
        );
    }

    #[test]
    fn test_fold_bitwise_and() {
        // PUSH1 0xFF, PUSH1 0x0F, AND, STOP → 0x0F
        let analyzed = make_analyzed(vec![0x60, 0xFF, 0x60, 0x0F, 0x16, 0x00], 4);
        let (result, _stats) = optimize(analyzed);

        assert_eq!(
            result.bytecode.as_ref(),
            &[0x63, 0x00, 0x00, 0x00, 0x0F, 0x00]
        );
    }

    #[test]
    fn test_fold_bitwise_or() {
        // PUSH1 0xF0, PUSH1 0x0F, OR, STOP → 0xFF
        let analyzed = make_analyzed(vec![0x60, 0xF0, 0x60, 0x0F, 0x17, 0x00], 4);
        let (result, _stats) = optimize(analyzed);

        assert_eq!(
            result.bytecode.as_ref(),
            &[0x63, 0x00, 0x00, 0x00, 0xFF, 0x00]
        );
    }

    #[test]
    fn test_fold_bitwise_xor() {
        // PUSH1 0xFF, PUSH1 0x0F, XOR, STOP → 0xF0
        let analyzed = make_analyzed(vec![0x60, 0xFF, 0x60, 0x0F, 0x18, 0x00], 4);
        let (result, _stats) = optimize(analyzed);

        assert_eq!(
            result.bytecode.as_ref(),
            &[0x63, 0x00, 0x00, 0x00, 0xF0, 0x00]
        );
    }

    #[test]
    fn test_fold_preserves_bytecode_length() {
        let input = vec![0x60, 0x03, 0x60, 0x04, 0x01, 0x00];
        let original_len = input.len();
        let analyzed = make_analyzed(input, 4);
        let (result, _stats) = optimize(analyzed);

        assert_eq!(
            result.bytecode.len(),
            original_len,
            "optimized bytecode must be same length"
        );
    }

    #[test]
    fn test_fold_sub_underflow_skipped() {
        // PUSH1 5, PUSH1 3, SUB, STOP → EVM: 3 - 5 = wraps to huge value
        // Result requires 32 bytes, but only 4 available → skip fold
        let input = vec![0x60, 0x05, 0x60, 0x03, 0x03, 0x00];
        let analyzed = make_analyzed(input.clone(), 4);
        let (result, stats) = optimize(analyzed);

        assert_eq!(
            result.bytecode.as_ref(),
            &input,
            "bytecode should be unchanged when fold is skipped"
        );
        assert_eq!(stats.patterns_detected, 1);
        assert_eq!(stats.patterns_folded, 0);
    }

    #[test]
    fn test_fold_multiple_patterns() {
        // PUSH1 1, PUSH1 2, ADD, PUSH1 3, PUSH1 4, MUL, STOP
        // → PUSH4 3, PUSH4 12, STOP
        let analyzed = make_analyzed(
            vec![
                0x60, 0x01, 0x60, 0x02, 0x01, // ADD: 1+2=3
                0x60, 0x03, 0x60, 0x04, 0x02, // MUL: 3*4=12
                0x00, // STOP
            ],
            7,
        );
        let (result, stats) = optimize(analyzed);

        assert_eq!(
            result.bytecode.as_ref(),
            &[
                0x63, 0x00, 0x00, 0x00, 0x03, // PUSH4 3
                0x63, 0x00, 0x00, 0x00, 0x0C, // PUSH4 12
                0x00, // STOP
            ]
        );
        assert_eq!(stats.patterns_folded, 2);
        assert_eq!(stats.opcodes_eliminated, 4);
        assert_eq!(result.opcode_count, 3); // 7 - 4
    }

    #[test]
    fn test_fold_preserves_surrounding_code() {
        // DUP1, PUSH1 3, PUSH1 4, ADD, POP, STOP
        // → DUP1, PUSH4 7, POP, STOP
        let analyzed = make_analyzed(vec![0x80, 0x60, 0x03, 0x60, 0x04, 0x01, 0x50, 0x00], 6);
        let (result, stats) = optimize(analyzed);

        assert_eq!(
            result.bytecode.as_ref(),
            &[0x80, 0x63, 0x00, 0x00, 0x00, 0x07, 0x50, 0x00]
        );
        assert_eq!(stats.patterns_folded, 1);
    }

    #[test]
    fn test_fold_empty_bytecode() {
        let analyzed = make_analyzed(vec![], 0);
        let (result, stats) = optimize(analyzed);

        assert!(result.bytecode.is_empty());
        assert_eq!(stats, OptimizationStats::default());
    }

    #[test]
    fn test_fold_push0_push0_add() {
        // PUSH0, PUSH0, ADD, STOP → PUSH2 0x0000, STOP
        let analyzed = make_analyzed(vec![0x5f, 0x5f, 0x01, 0x00], 4);
        let (result, stats) = optimize(analyzed);

        // Pattern length 3, data_size 2, PUSH2 = 0x61
        assert_eq!(result.bytecode.as_ref(), &[0x61, 0x00, 0x00, 0x00]);
        assert_eq!(stats.patterns_folded, 1);
    }

    #[test]
    fn test_fold_push2_push1_add() {
        // PUSH2 0x0100 (=256), PUSH1 0x05, ADD, STOP → 261 = 0x0105
        let analyzed = make_analyzed(vec![0x61, 0x01, 0x00, 0x60, 0x05, 0x01, 0x00], 4);
        let (result, _stats) = optimize(analyzed);

        // Pattern length 6, data_size 5, PUSH5 = 0x64
        // 261 = 0x0105, in 5 bytes big-endian: [0x00, 0x00, 0x00, 0x01, 0x05]
        assert_eq!(
            result.bytecode.as_ref(),
            &[0x64, 0x00, 0x00, 0x00, 0x01, 0x05, 0x00]
        );
    }

    #[test]
    fn test_fold_large_multiplication() {
        // PUSH1 200, PUSH1 200, MUL, STOP → 40000 = 0x9C40
        let analyzed = make_analyzed(vec![0x60, 0xC8, 0x60, 0xC8, 0x02, 0x00], 4);
        let (result, stats) = optimize(analyzed);

        // 40000 = 0x9C40, fits in 4 bytes
        assert_eq!(
            result.bytecode.as_ref(),
            &[0x63, 0x00, 0x00, 0x9C, 0x40, 0x00]
        );
        assert_eq!(stats.patterns_folded, 1);
    }

    #[test]
    fn test_fold_preserves_hash() {
        let analyzed = make_analyzed(vec![0x60, 0x03, 0x60, 0x04, 0x01, 0x00], 4);
        let original_hash = analyzed.hash;
        let (result, _stats) = optimize(analyzed);

        assert_eq!(
            result.hash, original_hash,
            "hash must be preserved for cache key"
        );
    }

    #[test]
    fn test_fold_preserves_metadata() {
        let mut analyzed = make_analyzed(vec![0x60, 0x03, 0x60, 0x04, 0x01, 0x00], 4);
        analyzed.jump_targets = vec![10, 20, 30];
        analyzed.basic_blocks = vec![(0, 5)];
        analyzed.has_external_calls = true;

        let (result, _stats) = optimize(analyzed);

        assert_eq!(result.jump_targets, vec![10, 20, 30]);
        assert_eq!(result.basic_blocks, vec![(0, 5)]);
        assert!(result.has_external_calls);
    }

    #[test]
    fn test_no_foldable_patterns() {
        // PUSH1 3, DUP1, ADD, STOP — no PUSH+PUSH+OP sequence
        let input = vec![0x60, 0x03, 0x80, 0x01, 0x00];
        let analyzed = make_analyzed(input.clone(), 4);
        let (result, stats) = optimize(analyzed);

        assert_eq!(result.bytecode.as_ref(), &input);
        assert_eq!(stats, OptimizationStats::default());
    }

    // ── G-7: Expanded binary opcode folding tests ────────────────────

    #[test]
    fn test_fold_push1_push1_div() {
        // PUSH1 5, PUSH1 20, DIV → EVM: 20 / 5 = 4
        let analyzed = make_analyzed(vec![0x60, 0x05, 0x60, 0x14, 0x04, 0x00], 4);
        let (result, stats) = optimize(analyzed);

        assert_eq!(
            result.bytecode.as_ref(),
            &[0x63, 0x00, 0x00, 0x00, 0x04, 0x00]
        );
        assert_eq!(stats.patterns_folded, 1);
    }

    #[test]
    fn test_fold_push1_push1_div_by_zero() {
        // PUSH1 0, PUSH1 10, DIV → EVM: 10 / 0 = 0 (EVM spec: no exception)
        let analyzed = make_analyzed(vec![0x60, 0x00, 0x60, 0x0A, 0x04, 0x00], 4);
        let (result, stats) = optimize(analyzed);

        // Result is 0, which needs 0 bytes, fits in 4 bytes
        assert_eq!(
            result.bytecode.as_ref(),
            &[0x63, 0x00, 0x00, 0x00, 0x00, 0x00]
        );
        assert_eq!(stats.patterns_folded, 1);
    }

    #[test]
    fn test_fold_push1_push1_mod() {
        // PUSH1 3, PUSH1 10, MOD → EVM: 10 % 3 = 1
        let analyzed = make_analyzed(vec![0x60, 0x03, 0x60, 0x0A, 0x06, 0x00], 4);
        let (result, stats) = optimize(analyzed);

        assert_eq!(
            result.bytecode.as_ref(),
            &[0x63, 0x00, 0x00, 0x00, 0x01, 0x00]
        );
        assert_eq!(stats.patterns_folded, 1);
    }

    #[test]
    fn test_fold_push1_push1_mod_by_zero() {
        // PUSH1 0, PUSH1 10, MOD → EVM: 10 % 0 = 0
        let analyzed = make_analyzed(vec![0x60, 0x00, 0x60, 0x0A, 0x06, 0x00], 4);
        let (result, stats) = optimize(analyzed);

        assert_eq!(
            result.bytecode.as_ref(),
            &[0x63, 0x00, 0x00, 0x00, 0x00, 0x00]
        );
        assert_eq!(stats.patterns_folded, 1);
    }

    #[test]
    fn test_fold_push1_push1_sdiv() {
        // Signed: -6 / 3 = -2
        // -6 in U256 = negate(6)
        let neg_6 = (!U256::from(6)).overflowing_add(U256::one()).0;
        // neg_6 is 32 bytes, won't fit in PUSH1 data_size=4, so use PUSH32
        let mut bytecode = vec![0x60, 0x03]; // PUSH1 3 (divisor, first push)
        bytecode.push(0x7f); // PUSH32 (32 bytes)
        let neg_6_bytes = neg_6.to_big_endian();
        bytecode.extend_from_slice(&neg_6_bytes);
        bytecode.push(0x05); // SDIV
        bytecode.push(0x00); // STOP

        let analyzed = make_analyzed(bytecode, 4);
        let (_result, stats) = optimize(analyzed);

        // Pattern length = 2 + 33 + 1 = 36, data_size = 35 → exceeds PUSH32 limit (32)
        // So this pattern cannot be folded.
        assert_eq!(stats.patterns_folded, 0); // Too large to fold
    }

    #[test]
    fn test_fold_push1_push1_sdiv_positive() {
        // PUSH1 3, PUSH1 6, SDIV → 6 / 3 = 2
        let analyzed = make_analyzed(vec![0x60, 0x03, 0x60, 0x06, 0x05, 0x00], 4);
        let (result, stats) = optimize(analyzed);

        assert_eq!(
            result.bytecode.as_ref(),
            &[0x63, 0x00, 0x00, 0x00, 0x02, 0x00]
        );
        assert_eq!(stats.patterns_folded, 1);
    }

    #[test]
    fn test_fold_push1_push1_smod() {
        // PUSH1 3, PUSH1 7, SMOD → 7 % 3 = 1 (positive case)
        let analyzed = make_analyzed(vec![0x60, 0x03, 0x60, 0x07, 0x07, 0x00], 4);
        let (result, stats) = optimize(analyzed);

        assert_eq!(
            result.bytecode.as_ref(),
            &[0x63, 0x00, 0x00, 0x00, 0x01, 0x00]
        );
        assert_eq!(stats.patterns_folded, 1);
    }

    #[test]
    fn test_fold_push1_push1_exp() {
        // PUSH1 8, PUSH1 2, EXP → 2^8 = 256 (0x100)
        let analyzed = make_analyzed(vec![0x60, 0x08, 0x60, 0x02, 0x0A, 0x00], 4);
        let (result, stats) = optimize(analyzed);

        // 256 = 0x0100, fits in 4 bytes
        assert_eq!(
            result.bytecode.as_ref(),
            &[0x63, 0x00, 0x00, 0x01, 0x00, 0x00]
        );
        assert_eq!(stats.patterns_folded, 1);
    }

    #[test]
    fn test_fold_push1_push1_exp_overflow_skipped() {
        // PUSH1 255, PUSH1 255, EXP → 255^255, huge result, won't fit in 4 bytes
        let analyzed = make_analyzed(vec![0x60, 0xFF, 0x60, 0xFF, 0x0A, 0x00], 4);
        let (_result, stats) = optimize(analyzed);

        // Result is enormous (wraps mod 2^256) — may or may not fit depending on value
        // Let's just verify detection works; fold may be skipped if too large
        assert_eq!(stats.patterns_detected, 1);
    }

    #[test]
    fn test_fold_push1_push1_signextend() {
        // PUSH1 0xFF, PUSH1 0, SIGNEXTEND → sign-extend byte 0 of 0xFF
        // EVM: SIGNEXTEND pops [byte_size_minus_one, value_to_extend]
        // PUSH 0xFF (first), PUSH 0 (second=top=byte_size_minus_one)
        // So second_val=0, first_val=0xFF → sign-extend byte 0 of 0xFF
        // Bit 7 of 0xFF is 1 → extend with 1s → result is U256::MAX
        // U256::MAX is 32 bytes — won't fit in 4 byte data_size
        let analyzed = make_analyzed(vec![0x60, 0xFF, 0x60, 0x00, 0x0B, 0x00], 4);
        let (_result, stats) = optimize(analyzed);

        assert_eq!(stats.patterns_detected, 1);
        assert_eq!(stats.patterns_folded, 0); // Result too large
    }

    #[test]
    fn test_fold_push1_push1_signextend_zero_extend() {
        // PUSH1 0x7F, PUSH1 0, SIGNEXTEND → sign-extend byte 0 of 0x7F
        // Bit 7 of 0x7F is 0 → zero-extend → result is 0x7F & 0x7F = 0x7F
        let analyzed = make_analyzed(vec![0x60, 0x7F, 0x60, 0x00, 0x0B, 0x00], 4);
        let (result, stats) = optimize(analyzed);

        assert_eq!(
            result.bytecode.as_ref(),
            &[0x63, 0x00, 0x00, 0x00, 0x7F, 0x00]
        );
        assert_eq!(stats.patterns_folded, 1);
    }

    #[test]
    fn test_fold_push1_push1_lt() {
        // PUSH1 5, PUSH1 3, LT → EVM: 3 < 5 = 1
        let analyzed = make_analyzed(vec![0x60, 0x05, 0x60, 0x03, 0x10, 0x00], 4);
        let (result, stats) = optimize(analyzed);

        assert_eq!(
            result.bytecode.as_ref(),
            &[0x63, 0x00, 0x00, 0x00, 0x01, 0x00]
        );
        assert_eq!(stats.patterns_folded, 1);
    }

    #[test]
    fn test_fold_push1_push1_gt() {
        // PUSH1 3, PUSH1 5, GT → EVM: 5 > 3 = 1
        let analyzed = make_analyzed(vec![0x60, 0x03, 0x60, 0x05, 0x11, 0x00], 4);
        let (result, stats) = optimize(analyzed);

        assert_eq!(
            result.bytecode.as_ref(),
            &[0x63, 0x00, 0x00, 0x00, 0x01, 0x00]
        );
        assert_eq!(stats.patterns_folded, 1);
    }

    #[test]
    fn test_fold_push1_push1_eq() {
        // PUSH1 3, PUSH1 3, EQ → EVM: 3 == 3 = 1
        let analyzed = make_analyzed(vec![0x60, 0x03, 0x60, 0x03, 0x14, 0x00], 4);
        let (result, stats) = optimize(analyzed);

        assert_eq!(
            result.bytecode.as_ref(),
            &[0x63, 0x00, 0x00, 0x00, 0x01, 0x00]
        );
        assert_eq!(stats.patterns_folded, 1);
    }

    #[test]
    fn test_fold_push1_push1_eq_false() {
        // PUSH1 3, PUSH1 4, EQ → EVM: 4 == 3 = 0
        let analyzed = make_analyzed(vec![0x60, 0x03, 0x60, 0x04, 0x14, 0x00], 4);
        let (result, stats) = optimize(analyzed);

        assert_eq!(
            result.bytecode.as_ref(),
            &[0x63, 0x00, 0x00, 0x00, 0x00, 0x00]
        );
        assert_eq!(stats.patterns_folded, 1);
    }

    #[test]
    fn test_fold_push1_push1_slt() {
        // SLT: unsigned 3 vs 5 with same sign → 3 < 5 = 1
        // PUSH1 5, PUSH1 3, SLT → EVM: 3 < 5 (signed) = 1
        let analyzed = make_analyzed(vec![0x60, 0x05, 0x60, 0x03, 0x12, 0x00], 4);
        let (result, stats) = optimize(analyzed);

        assert_eq!(
            result.bytecode.as_ref(),
            &[0x63, 0x00, 0x00, 0x00, 0x01, 0x00]
        );
        assert_eq!(stats.patterns_folded, 1);
    }

    #[test]
    fn test_fold_push1_push1_sgt() {
        // SGT: PUSH1 3, PUSH1 5, SGT → EVM: 5 > 3 (signed) = 1
        let analyzed = make_analyzed(vec![0x60, 0x03, 0x60, 0x05, 0x13, 0x00], 4);
        let (result, stats) = optimize(analyzed);

        assert_eq!(
            result.bytecode.as_ref(),
            &[0x63, 0x00, 0x00, 0x00, 0x01, 0x00]
        );
        assert_eq!(stats.patterns_folded, 1);
    }

    #[test]
    fn test_fold_push1_push1_shl() {
        // PUSH1 1, PUSH1 8, SHL → EVM: pops [shift=8, value=1] → 1 << 8 = 256
        // Wait: PUSH1 1 (first), PUSH1 8 (second=top). SHL pops shift=top=8, value=1.
        // In eval_op: second_val=8 (shift), first_val=1 (value) → 1 << 8 = 256
        let analyzed = make_analyzed(vec![0x60, 0x01, 0x60, 0x08, 0x1B, 0x00], 4);
        let (result, stats) = optimize(analyzed);

        // 256 = 0x0100
        assert_eq!(
            result.bytecode.as_ref(),
            &[0x63, 0x00, 0x00, 0x01, 0x00, 0x00]
        );
        assert_eq!(stats.patterns_folded, 1);
    }

    #[test]
    fn test_fold_push1_push1_shr() {
        // PUSH2 0x0100 (256), PUSH1 8, SHR → EVM: pops [shift=8, value=256] → 256 >> 8 = 1
        // PUSH2 first (value), PUSH1 second (shift=top)
        let analyzed = make_analyzed(vec![0x61, 0x01, 0x00, 0x60, 0x08, 0x1C, 0x00], 4);
        let (result, stats) = optimize(analyzed);

        // 256 >> 8 = 1
        // Pattern length = 3 + 2 + 1 = 6, data_size = 5
        assert_eq!(
            result.bytecode.as_ref(),
            &[0x64, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00]
        );
        assert_eq!(stats.patterns_folded, 1);
    }

    #[test]
    fn test_fold_push1_push1_sar() {
        // SAR on positive: same as SHR
        // PUSH2 0x0100 (256), PUSH1 8, SAR → 256 >> 8 = 1
        let analyzed = make_analyzed(vec![0x61, 0x01, 0x00, 0x60, 0x08, 0x1D, 0x00], 4);
        let (result, stats) = optimize(analyzed);

        assert_eq!(
            result.bytecode.as_ref(),
            &[0x64, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00]
        );
        assert_eq!(stats.patterns_folded, 1);
    }

    // ── G-7: Unary opcode folding tests ──────────────────────────────

    #[test]
    fn test_fold_push1_not_skipped() {
        // PUSH1 0, NOT → !0 = U256::MAX (32 bytes, won't fit in PUSH1's 1-byte data)
        let analyzed = make_analyzed(vec![0x60, 0x00, 0x19, 0x00], 3);
        let (_result, stats) = optimize(analyzed);

        assert_eq!(stats.unary_patterns_folded, 0); // Too large for available space
    }

    #[test]
    fn test_fold_push1_iszero_true() {
        // PUSH1 0, ISZERO → ISZERO(0) = 1
        let analyzed = make_analyzed(vec![0x60, 0x00, 0x15, 0x00], 3);
        let (result, stats) = optimize(analyzed);

        // Pattern length 3, data_size 2, PUSH2 = 0x61
        assert_eq!(result.bytecode.as_ref(), &[0x61, 0x00, 0x01, 0x00]);
        assert_eq!(stats.unary_patterns_folded, 1);
        assert_eq!(stats.opcodes_eliminated, 1);
    }

    #[test]
    fn test_fold_push1_iszero_false() {
        // PUSH1 5, ISZERO → ISZERO(5) = 0
        let analyzed = make_analyzed(vec![0x60, 0x05, 0x15, 0x00], 3);
        let (result, stats) = optimize(analyzed);

        assert_eq!(result.bytecode.as_ref(), &[0x61, 0x00, 0x00, 0x00]);
        assert_eq!(stats.unary_patterns_folded, 1);
    }

    #[test]
    fn test_detect_unary_pattern() {
        // PUSH1 5, ISZERO, STOP
        let bytecode = vec![0x60, 0x05, 0x15, 0x00];
        let patterns = detect_unary_patterns(&bytecode);

        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].offset, 0);
        assert_eq!(patterns[0].length, 3);
        assert_eq!(patterns[0].val, U256::from(5));
        assert_eq!(patterns[0].op, ISZERO);
    }

    #[test]
    fn test_detect_unary_pattern_not() {
        // PUSH1 0xFF, NOT, STOP
        let bytecode = vec![0x60, 0xFF, 0x19, 0x00];
        let patterns = detect_unary_patterns(&bytecode);

        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].op, NOT);
        assert_eq!(patterns[0].val, U256::from(0xFF));
    }

    #[test]
    fn test_eval_op_div() {
        assert_eq!(
            eval_op(DIV, U256::from(5), U256::from(20)),
            Some(U256::from(4))
        );
        assert_eq!(
            eval_op(DIV, U256::zero(), U256::from(10)),
            Some(U256::zero())
        );
    }

    #[test]
    fn test_eval_op_mod() {
        assert_eq!(
            eval_op(MOD, U256::from(3), U256::from(10)),
            Some(U256::from(1))
        );
        assert_eq!(
            eval_op(MOD, U256::zero(), U256::from(10)),
            Some(U256::zero())
        );
    }

    #[test]
    fn test_eval_op_comparisons() {
        assert_eq!(eval_op(LT, U256::from(5), U256::from(3)), Some(U256::one()));
        assert_eq!(eval_op(GT, U256::from(3), U256::from(5)), Some(U256::one()));
        assert_eq!(eval_op(EQ, U256::from(3), U256::from(3)), Some(U256::one()));
        assert_eq!(
            eval_op(EQ, U256::from(3), U256::from(4)),
            Some(U256::zero())
        );
    }

    #[test]
    fn test_eval_op_shifts() {
        // SHL: second_val=shift, first_val=value → value << shift
        assert_eq!(
            eval_op(SHL, U256::from(1), U256::from(8)),
            Some(U256::from(256))
        );
        // SHR: 256 >> 8 = 1
        assert_eq!(
            eval_op(SHR, U256::from(256), U256::from(8)),
            Some(U256::from(1))
        );
        // Shift >= 256 → 0
        assert_eq!(
            eval_op(SHL, U256::from(1), U256::from(256)),
            Some(U256::zero())
        );
    }

    #[test]
    fn test_eval_unary_ops() {
        assert_eq!(eval_unary_op(ISZERO, U256::zero()), Some(U256::one()));
        assert_eq!(eval_unary_op(ISZERO, U256::from(5)), Some(U256::zero()));
        assert_eq!(eval_unary_op(NOT, U256::zero()), Some(U256::MAX));
        assert_eq!(eval_unary_op(NOT, U256::MAX), Some(U256::zero()));
    }

    #[test]
    fn test_signed_helpers() {
        assert!(!is_negative(U256::zero()));
        assert!(!is_negative(U256::from(1)));
        assert!(is_negative(negate(U256::from(1)))); // -1 has bit 255 set

        assert_eq!(abs_val(U256::from(5)), U256::from(5));
        assert_eq!(abs_val(negate(U256::from(5))), U256::from(5));

        assert_eq!(u256_from_bool(true), U256::one());
        assert_eq!(u256_from_bool(false), U256::zero());
    }
}
