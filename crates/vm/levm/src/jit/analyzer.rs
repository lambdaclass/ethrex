//! Bytecode analyzer for JIT compilation.
//!
//! Identifies basic block boundaries in EVM bytecode. Reuses LEVM's
//! pre-computed `jump_targets` to avoid redundant JUMPDEST scanning.

use bytes::Bytes;
use ethrex_common::H256;

use super::types::AnalyzedBytecode;

/// Opcodes that terminate a basic block.
const STOP: u8 = 0x00;
const JUMP: u8 = 0x56;
const JUMPI: u8 = 0x57;
const JUMPDEST: u8 = 0x5b;
const RETURN: u8 = 0xf3;
const REVERT: u8 = 0xfd;
const INVALID: u8 = 0xfe;
const SELFDESTRUCT: u8 = 0xff;

/// Returns the number of immediate bytes following a PUSH opcode.
/// PUSH1..PUSH32 are opcodes 0x60..0x7f, pushing 1..32 bytes.
fn push_size(opcode: u8) -> usize {
    if (0x60..=0x7f).contains(&opcode) {
        // PUSH1 = 0x60 pushes 1 byte, PUSH32 = 0x7f pushes 32 bytes
        #[allow(clippy::as_conversions, clippy::arithmetic_side_effects)]
        let size = (opcode - 0x5f) as usize;
        size
    } else {
        0
    }
}

/// Analyze bytecode to identify basic block boundaries.
///
/// Reuses the `jump_targets` already computed by LEVM's `Code::compute_jump_targets()`.
pub fn analyze_bytecode(bytecode: Bytes, hash: H256, jump_targets: Vec<u32>) -> AnalyzedBytecode {
    let mut basic_blocks = Vec::new();
    let mut block_start: usize = 0;
    let mut opcode_count: usize = 0;
    let mut i: usize = 0;
    let len = bytecode.len();

    while i < len {
        #[expect(clippy::indexing_slicing, reason = "i < len checked in loop condition")]
        let opcode = bytecode[i];
        opcode_count = opcode_count.saturating_add(1);

        let is_block_terminator = matches!(
            opcode,
            STOP | JUMP | JUMPI | RETURN | REVERT | INVALID | SELFDESTRUCT
        );

        if is_block_terminator {
            basic_blocks.push((block_start, i));
            block_start = i.saturating_add(1);
        } else if opcode == JUMPDEST && i > block_start {
            // JUMPDEST starts a new block (end previous block before it)
            basic_blocks.push((block_start, i.saturating_sub(1)));
            block_start = i;
        }

        // Skip PUSH immediate bytes
        i = i.saturating_add(1).saturating_add(push_size(opcode));
    }

    // Close the final block if it wasn't terminated
    if block_start < len {
        basic_blocks.push((block_start, len.saturating_sub(1)));
    }

    AnalyzedBytecode {
        hash,
        bytecode,
        jump_targets,
        basic_blocks,
        opcode_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_size() {
        assert_eq!(push_size(0x00), 0); // STOP
        assert_eq!(push_size(0x60), 1); // PUSH1
        assert_eq!(push_size(0x7f), 32); // PUSH32
        assert_eq!(push_size(0x80), 0); // DUP1
    }

    #[test]
    fn test_simple_basic_blocks() {
        // PUSH1 0x01 PUSH1 0x02 ADD STOP
        let bytecode = Bytes::from(vec![0x60, 0x01, 0x60, 0x02, 0x01, 0x00]);
        let result = analyze_bytecode(bytecode, H256::zero(), vec![]);

        assert_eq!(result.basic_blocks.len(), 1);
        assert_eq!(result.basic_blocks[0], (0, 5)); // STOP at index 5
        assert_eq!(result.opcode_count, 4); // PUSH1, PUSH1, ADD, STOP
    }

    #[test]
    fn test_jumpdest_splits_blocks() {
        // PUSH1 0x04 JUMP JUMPDEST STOP
        // Block 1: [0..2] PUSH1 0x04 JUMP (terminated by JUMP)
        // Block 2: [3..4] JUMPDEST STOP (JUMPDEST at block_start, no split; STOP terminates)
        let bytecode = Bytes::from(vec![0x60, 0x04, 0x56, 0x5b, 0x00]);
        let result = analyze_bytecode(bytecode, H256::zero(), vec![3]);

        assert_eq!(result.basic_blocks.len(), 2);
        assert_eq!(result.basic_blocks[0], (0, 2)); // PUSH1 0x04 JUMP
        assert_eq!(result.basic_blocks[1], (3, 4)); // JUMPDEST STOP
    }

    #[test]
    fn test_empty_bytecode() {
        let bytecode = Bytes::new();
        let result = analyze_bytecode(bytecode, H256::zero(), vec![]);

        assert!(result.basic_blocks.is_empty());
        assert_eq!(result.opcode_count, 0);
    }
}
