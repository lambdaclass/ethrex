use bytes::Bytes;
use ethereum_types::H256;
use ethereum_types::{Address, U256};
use serde::Serialize;
use std::borrow::Cow;
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

// ─── EIP-3155 OpcodeTracer types ──────────────────────────────────────────────

/// Per-opcode trace entry in strict EIP-3155 format.
///
/// Fields are kept as native types in memory; `Serialize` converts them to the
/// exact encoding specified by EIP-3155 (https://eips.ethereum.org/EIPS/eip-3155).
#[derive(Debug)]
pub struct OpcodeStep {
    pub pc: u64,
    /// Raw opcode byte value (e.g. 96 for PUSH1).
    pub op: u8,
    pub gas: u64,
    pub gas_cost: u64,
    /// Current memory size in bytes (always emitted).
    pub mem_size: u64,
    pub depth: u32,
    /// Return data from the previous sub-call (always emitted; `"0x"` when disabled or empty).
    pub return_data: bytes::Bytes,
    /// Gas refund counter (always emitted; `"0x0"` when zero).
    pub refund: u64,
    /// `Some(vec)` when stack capture is enabled (bottom-first); `None` when disabled (emits JSON null).
    pub stack: Option<Vec<U256>>,
    /// `Some(chunks)` when memory capture is enabled; `None` when disabled (field omitted).
    pub memory: Option<Vec<MemoryChunk>>,
    /// `Some(map)` at SLOAD/SSTORE steps when storage capture is enabled (single entry); `None` otherwise.
    pub storage: Option<BTreeMap<H256, H256>>,
    pub error: Option<String>,
}

/// A 32-byte chunk of EVM memory, serialized as `"0x" + 64 lowercase hex chars`.
/// The *caller* zero-pads the last partial chunk before constructing this type.
#[derive(Debug)]
pub struct MemoryChunk(pub [u8; 32]);

/// Top-level result returned by an opcode trace, in EIP-3155 format.
#[derive(Debug)]
pub struct OpcodeTraceResult {
    pub gas_used: u64,
    pub pass: bool,
    pub output: bytes::Bytes,
    pub steps: Vec<OpcodeStep>,
}

// ─── Helpers ──────────────────────────────────────────────────────────────

/// Returns the EIP-3155 opcode mnemonic for `byte`.
///
/// `0xFE` → `"INVALID"`.  All assigned opcodes → their uppercase name
/// (e.g. `"PUSH1"`, `"ADD"`).  Unassigned bytes → `"opcode 0xNN"` (lowercase
/// hex, two digits).
pub fn opcode_name(byte: u8) -> Cow<'static, str> {
    match byte {
        0x00 => Cow::Borrowed("STOP"),
        0x01 => Cow::Borrowed("ADD"),
        0x02 => Cow::Borrowed("MUL"),
        0x03 => Cow::Borrowed("SUB"),
        0x04 => Cow::Borrowed("DIV"),
        0x05 => Cow::Borrowed("SDIV"),
        0x06 => Cow::Borrowed("MOD"),
        0x07 => Cow::Borrowed("SMOD"),
        0x08 => Cow::Borrowed("ADDMOD"),
        0x09 => Cow::Borrowed("MULMOD"),
        0x0A => Cow::Borrowed("EXP"),
        0x0B => Cow::Borrowed("SIGNEXTEND"),
        0x10 => Cow::Borrowed("LT"),
        0x11 => Cow::Borrowed("GT"),
        0x12 => Cow::Borrowed("SLT"),
        0x13 => Cow::Borrowed("SGT"),
        0x14 => Cow::Borrowed("EQ"),
        0x15 => Cow::Borrowed("ISZERO"),
        0x16 => Cow::Borrowed("AND"),
        0x17 => Cow::Borrowed("OR"),
        0x18 => Cow::Borrowed("XOR"),
        0x19 => Cow::Borrowed("NOT"),
        0x1A => Cow::Borrowed("BYTE"),
        0x1B => Cow::Borrowed("SHL"),
        0x1C => Cow::Borrowed("SHR"),
        0x1D => Cow::Borrowed("SAR"),
        0x20 => Cow::Borrowed("KECCAK256"),
        0x30 => Cow::Borrowed("ADDRESS"),
        0x31 => Cow::Borrowed("BALANCE"),
        0x32 => Cow::Borrowed("ORIGIN"),
        0x33 => Cow::Borrowed("CALLER"),
        0x34 => Cow::Borrowed("CALLVALUE"),
        0x35 => Cow::Borrowed("CALLDATALOAD"),
        0x36 => Cow::Borrowed("CALLDATASIZE"),
        0x37 => Cow::Borrowed("CALLDATACOPY"),
        0x38 => Cow::Borrowed("CODESIZE"),
        0x39 => Cow::Borrowed("CODECOPY"),
        0x3A => Cow::Borrowed("GASPRICE"),
        0x3B => Cow::Borrowed("EXTCODESIZE"),
        0x3C => Cow::Borrowed("EXTCODECOPY"),
        0x3D => Cow::Borrowed("RETURNDATASIZE"),
        0x3E => Cow::Borrowed("RETURNDATACOPY"),
        0x3F => Cow::Borrowed("EXTCODEHASH"),
        0x40 => Cow::Borrowed("BLOCKHASH"),
        0x41 => Cow::Borrowed("COINBASE"),
        0x42 => Cow::Borrowed("TIMESTAMP"),
        0x43 => Cow::Borrowed("NUMBER"),
        0x44 => Cow::Borrowed("PREVRANDAO"),
        0x45 => Cow::Borrowed("GASLIMIT"),
        0x46 => Cow::Borrowed("CHAINID"),
        0x47 => Cow::Borrowed("SELFBALANCE"),
        0x48 => Cow::Borrowed("BASEFEE"),
        0x49 => Cow::Borrowed("BLOBHASH"),
        0x4A => Cow::Borrowed("BLOBBASEFEE"),
        0x50 => Cow::Borrowed("POP"),
        0x51 => Cow::Borrowed("MLOAD"),
        0x52 => Cow::Borrowed("MSTORE"),
        0x53 => Cow::Borrowed("MSTORE8"),
        0x54 => Cow::Borrowed("SLOAD"),
        0x55 => Cow::Borrowed("SSTORE"),
        0x56 => Cow::Borrowed("JUMP"),
        0x57 => Cow::Borrowed("JUMPI"),
        0x58 => Cow::Borrowed("PC"),
        0x59 => Cow::Borrowed("MSIZE"),
        0x5A => Cow::Borrowed("GAS"),
        0x5B => Cow::Borrowed("JUMPDEST"),
        0x5C => Cow::Borrowed("TLOAD"),
        0x5D => Cow::Borrowed("TSTORE"),
        0x5E => Cow::Borrowed("MCOPY"),
        0x5F => Cow::Borrowed("PUSH0"),
        0x60 => Cow::Borrowed("PUSH1"),
        0x61 => Cow::Borrowed("PUSH2"),
        0x62 => Cow::Borrowed("PUSH3"),
        0x63 => Cow::Borrowed("PUSH4"),
        0x64 => Cow::Borrowed("PUSH5"),
        0x65 => Cow::Borrowed("PUSH6"),
        0x66 => Cow::Borrowed("PUSH7"),
        0x67 => Cow::Borrowed("PUSH8"),
        0x68 => Cow::Borrowed("PUSH9"),
        0x69 => Cow::Borrowed("PUSH10"),
        0x6A => Cow::Borrowed("PUSH11"),
        0x6B => Cow::Borrowed("PUSH12"),
        0x6C => Cow::Borrowed("PUSH13"),
        0x6D => Cow::Borrowed("PUSH14"),
        0x6E => Cow::Borrowed("PUSH15"),
        0x6F => Cow::Borrowed("PUSH16"),
        0x70 => Cow::Borrowed("PUSH17"),
        0x71 => Cow::Borrowed("PUSH18"),
        0x72 => Cow::Borrowed("PUSH19"),
        0x73 => Cow::Borrowed("PUSH20"),
        0x74 => Cow::Borrowed("PUSH21"),
        0x75 => Cow::Borrowed("PUSH22"),
        0x76 => Cow::Borrowed("PUSH23"),
        0x77 => Cow::Borrowed("PUSH24"),
        0x78 => Cow::Borrowed("PUSH25"),
        0x79 => Cow::Borrowed("PUSH26"),
        0x7A => Cow::Borrowed("PUSH27"),
        0x7B => Cow::Borrowed("PUSH28"),
        0x7C => Cow::Borrowed("PUSH29"),
        0x7D => Cow::Borrowed("PUSH30"),
        0x7E => Cow::Borrowed("PUSH31"),
        0x7F => Cow::Borrowed("PUSH32"),
        0x80 => Cow::Borrowed("DUP1"),
        0x81 => Cow::Borrowed("DUP2"),
        0x82 => Cow::Borrowed("DUP3"),
        0x83 => Cow::Borrowed("DUP4"),
        0x84 => Cow::Borrowed("DUP5"),
        0x85 => Cow::Borrowed("DUP6"),
        0x86 => Cow::Borrowed("DUP7"),
        0x87 => Cow::Borrowed("DUP8"),
        0x88 => Cow::Borrowed("DUP9"),
        0x89 => Cow::Borrowed("DUP10"),
        0x8A => Cow::Borrowed("DUP11"),
        0x8B => Cow::Borrowed("DUP12"),
        0x8C => Cow::Borrowed("DUP13"),
        0x8D => Cow::Borrowed("DUP14"),
        0x8E => Cow::Borrowed("DUP15"),
        0x8F => Cow::Borrowed("DUP16"),
        0x90 => Cow::Borrowed("SWAP1"),
        0x91 => Cow::Borrowed("SWAP2"),
        0x92 => Cow::Borrowed("SWAP3"),
        0x93 => Cow::Borrowed("SWAP4"),
        0x94 => Cow::Borrowed("SWAP5"),
        0x95 => Cow::Borrowed("SWAP6"),
        0x96 => Cow::Borrowed("SWAP7"),
        0x97 => Cow::Borrowed("SWAP8"),
        0x98 => Cow::Borrowed("SWAP9"),
        0x99 => Cow::Borrowed("SWAP10"),
        0x9A => Cow::Borrowed("SWAP11"),
        0x9B => Cow::Borrowed("SWAP12"),
        0x9C => Cow::Borrowed("SWAP13"),
        0x9D => Cow::Borrowed("SWAP14"),
        0x9E => Cow::Borrowed("SWAP15"),
        0x9F => Cow::Borrowed("SWAP16"),
        0xA0 => Cow::Borrowed("LOG0"),
        0xA1 => Cow::Borrowed("LOG1"),
        0xA2 => Cow::Borrowed("LOG2"),
        0xA3 => Cow::Borrowed("LOG3"),
        0xA4 => Cow::Borrowed("LOG4"),
        0xE6 => Cow::Borrowed("DUPN"),
        0xE7 => Cow::Borrowed("SWAPN"),
        0xE8 => Cow::Borrowed("EXCHANGE"),
        0xF0 => Cow::Borrowed("CREATE"),
        0xF1 => Cow::Borrowed("CALL"),
        0xF2 => Cow::Borrowed("CALLCODE"),
        0xF3 => Cow::Borrowed("RETURN"),
        0xF4 => Cow::Borrowed("DELEGATECALL"),
        0xF5 => Cow::Borrowed("CREATE2"),
        0xFA => Cow::Borrowed("STATICCALL"),
        0xFD => Cow::Borrowed("REVERT"),
        0xFE => Cow::Borrowed("INVALID"),
        0xFF => Cow::Borrowed("SELFDESTRUCT"),
        b => Cow::Owned(format!("opcode 0x{:02x}", b)),
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

// ─── Serialize impls ──────────────────────────────────────────────────────

impl serde::Serialize for MemoryChunk {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("0x{}", hex::encode(self.0)))
    }
}

impl serde::Serialize for OpcodeStep {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;

        // Required fields: pc, op, gas, gasCost, memSize, stack, depth, returnData, refund, opName = 10
        // Optional: error, memory, storage
        let mut field_count = 10;
        if self.error.is_some() {
            field_count += 1;
        }
        if self.memory.is_some() {
            field_count += 1;
        }
        if self.storage.is_some() {
            field_count += 1;
        }

        let mut map = serializer.serialize_map(Some(field_count))?;

        map.serialize_entry("pc", &self.pc)?;
        map.serialize_entry("op", &self.op)?;
        map.serialize_entry("gas", &format!("{:#x}", self.gas))?;
        map.serialize_entry("gasCost", &format!("{:#x}", self.gas_cost))?;
        map.serialize_entry("memSize", &self.mem_size)?;

        // stack: Some → array of hex strings; None → JSON null (required field)
        struct StackSerializer<'a>(&'a Option<Vec<U256>>);
        impl serde::Serialize for StackSerializer<'_> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                use serde::ser::SerializeSeq;
                match self.0 {
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
        }
        map.serialize_entry("stack", &StackSerializer(&self.stack))?;

        map.serialize_entry("depth", &self.depth)?;
        map.serialize_entry(
            "returnData",
            &format!("0x{}", hex::encode(&self.return_data)),
        )?;
        map.serialize_entry("refund", &format!("{:#x}", self.refund))?;
        map.serialize_entry("opName", &opcode_name(self.op))?;

        if let Some(err) = &self.error {
            map.serialize_entry("error", err)?;
        }

        if let Some(mem) = &self.memory {
            map.serialize_entry("memory", mem)?;
        }

        if let Some(storage) = &self.storage {
            struct StorageSerializer<'a>(&'a BTreeMap<H256, H256>);
            impl serde::Serialize for StorageSerializer<'_> {
                fn serialize<S: serde::Serializer>(
                    &self,
                    serializer: S,
                ) -> Result<S::Ok, S::Error> {
                    use serde::ser::SerializeMap;
                    let mut m = serializer.serialize_map(Some(self.0.len()))?;
                    for (k, v) in self.0 {
                        let k_str = format!("0x{}", hex::encode(k.as_bytes()));
                        let v_str = format!("0x{}", hex::encode(v.as_bytes()));
                        m.serialize_entry(&k_str, &v_str)?;
                    }
                    m.end()
                }
            }
            map.serialize_entry("storage", &StorageSerializer(storage))?;
        }

        map.end()
    }
}

impl serde::Serialize for OpcodeTraceResult {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(4))?;
        map.serialize_entry("pass", &self.pass)?;
        map.serialize_entry("gasUsed", &format!("{:#x}", self.gas_used))?;
        map.serialize_entry("output", &format!("0x{}", hex::encode(&self.output)))?;
        map.serialize_entry("steps", &self.steps)?;
        map.end()
    }
}
