//! [`Code`]'s JUMPDEST bitmap: which offsets are valid jump destinations.

use ethrex_common::H256;
use ethrex_common::types::Code;

const JUMPDEST: u8 = 0x5b;
const PUSH1: u8 = 0x60;
const PUSH32: u8 = 0x7f;
const STOP: u8 = 0x00;

fn code_of(bytecode: Vec<u8>) -> Code {
    Code::from_bytecode_unchecked(bytecode.into(), H256::zero())
}

/// A `JUMPDEST` reached by the opcode walk SHALL be a valid destination.
#[test]
fn jumpdests_outside_immediates_are_valid() {
    let code = code_of(vec![STOP, JUMPDEST, STOP, JUMPDEST]);

    assert!(code.is_valid_jumpdest(1));
    assert!(code.is_valid_jumpdest(3));
    assert!(!code.is_valid_jumpdest(0));
    assert!(!code.is_valid_jumpdest(2));
}

/// A `0x5b` byte that is part of a `PUSH` immediate SHALL NOT be a valid destination:
/// it is data, not an opcode.
#[test]
fn jumpdest_bytes_inside_push_immediates_are_not_valid() {
    // PUSH1 0x5b | JUMPDEST | PUSH32 <32 x 0x5b> | JUMPDEST
    let mut bytecode = vec![PUSH1, JUMPDEST, JUMPDEST, PUSH32];
    bytecode.extend([JUMPDEST; 32]);
    bytecode.push(JUMPDEST);
    let code = code_of(bytecode);

    assert!(!code.is_valid_jumpdest(1), "PUSH1 immediate");
    assert!(code.is_valid_jumpdest(2), "opcode between the two PUSHes");
    for offset in 4..36 {
        assert!(!code.is_valid_jumpdest(offset), "PUSH32 immediate {offset}");
    }
    assert!(code.is_valid_jumpdest(36), "opcode after the immediates");
}

/// Offsets past the bytecode SHALL NOT be valid, including offsets inside the trailing
/// [`BYTECODE_PADDING`](ethrex_common::types::BYTECODE_PADDING) and inside the last,
/// partially used bitmap byte.
#[test]
fn offsets_past_the_bytecode_are_not_valid() {
    // 9 bytes of code, so the bitmap's second byte covers offsets 8..16 and only its
    // lowest bit is meaningful.
    let code = code_of(vec![JUMPDEST; 9]);

    assert!(code.is_valid_jumpdest(8));
    for offset in [9, 10, 15, 16, 100, usize::MAX] {
        assert!(!code.is_valid_jumpdest(offset));
    }
}

/// Jumpless bytecode SHALL have an empty bitmap, as SHALL empty code.
#[test]
fn jumpless_bytecode_has_an_empty_bitmap() {
    assert!(code_of(vec![STOP; 64]).jumpdests().is_empty());
    assert!(code_of(vec![PUSH1, JUMPDEST]).jumpdests().is_empty());
    assert!(Code::default().jumpdests().is_empty());
    assert!(!Code::default().is_valid_jumpdest(0));
}

/// The bitmap SHALL be one bit per bytecode byte.
#[test]
fn bitmap_is_one_bit_per_bytecode_byte() {
    for len in [1usize, 7, 8, 9, 24576] {
        let mut bytecode = vec![STOP; len];
        bytecode[len - 1] = JUMPDEST;
        let code = code_of(bytecode);

        assert_eq!(code.jumpdests().len(), len.div_ceil(8), "len {len}");
        assert!(code.is_valid_jumpdest(len - 1), "len {len}");
    }
}
