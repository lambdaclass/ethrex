use bytes::Bytes;
use ethereum_types::H256;
use ethereum_types::{Address, U256};
use serde::Serialize;
use std::collections::BTreeMap;

/// Collection of traces of each call frame as defined in geth's `callTracer` output
/// https://geth.ethereum.org/docs/developers/evm-tracing/built-in-tracers#call-tracer
pub type CallTrace = Vec<CallTraceFrame>;

/// Trace of each call frame as defined in geth's `callTracer` output
/// https://geth.ethereum.org/docs/developers/evm-tracing/built-in-tracers#call-tracer
#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CallTraceFrame {
    /// Type of the Call
    #[serde(rename = "type")]
    pub call_type: CallType,
    /// Address that initiated the call
    pub from: Address,
    /// Address that received the call
    pub to: Address,
    /// Amount transfered
    pub value: U256,
    /// Gas provided for the call
    #[serde(with = "crate::serde_utils::u64::hex_str")]
    pub gas: u64,
    /// Gas used by the call
    #[serde(with = "crate::serde_utils::u64::hex_str")]
    pub gas_used: u64,
    /// Call data
    #[serde(with = "crate::serde_utils::bytes")]
    pub input: Bytes,
    /// Return data
    #[serde(with = "crate::serde_utils::bytes")]
    pub output: Bytes,
    /// Error returned if the call failed
    pub error: Option<String>,
    /// Revert reason if the call reverted
    pub revert_reason: Option<String>,
    /// List of nested sub-calls
    pub calls: Vec<CallTraceFrame>,
    /// Logs (if enabled)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub logs: Vec<CallLog>,
}

#[derive(Serialize, Debug, Default)]
pub enum CallType {
    #[default]
    CALL,
    CALLCODE,
    STATICCALL,
    DELEGATECALL,
    CREATE,
    CREATE2,
    SELFDESTRUCT,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CallLog {
    pub address: Address,
    pub topics: Vec<H256>,
    #[serde(with = "crate::serde_utils::bytes")]
    pub data: Bytes,
    pub position: u64,
}

/// Per-account state entry emitted by the prestateTracer.
///
/// `balance` is `Option<U256>`: `None` means "field absent from output",
/// `Some(0)` still serializes (lets diff post emit a balance that became zero).
#[derive(Debug, Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PrestateAccountState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balance: Option<U256>,
    #[serde(default, skip_serializing_if = "is_zero_nonce")]
    pub nonce: u64,
    #[serde(
        default,
        skip_serializing_if = "Bytes::is_empty",
        with = "crate::serde_utils::bytes"
    )]
    pub code: Bytes,
    #[serde(default, skip_serializing_if = "H256::is_zero")]
    pub code_hash: H256,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub storage: BTreeMap<H256, H256>,
}

impl PrestateAccountState {
    /// True when no field conveys information; `Some(0)` balance counts as empty.
    pub fn is_empty(&self) -> bool {
        self.balance.unwrap_or_default().is_zero()
            && self.nonce == 0
            && self.code.is_empty()
            && self.code_hash.is_zero()
            && self.storage.is_empty()
    }
}

/// Per-transaction prestate trace (non-diff mode). `BTreeMap` keeps JSON output
/// deterministic via sorted keys.
pub type PrestateTrace = BTreeMap<Address, PrestateAccountState>;

/// Result of a prestateTracer execution — either a plain prestate map or a diff.
#[derive(Debug, Clone)]
pub enum PrestateResult {
    /// Non-diff mode: map of address → pre-tx account state.
    Prestate(PrestateTrace),
    /// Diff mode: pre-tx and post-tx state for all touched accounts.
    Diff(PrePostState),
}

/// Per-transaction prestate trace (diff mode).
/// Contains the pre-tx and post-tx state for all touched accounts.
#[derive(Debug, Serialize, Default, Clone)]
pub struct PrePostState {
    pub pre: BTreeMap<Address, PrestateAccountState>,
    pub post: BTreeMap<Address, PrestateAccountState>,
}

fn is_zero_nonce(n: &u64) -> bool {
    *n == 0
}

// ─── EIP-3155 StructLog types ──────────────────────────────────────────────

/// Per-opcode trace entry matching geth's `structLogLegacy` wire format.
///
/// Fields are kept as native types in memory; `Serialize` converts them to the
/// exact encoding that `debug_traceTransaction` returns from geth.
#[derive(Debug)]
pub struct StructLog {
    pub pc: u64,
    /// Raw opcode byte.  Serialized via `opcode_name`.
    pub op: u8,
    pub gas: u64,
    pub gas_cost: u64,
    pub depth: u32,
    pub refund: u64,
    /// `Some(vec)` when stack capture is enabled (may be empty); `None` when disabled.
    pub stack: Option<Vec<U256>>,
    /// `Some(chunks)` when memory capture is enabled; `None` when disabled.
    pub memory: Option<Vec<MemoryChunk>>,
    /// `Some(map)` at SLOAD/SSTORE steps when storage capture is enabled.
    pub storage: Option<BTreeMap<H256, H256>>,
    /// Non-empty return data from the previous sub-call, when enabled.
    pub return_data: Option<bytes::Bytes>,
    pub error: Option<String>,
}

/// A 32-byte chunk of EVM memory, serialized as `"0x" + 64 lowercase hex chars`.
/// The *caller* zero-pads the last partial chunk before constructing this type.
#[derive(Debug)]
pub struct MemoryChunk(pub [u8; 32]);

/// Top-level result returned by a struct-log trace, matching geth's
/// `executionResult` shape.
#[derive(Debug)]
pub struct StructLogResult {
    pub gas: u64,
    pub failed: bool,
    pub return_value: bytes::Bytes,
    pub struct_logs: Vec<StructLog>,
}

// ─── Helpers ──────────────────────────────────────────────────────────────

/// Returns the geth-compatible opcode mnemonic for `byte`.
///
/// `0xFE` → `"INVALID"`.  All other assigned opcodes → their uppercase name
/// (e.g. `"PUSH1"`, `"ADD"`).  Unassigned bytes → `"opcode 0xNN"` (lowercase
/// hex, two digits), matching geth's fallback.
pub fn opcode_name(byte: u8) -> String {
    match byte {
        0x00 => "STOP".to_string(),
        0x01 => "ADD".to_string(),
        0x02 => "MUL".to_string(),
        0x03 => "SUB".to_string(),
        0x04 => "DIV".to_string(),
        0x05 => "SDIV".to_string(),
        0x06 => "MOD".to_string(),
        0x07 => "SMOD".to_string(),
        0x08 => "ADDMOD".to_string(),
        0x09 => "MULMOD".to_string(),
        0x0A => "EXP".to_string(),
        0x0B => "SIGNEXTEND".to_string(),
        0x10 => "LT".to_string(),
        0x11 => "GT".to_string(),
        0x12 => "SLT".to_string(),
        0x13 => "SGT".to_string(),
        0x14 => "EQ".to_string(),
        0x15 => "ISZERO".to_string(),
        0x16 => "AND".to_string(),
        0x17 => "OR".to_string(),
        0x18 => "XOR".to_string(),
        0x19 => "NOT".to_string(),
        0x1A => "BYTE".to_string(),
        0x1B => "SHL".to_string(),
        0x1C => "SHR".to_string(),
        0x1D => "SAR".to_string(),
        0x1E => "CLZ".to_string(),
        0x20 => "KECCAK256".to_string(),
        0x30 => "ADDRESS".to_string(),
        0x31 => "BALANCE".to_string(),
        0x32 => "ORIGIN".to_string(),
        0x33 => "CALLER".to_string(),
        0x34 => "CALLVALUE".to_string(),
        0x35 => "CALLDATALOAD".to_string(),
        0x36 => "CALLDATASIZE".to_string(),
        0x37 => "CALLDATACOPY".to_string(),
        0x38 => "CODESIZE".to_string(),
        0x39 => "CODECOPY".to_string(),
        0x3A => "GASPRICE".to_string(),
        0x3B => "EXTCODESIZE".to_string(),
        0x3C => "EXTCODECOPY".to_string(),
        0x3D => "RETURNDATASIZE".to_string(),
        0x3E => "RETURNDATACOPY".to_string(),
        0x3F => "EXTCODEHASH".to_string(),
        0x40 => "BLOCKHASH".to_string(),
        0x41 => "COINBASE".to_string(),
        0x42 => "TIMESTAMP".to_string(),
        0x43 => "NUMBER".to_string(),
        0x44 => "PREVRANDAO".to_string(),
        0x45 => "GASLIMIT".to_string(),
        0x46 => "CHAINID".to_string(),
        0x47 => "SELFBALANCE".to_string(),
        0x48 => "BASEFEE".to_string(),
        0x49 => "BLOBHASH".to_string(),
        0x4A => "BLOBBASEFEE".to_string(),
        0x4B => "SLOTNUM".to_string(),
        0x50 => "POP".to_string(),
        0x51 => "MLOAD".to_string(),
        0x52 => "MSTORE".to_string(),
        0x53 => "MSTORE8".to_string(),
        0x54 => "SLOAD".to_string(),
        0x55 => "SSTORE".to_string(),
        0x56 => "JUMP".to_string(),
        0x57 => "JUMPI".to_string(),
        0x58 => "PC".to_string(),
        0x59 => "MSIZE".to_string(),
        0x5A => "GAS".to_string(),
        0x5B => "JUMPDEST".to_string(),
        0x5C => "TLOAD".to_string(),
        0x5D => "TSTORE".to_string(),
        0x5E => "MCOPY".to_string(),
        0x5F => "PUSH0".to_string(),
        0x60 => "PUSH1".to_string(),
        0x61 => "PUSH2".to_string(),
        0x62 => "PUSH3".to_string(),
        0x63 => "PUSH4".to_string(),
        0x64 => "PUSH5".to_string(),
        0x65 => "PUSH6".to_string(),
        0x66 => "PUSH7".to_string(),
        0x67 => "PUSH8".to_string(),
        0x68 => "PUSH9".to_string(),
        0x69 => "PUSH10".to_string(),
        0x6A => "PUSH11".to_string(),
        0x6B => "PUSH12".to_string(),
        0x6C => "PUSH13".to_string(),
        0x6D => "PUSH14".to_string(),
        0x6E => "PUSH15".to_string(),
        0x6F => "PUSH16".to_string(),
        0x70 => "PUSH17".to_string(),
        0x71 => "PUSH18".to_string(),
        0x72 => "PUSH19".to_string(),
        0x73 => "PUSH20".to_string(),
        0x74 => "PUSH21".to_string(),
        0x75 => "PUSH22".to_string(),
        0x76 => "PUSH23".to_string(),
        0x77 => "PUSH24".to_string(),
        0x78 => "PUSH25".to_string(),
        0x79 => "PUSH26".to_string(),
        0x7A => "PUSH27".to_string(),
        0x7B => "PUSH28".to_string(),
        0x7C => "PUSH29".to_string(),
        0x7D => "PUSH30".to_string(),
        0x7E => "PUSH31".to_string(),
        0x7F => "PUSH32".to_string(),
        0x80 => "DUP1".to_string(),
        0x81 => "DUP2".to_string(),
        0x82 => "DUP3".to_string(),
        0x83 => "DUP4".to_string(),
        0x84 => "DUP5".to_string(),
        0x85 => "DUP6".to_string(),
        0x86 => "DUP7".to_string(),
        0x87 => "DUP8".to_string(),
        0x88 => "DUP9".to_string(),
        0x89 => "DUP10".to_string(),
        0x8A => "DUP11".to_string(),
        0x8B => "DUP12".to_string(),
        0x8C => "DUP13".to_string(),
        0x8D => "DUP14".to_string(),
        0x8E => "DUP15".to_string(),
        0x8F => "DUP16".to_string(),
        0x90 => "SWAP1".to_string(),
        0x91 => "SWAP2".to_string(),
        0x92 => "SWAP3".to_string(),
        0x93 => "SWAP4".to_string(),
        0x94 => "SWAP5".to_string(),
        0x95 => "SWAP6".to_string(),
        0x96 => "SWAP7".to_string(),
        0x97 => "SWAP8".to_string(),
        0x98 => "SWAP9".to_string(),
        0x99 => "SWAP10".to_string(),
        0x9A => "SWAP11".to_string(),
        0x9B => "SWAP12".to_string(),
        0x9C => "SWAP13".to_string(),
        0x9D => "SWAP14".to_string(),
        0x9E => "SWAP15".to_string(),
        0x9F => "SWAP16".to_string(),
        0xA0 => "LOG0".to_string(),
        0xA1 => "LOG1".to_string(),
        0xA2 => "LOG2".to_string(),
        0xA3 => "LOG3".to_string(),
        0xA4 => "LOG4".to_string(),
        0xE6 => "DUPN".to_string(),
        0xE7 => "SWAPN".to_string(),
        0xE8 => "EXCHANGE".to_string(),
        0xF0 => "CREATE".to_string(),
        0xF1 => "CALL".to_string(),
        0xF2 => "CALLCODE".to_string(),
        0xF3 => "RETURN".to_string(),
        0xF4 => "DELEGATECALL".to_string(),
        0xF5 => "CREATE2".to_string(),
        0xFA => "STATICCALL".to_string(),
        0xFD => "REVERT".to_string(),
        0xFE => "INVALID".to_string(),
        0xFF => "SELFDESTRUCT".to_string(),
        b => format!("opcode 0x{:02x}", b),
    }
}

/// Converts a `U256` to geth's `uint256.Int.Hex()` form: `"0x"` followed by
/// lowercase hex with leading zeros stripped.  Zero → `"0x0"` (not `"0x"`).
pub fn geth_uint256_hex(v: &U256) -> String {
    if v.is_zero() {
        return "0x0".to_string();
    }
    // U256 words are little-endian; convert to big-endian bytes.
    let bytes = crate::utils::u256_to_big_endian(*v);
    let hex_str = hex::encode(bytes);
    let stripped = hex_str.trim_start_matches('0');
    format!("0x{}", stripped)
}

fn is_zero_u64(n: &u64) -> bool {
    *n == 0
}

fn serialize_stack<S>(stack: &Option<Vec<U256>>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeSeq;
    match stack {
        None => serializer.serialize_none(),
        Some(vec) => {
            let mut seq = serializer.serialize_seq(Some(vec.len()))?;
            for v in vec {
                seq.serialize_element(&geth_uint256_hex(v))?;
            }
            seq.end()
        }
    }
}

fn serialize_return_data<S>(rd: &Option<bytes::Bytes>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match rd {
        None => serializer.serialize_none(),
        Some(b) => serializer.serialize_str(&format!("0x{}", hex::encode(b))),
    }
}

fn serialize_storage<S>(
    storage: &Option<BTreeMap<H256, H256>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeMap;
    match storage {
        None => serializer.serialize_none(),
        Some(map) => {
            let mut m = serializer.serialize_map(Some(map.len()))?;
            for (k, v) in map {
                let k_str = format!("0x{}", hex::encode(k.as_bytes()));
                let v_str = format!("0x{}", hex::encode(v.as_bytes()));
                m.serialize_entry(&k_str, &v_str)?;
            }
            m.end()
        }
    }
}

fn is_return_data_absent(rd: &Option<bytes::Bytes>) -> bool {
    match rd {
        None => true,
        Some(b) => b.is_empty(),
    }
}

// ─── Serialize impls ──────────────────────────────────────────────────────

impl serde::Serialize for MemoryChunk {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("0x{}", hex::encode(self.0)))
    }
}

impl serde::Serialize for StructLog {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;

        // Count the number of fields that will be emitted so the map hint is accurate.
        // Required fields: pc, op, gas, gasCost, depth = 5
        // Optional: refund, stack, memory, storage, returnData, error
        let mut field_count = 5;
        if !is_zero_u64(&self.refund) {
            field_count += 1;
        }
        if self.stack.is_some() {
            field_count += 1;
        }
        if self.memory.is_some() {
            field_count += 1;
        }
        if self.storage.is_some() {
            field_count += 1;
        }
        if !is_return_data_absent(&self.return_data) {
            field_count += 1;
        }
        if self.error.is_some() {
            field_count += 1;
        }

        let mut map = serializer.serialize_map(Some(field_count))?;

        map.serialize_entry("pc", &self.pc)?;
        map.serialize_entry("op", &opcode_name(self.op))?;
        map.serialize_entry("gas", &self.gas)?;
        map.serialize_entry("gasCost", &self.gas_cost)?;
        map.serialize_entry("depth", &self.depth)?;

        if !is_zero_u64(&self.refund) {
            map.serialize_entry("refund", &self.refund)?;
        }

        if self.stack.is_some() {
            // Serialize stack via the custom serializer logic inline.
            struct StackWrapper<'a>(&'a Option<Vec<U256>>);
            impl serde::Serialize for StackWrapper<'_> {
                fn serialize<S: serde::Serializer>(
                    &self,
                    serializer: S,
                ) -> Result<S::Ok, S::Error> {
                    serialize_stack(self.0, serializer)
                }
            }
            map.serialize_entry("stack", &StackWrapper(&self.stack))?;
        }

        if let Some(mem) = &self.memory {
            map.serialize_entry("memory", mem)?;
        }

        if self.storage.is_some() {
            struct StorageWrapper<'a>(&'a Option<BTreeMap<H256, H256>>);
            impl serde::Serialize for StorageWrapper<'_> {
                fn serialize<S: serde::Serializer>(
                    &self,
                    serializer: S,
                ) -> Result<S::Ok, S::Error> {
                    serialize_storage(self.0, serializer)
                }
            }
            map.serialize_entry("storage", &StorageWrapper(&self.storage))?;
        }

        if !is_return_data_absent(&self.return_data) {
            struct RdWrapper<'a>(&'a Option<bytes::Bytes>);
            impl serde::Serialize for RdWrapper<'_> {
                fn serialize<S: serde::Serializer>(
                    &self,
                    serializer: S,
                ) -> Result<S::Ok, S::Error> {
                    serialize_return_data(self.0, serializer)
                }
            }
            map.serialize_entry("returnData", &RdWrapper(&self.return_data))?;
        }

        if let Some(err) = &self.error {
            map.serialize_entry("error", err)?;
        }

        map.end()
    }
}

impl serde::Serialize for StructLogResult {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(4))?;
        map.serialize_entry("gas", &self.gas)?;
        map.serialize_entry("failed", &self.failed)?;
        map.serialize_entry(
            "returnValue",
            &format!("0x{}", hex::encode(&self.return_value)),
        )?;
        map.serialize_entry("structLogs", &self.struct_logs)?;
        map.end()
    }
}

// ─── Unit tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use ethereum_types::{H256, U256};
    use serde_json::Value;

    fn to_json<T: serde::Serialize>(v: &T) -> Value {
        serde_json::to_value(v).expect("serialize failed")
    }

    // ── geth_uint256_hex ──────────────────────────────────────────────────

    #[test]
    fn uint256_zero_is_0x0() {
        assert_eq!(geth_uint256_hex(&U256::zero()), "0x0");
    }

    #[test]
    fn uint256_one() {
        assert_eq!(geth_uint256_hex(&U256::from(1u64)), "0x1");
    }

    #[test]
    fn uint256_max() {
        let expected = format!("0x{}", "f".repeat(64));
        assert_eq!(geth_uint256_hex(&U256::MAX), expected);
    }

    // ── opcode_name ───────────────────────────────────────────────────────

    #[test]
    fn opcode_name_invalid() {
        assert_eq!(opcode_name(0xFE), "INVALID");
    }

    #[test]
    fn opcode_name_push1() {
        assert_eq!(opcode_name(0x60), "PUSH1");
    }

    #[test]
    fn opcode_name_unknown() {
        assert_eq!(opcode_name(0xC1), "opcode 0xc1");
    }

    // ── MemoryChunk ───────────────────────────────────────────────────────

    #[test]
    fn memory_chunk_zero_bytes() {
        let chunk = MemoryChunk([0u8; 32]);
        let j = to_json(&chunk);
        assert_eq!(j, Value::String(format!("0x{}", "0".repeat(64))));
    }

    // ── StructLog — stack field ───────────────────────────────────────────

    #[test]
    fn stack_none_omits_field() {
        let log = StructLog {
            pc: 0,
            op: 0x00,
            gas: 0,
            gas_cost: 0,
            depth: 1,
            refund: 0,
            stack: None,
            memory: None,
            storage: None,
            return_data: None,
            error: None,
        };
        let j = to_json(&log);
        assert!(j.get("stack").is_none(), "stack field should be absent");
    }

    #[test]
    fn stack_empty_vec_present_as_array() {
        let log = StructLog {
            pc: 0,
            op: 0x00,
            gas: 0,
            gas_cost: 0,
            depth: 1,
            refund: 0,
            stack: Some(vec![]),
            memory: None,
            storage: None,
            return_data: None,
            error: None,
        };
        let j = to_json(&log);
        let stack = j.get("stack").expect("stack field should be present");
        assert_eq!(stack, &Value::Array(vec![]));
    }

    #[test]
    fn stack_values_encoded_correctly() {
        let log = StructLog {
            pc: 0,
            op: 0x00,
            gas: 0,
            gas_cost: 0,
            depth: 1,
            refund: 0,
            stack: Some(vec![U256::zero(), U256::from(1u64), U256::MAX]),
            memory: None,
            storage: None,
            return_data: None,
            error: None,
        };
        let j = to_json(&log);
        let stack = j["stack"].as_array().expect("stack must be array");
        assert_eq!(stack[0], Value::String("0x0".to_string()));
        assert_eq!(stack[1], Value::String("0x1".to_string()));
        assert_eq!(stack[2], Value::String(format!("0x{}", "f".repeat(64))));
    }

    // ── StructLog — memory field ──────────────────────────────────────────

    #[test]
    fn memory_33_bytes_two_chunks_padded() {
        // 33 zero bytes → 2 chunks; second padded to 32 bytes
        let chunk0 = MemoryChunk([0u8; 32]);
        let chunk1 = MemoryChunk([0u8; 32]); // last byte padded
        let log = StructLog {
            pc: 0,
            op: 0x00,
            gas: 0,
            gas_cost: 0,
            depth: 1,
            refund: 0,
            stack: None,
            memory: Some(vec![chunk0, chunk1]),
            storage: None,
            return_data: None,
            error: None,
        };
        let j = to_json(&log);
        let mem = j["memory"].as_array().expect("memory must be array");
        assert_eq!(mem.len(), 2);
        let zeros64 = format!("0x{}", "0".repeat(64));
        assert_eq!(mem[0], Value::String(zeros64.clone()));
        assert_eq!(mem[1], Value::String(zeros64));
    }

    // ── StructLog — storage field ─────────────────────────────────────────

    #[test]
    fn storage_entry_encoded_correctly() {
        let mut storage = BTreeMap::new();
        storage.insert(H256::from_low_u64_be(1), H256::from_low_u64_be(0x2a));
        let log = StructLog {
            pc: 0,
            op: 0x54,
            gas: 0,
            gas_cost: 0,
            depth: 1,
            refund: 0,
            stack: None,
            memory: None,
            storage: Some(storage),
            return_data: None,
            error: None,
        };
        let j = to_json(&log);
        let s = j["storage"].as_object().expect("storage must be object");
        let expected_key = format!("0x{:0>64}", "1");
        let expected_val = format!("0x{:0>64}", "2a");
        let got_val = s.get(&expected_key).expect("key not found");
        assert_eq!(got_val, &Value::String(expected_val));
    }

    // ── StructLog — error field ───────────────────────────────────────────

    #[test]
    fn error_some_is_present() {
        let log = StructLog {
            pc: 0,
            op: 0x00,
            gas: 0,
            gas_cost: 0,
            depth: 1,
            refund: 0,
            stack: None,
            memory: None,
            storage: None,
            return_data: None,
            error: Some("out of gas".to_string()),
        };
        let j = to_json(&log);
        assert_eq!(j["error"], Value::String("out of gas".to_string()));
    }

    #[test]
    fn error_none_is_absent() {
        let log = StructLog {
            pc: 0,
            op: 0x00,
            gas: 0,
            gas_cost: 0,
            depth: 1,
            refund: 0,
            stack: None,
            memory: None,
            storage: None,
            return_data: None,
            error: None,
        };
        let j = to_json(&log);
        assert!(j.get("error").is_none());
    }

    // ── StructLog — refund field ──────────────────────────────────────────

    #[test]
    fn refund_zero_is_absent() {
        let log = StructLog {
            pc: 0,
            op: 0x00,
            gas: 0,
            gas_cost: 0,
            depth: 1,
            refund: 0,
            stack: None,
            memory: None,
            storage: None,
            return_data: None,
            error: None,
        };
        let j = to_json(&log);
        assert!(j.get("refund").is_none());
    }

    #[test]
    fn refund_nonzero_is_present() {
        let log = StructLog {
            pc: 0,
            op: 0x00,
            gas: 0,
            gas_cost: 0,
            depth: 1,
            refund: 5,
            stack: None,
            memory: None,
            storage: None,
            return_data: None,
            error: None,
        };
        let j = to_json(&log);
        assert_eq!(j["refund"], Value::Number(5.into()));
    }

    // ── StructLog — returnData field ──────────────────────────────────────

    #[test]
    fn return_data_none_is_absent() {
        let log = StructLog {
            pc: 0,
            op: 0x00,
            gas: 0,
            gas_cost: 0,
            depth: 1,
            refund: 0,
            stack: None,
            memory: None,
            storage: None,
            return_data: None,
            error: None,
        };
        let j = to_json(&log);
        assert!(j.get("returnData").is_none());
    }

    #[test]
    fn return_data_empty_bytes_is_absent() {
        let log = StructLog {
            pc: 0,
            op: 0x00,
            gas: 0,
            gas_cost: 0,
            depth: 1,
            refund: 0,
            stack: None,
            memory: None,
            storage: None,
            return_data: Some(Bytes::new()),
            error: None,
        };
        let j = to_json(&log);
        assert!(j.get("returnData").is_none());
    }

    #[test]
    fn return_data_nonempty_is_present() {
        let log = StructLog {
            pc: 0,
            op: 0x00,
            gas: 0,
            gas_cost: 0,
            depth: 1,
            refund: 0,
            stack: None,
            memory: None,
            storage: None,
            return_data: Some(Bytes::from_static(b"\x00\x01")),
            error: None,
        };
        let j = to_json(&log);
        assert_eq!(j["returnData"], Value::String("0x0001".to_string()));
    }

    // ── StructLogResult ───────────────────────────────────────────────────

    #[test]
    fn struct_log_result_shape() {
        let result = StructLogResult {
            gas: 21000,
            failed: false,
            return_value: Bytes::from_static(b"\x00\x01"),
            struct_logs: vec![],
        };
        let j = to_json(&result);
        assert_eq!(j["gas"], Value::Number(21000.into()));
        assert_eq!(j["failed"], Value::Bool(false));
        assert_eq!(j["returnValue"], Value::String("0x0001".to_string()));
        assert_eq!(j["structLogs"], Value::Array(vec![]));
    }

    // ── Full StructLog JSON shape (fixture-style) ─────────────────────────

    #[test]
    fn full_struct_log_fixture() {
        let mut storage = BTreeMap::new();
        storage.insert(H256::from_low_u64_be(1), H256::from_low_u64_be(0x2a));

        let log = StructLog {
            pc: 0,
            op: 0x60, // PUSH1
            gas: 30000,
            gas_cost: 3,
            depth: 1,
            refund: 0,
            stack: Some(vec![U256::zero(), U256::from(1u64)]),
            memory: Some(vec![MemoryChunk([0u8; 32])]),
            storage: Some(storage),
            return_data: None,
            error: None,
        };

        let j = to_json(&log);
        // Verify required fields are present with correct types
        assert_eq!(j["pc"], Value::Number(0.into()));
        assert_eq!(j["op"], Value::String("PUSH1".to_string()));
        assert_eq!(j["gas"], Value::Number(30000.into()));
        assert_eq!(j["gasCost"], Value::Number(3.into()));
        assert_eq!(j["depth"], Value::Number(1.into()));
        // refund absent (zero)
        assert!(j.get("refund").is_none());
        // stack present with two entries
        let stack = j["stack"].as_array().expect("stack");
        assert_eq!(stack.len(), 2);
        assert_eq!(stack[0], Value::String("0x0".to_string()));
        assert_eq!(stack[1], Value::String("0x1".to_string()));
        // memory present
        assert_eq!(j["memory"].as_array().expect("memory").len(), 1);
        // storage present
        assert!(j["storage"].as_object().is_some());
        // returnData absent (None)
        assert!(j.get("returnData").is_none());
        // error absent (None)
        assert!(j.get("error").is_none());

        // Emit the full JSON for manual inspection
        let s = serde_json::to_string(&log).expect("to_string");
        // Ensure it parses back
        let reparsed: Value = serde_json::from_str(&s).expect("reparse");
        assert_eq!(reparsed["op"], Value::String("PUSH1".to_string()));
    }
}
