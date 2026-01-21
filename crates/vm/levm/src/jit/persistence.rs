//! JIT code persistence layer
//!
//! Provides serialization and deserialization of JIT-compiled code
//! for caching in the storage layer.

use ethrex_common::U256;
use serde::{Deserialize, Serialize};

/// Opcode identifier for serialization.
///
/// Instead of storing function pointers, we store which opcode
/// is at each PC and reconstruct the function table at load time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum JitOpcodeId {
    Stop = 0x00,
    Add = 0x01,
    Mul = 0x02,
    Sub = 0x03,
    Div = 0x04,
    Sdiv = 0x05,
    Mod = 0x06,
    Smod = 0x07,
    Addmod = 0x08,
    Mulmod = 0x09,
    Exp = 0x0a,
    Signextend = 0x0b,
    Lt = 0x10,
    Gt = 0x11,
    Eq = 0x14,
    Iszero = 0x15,
    And = 0x16,
    Or = 0x17,
    Xor = 0x18,
    Not = 0x19,
    Byte = 0x1a,
    Shl = 0x1b,
    Shr = 0x1c,
    Sar = 0x1d,
    Address = 0x30,
    Caller = 0x33,
    Callvalue = 0x34,
    Calldataload = 0x35,
    Calldatasize = 0x36,
    Codesize = 0x38,
    Pop = 0x50,
    Mload = 0x51,
    Mstore = 0x52,
    Mstore8 = 0x53,
    Sload = 0x54,
    Sstore = 0x55,
    Jump = 0x56,
    Jumpi = 0x57,
    Pc = 0x58,
    Msize = 0x59,
    Gas = 0x5a,
    Jumpdest = 0x5b,
    Push = 0x60, // Covers PUSH1-PUSH32
    Dup1 = 0x80,
    Dup2 = 0x81,
    Dup3 = 0x82,
    Dup4 = 0x83,
    Dup5 = 0x84,
    Dup6 = 0x85,
    Dup7 = 0x86,
    Dup8 = 0x87,
    Dup9 = 0x88,
    Dup10 = 0x89,
    Dup11 = 0x8a,
    Dup12 = 0x8b,
    Dup13 = 0x8c,
    Dup14 = 0x8d,
    Dup15 = 0x8e,
    Dup16 = 0x8f,
    Swap1 = 0x90,
    Swap2 = 0x91,
    Swap3 = 0x92,
    Swap4 = 0x93,
    Swap5 = 0x94,
    Swap6 = 0x95,
    Swap7 = 0x96,
    Swap8 = 0x97,
    Swap9 = 0x98,
    Swap10 = 0x99,
    Swap11 = 0x9a,
    Swap12 = 0x9b,
    Swap13 = 0x9c,
    Swap14 = 0x9d,
    Swap15 = 0x9e,
    Swap16 = 0x9f,
    Return = 0xf3,
    Revert = 0xfd,
    Invalid = 0xfe,
}

/// Serialized compiled operation at a single PC
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedOp {
    /// Which opcode this is
    pub opcode_id: JitOpcodeId,
    /// Size of this instruction in bytecode
    pub size: u8,
}

/// Serializable representation of JIT-compiled code.
///
/// This struct can be serialized to bytes and stored in the database,
/// then deserialized and used to reconstruct a JitCode at load time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedJitCode {
    /// Compiled operations indexed by bytecode PC.
    /// None means no instruction starts at this PC (middle of PUSH value).
    pub ops: Vec<Option<SerializedOp>>,
    /// Valid jump destinations bitmap.
    pub valid_jumpdests: Vec<bool>,
    /// Push values indexed by PC (only valid for PUSH instructions).
    /// Stored as big-endian bytes for compact serialization.
    pub push_values: Vec<Option<[u8; 32]>>,
}

impl SerializedJitCode {
    /// Serialize to bytes using bincode.
    pub fn to_bytes(&self) -> Result<Vec<u8>, bincode::Error> {
        bincode::serialize(self)
    }

    /// Deserialize from bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, bincode::Error> {
        bincode::deserialize(bytes)
    }
}

/// Convert a U256 to big-endian bytes for serialization.
pub fn u256_to_bytes(value: U256) -> [u8; 32] {
    value.to_big_endian()
}

/// Convert big-endian bytes back to U256.
pub fn bytes_to_u256(bytes: &[u8; 32]) -> U256 {
    U256::from_big_endian(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialize_deserialize() {
        let code = SerializedJitCode {
            ops: vec![
                Some(SerializedOp { opcode_id: JitOpcodeId::Push, size: 2 }),
                None, // Middle of PUSH value
                Some(SerializedOp { opcode_id: JitOpcodeId::Stop, size: 1 }),
            ],
            valid_jumpdests: vec![false, false, false],
            push_values: vec![Some([0u8; 32]), None, None],
        };

        let bytes = code.to_bytes().expect("serialization should succeed in test");
        let deserialized = SerializedJitCode::from_bytes(&bytes).expect("deserialization should succeed in test");

        assert_eq!(deserialized.ops.len(), 3);
        assert!(deserialized.ops[0].is_some());
        assert!(deserialized.ops[1].is_none());
        assert!(deserialized.ops[2].is_some());
    }

    #[test]
    fn test_u256_roundtrip() {
        let value = U256::from(0x123456789ABCDEFu64);
        let bytes = u256_to_bytes(value);
        let restored = bytes_to_u256(&bytes);
        assert_eq!(value, restored);
    }
}
