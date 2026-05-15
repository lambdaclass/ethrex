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

// ─── OpcodeTracer types ──────────────────────────────────────────────────────

/// Per-opcode trace entry conforming to [EIP-3155](https://eips.ethereum.org/EIPS/eip-3155).
///
/// Wire format per the spec: `op` is a numeric opcode byte; `gas`, `gasCost`, `refund`
/// are `"0xN"` hex strings ("Hex-Number"); `pc`, `memSize`, `depth` are plain JSON
/// numbers; `stack` is always an array (never null) of `"0xN"` hex strings. The
/// optional `opName`, `error`, `memory`, `storage` fields follow when populated.
/// Field order matches the spec's listed order.
///
/// When emitted via geth's `debug_traceTransaction` RPC, this struct lives inside
/// the geth-specific `{failed, gas, returnValue, structLogs}` wrapper
/// ([`OpcodeTraceResult`]); when emitted via an EIP-3155 streaming sink, it stands
/// alone as a JSONL line.
#[derive(Debug)]
pub struct OpcodeStep {
    pub pc: u64,
    /// Raw opcode byte value (e.g. 0x60 for PUSH1). Emitted as a JSON number under
    /// the `"op"` key; the mnemonic is emitted separately as `"opName"`.
    pub op: u8,
    pub gas: u64,
    pub gas_cost: u64,
    /// Current memory size in bytes (always emitted).
    pub mem_size: u64,
    pub depth: u32,
    /// Return data from the previous sub-call (always emitted; `"0x"` when disabled or empty).
    pub return_data: bytes::Bytes,
    /// Gas refund counter (always emitted).
    pub refund: u64,
    /// `Some(vec)` when stack capture is enabled (bottom-first); `None` when disabled
    /// (still serialized as `[]` per EIP-3155's "MUST initialize to empty array" rule).
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

/// Top-level result returned by an opcode (EIP-3155) trace.
///
/// Wraps per-step entries as `{failed, gas, returnValue, structLogs}` matching
/// the de-facto `debug_traceTransaction` response shape used across major
/// execution clients.
#[derive(Debug)]
pub struct OpcodeTraceResult {
    pub gas_used: u64,
    /// True iff the transaction completed without error. Serialized as the
    /// inverted `failed` field on the wire.
    pub pass: bool,
    pub output: bytes::Bytes,
    pub steps: Vec<OpcodeStep>,
}

// ─── Helpers ──────────────────────────────────────────────────────────────

/// Returns the opcode mnemonic for `byte`.
///
/// Known opcodes → their uppercase name (`"PUSH1"`, `"ADD"`, `"INVALID"` for
/// 0xFE). Unassigned bytes → `None`; callers wanting the conventional unknown
/// string should fall back to `format!("opcode 0x{:02x} not defined", byte)`.
///
/// The table is **fork-agnostic by design**, matching geth's
/// `core/vm/opcodes.go::opCodeToString` (also a flat 256-entry table). Fork
/// validity is enforced at *dispatch* via the VM's per-fork opcode table:
/// e.g. byte `0x5F` (PUSH0) halts pre-Shanghai with `InvalidOpcode` before
/// the tracer ever emits a step for it, so the name lookup never fires for
/// invalid-for-this-fork bytes in practice.
pub fn opcode_name(byte: u8) -> Option<&'static str> {
    match byte {
        0x00 => Some("STOP"),
        0x01 => Some("ADD"),
        0x02 => Some("MUL"),
        0x03 => Some("SUB"),
        0x04 => Some("DIV"),
        0x05 => Some("SDIV"),
        0x06 => Some("MOD"),
        0x07 => Some("SMOD"),
        0x08 => Some("ADDMOD"),
        0x09 => Some("MULMOD"),
        0x0A => Some("EXP"),
        0x0B => Some("SIGNEXTEND"),
        0x10 => Some("LT"),
        0x11 => Some("GT"),
        0x12 => Some("SLT"),
        0x13 => Some("SGT"),
        0x14 => Some("EQ"),
        0x15 => Some("ISZERO"),
        0x16 => Some("AND"),
        0x17 => Some("OR"),
        0x18 => Some("XOR"),
        0x19 => Some("NOT"),
        0x1A => Some("BYTE"),
        0x1B => Some("SHL"),
        0x1C => Some("SHR"),
        0x1D => Some("SAR"),
        0x1E => Some("CLZ"),
        0x20 => Some("KECCAK256"),
        0x30 => Some("ADDRESS"),
        0x31 => Some("BALANCE"),
        0x32 => Some("ORIGIN"),
        0x33 => Some("CALLER"),
        0x34 => Some("CALLVALUE"),
        0x35 => Some("CALLDATALOAD"),
        0x36 => Some("CALLDATASIZE"),
        0x37 => Some("CALLDATACOPY"),
        0x38 => Some("CODESIZE"),
        0x39 => Some("CODECOPY"),
        0x3A => Some("GASPRICE"),
        0x3B => Some("EXTCODESIZE"),
        0x3C => Some("EXTCODECOPY"),
        0x3D => Some("RETURNDATASIZE"),
        0x3E => Some("RETURNDATACOPY"),
        0x3F => Some("EXTCODEHASH"),
        0x40 => Some("BLOCKHASH"),
        0x41 => Some("COINBASE"),
        0x42 => Some("TIMESTAMP"),
        0x43 => Some("NUMBER"),
        0x44 => Some("PREVRANDAO"),
        0x45 => Some("GASLIMIT"),
        0x46 => Some("CHAINID"),
        0x47 => Some("SELFBALANCE"),
        0x48 => Some("BASEFEE"),
        0x49 => Some("BLOBHASH"),
        0x4A => Some("BLOBBASEFEE"),
        0x4B => Some("SLOTNUM"),
        0x50 => Some("POP"),
        0x51 => Some("MLOAD"),
        0x52 => Some("MSTORE"),
        0x53 => Some("MSTORE8"),
        0x54 => Some("SLOAD"),
        0x55 => Some("SSTORE"),
        0x56 => Some("JUMP"),
        0x57 => Some("JUMPI"),
        0x58 => Some("PC"),
        0x59 => Some("MSIZE"),
        0x5A => Some("GAS"),
        0x5B => Some("JUMPDEST"),
        0x5C => Some("TLOAD"),
        0x5D => Some("TSTORE"),
        0x5E => Some("MCOPY"),
        0x5F => Some("PUSH0"),
        0x60 => Some("PUSH1"),
        0x61 => Some("PUSH2"),
        0x62 => Some("PUSH3"),
        0x63 => Some("PUSH4"),
        0x64 => Some("PUSH5"),
        0x65 => Some("PUSH6"),
        0x66 => Some("PUSH7"),
        0x67 => Some("PUSH8"),
        0x68 => Some("PUSH9"),
        0x69 => Some("PUSH10"),
        0x6A => Some("PUSH11"),
        0x6B => Some("PUSH12"),
        0x6C => Some("PUSH13"),
        0x6D => Some("PUSH14"),
        0x6E => Some("PUSH15"),
        0x6F => Some("PUSH16"),
        0x70 => Some("PUSH17"),
        0x71 => Some("PUSH18"),
        0x72 => Some("PUSH19"),
        0x73 => Some("PUSH20"),
        0x74 => Some("PUSH21"),
        0x75 => Some("PUSH22"),
        0x76 => Some("PUSH23"),
        0x77 => Some("PUSH24"),
        0x78 => Some("PUSH25"),
        0x79 => Some("PUSH26"),
        0x7A => Some("PUSH27"),
        0x7B => Some("PUSH28"),
        0x7C => Some("PUSH29"),
        0x7D => Some("PUSH30"),
        0x7E => Some("PUSH31"),
        0x7F => Some("PUSH32"),
        0x80 => Some("DUP1"),
        0x81 => Some("DUP2"),
        0x82 => Some("DUP3"),
        0x83 => Some("DUP4"),
        0x84 => Some("DUP5"),
        0x85 => Some("DUP6"),
        0x86 => Some("DUP7"),
        0x87 => Some("DUP8"),
        0x88 => Some("DUP9"),
        0x89 => Some("DUP10"),
        0x8A => Some("DUP11"),
        0x8B => Some("DUP12"),
        0x8C => Some("DUP13"),
        0x8D => Some("DUP14"),
        0x8E => Some("DUP15"),
        0x8F => Some("DUP16"),
        0x90 => Some("SWAP1"),
        0x91 => Some("SWAP2"),
        0x92 => Some("SWAP3"),
        0x93 => Some("SWAP4"),
        0x94 => Some("SWAP5"),
        0x95 => Some("SWAP6"),
        0x96 => Some("SWAP7"),
        0x97 => Some("SWAP8"),
        0x98 => Some("SWAP9"),
        0x99 => Some("SWAP10"),
        0x9A => Some("SWAP11"),
        0x9B => Some("SWAP12"),
        0x9C => Some("SWAP13"),
        0x9D => Some("SWAP14"),
        0x9E => Some("SWAP15"),
        0x9F => Some("SWAP16"),
        0xA0 => Some("LOG0"),
        0xA1 => Some("LOG1"),
        0xA2 => Some("LOG2"),
        0xA3 => Some("LOG3"),
        0xA4 => Some("LOG4"),
        0xE6 => Some("DUPN"),
        0xE7 => Some("SWAPN"),
        0xE8 => Some("EXCHANGE"),
        0xF0 => Some("CREATE"),
        0xF1 => Some("CALL"),
        0xF2 => Some("CALLCODE"),
        0xF3 => Some("RETURN"),
        0xF4 => Some("DELEGATECALL"),
        0xF5 => Some("CREATE2"),
        0xFA => Some("STATICCALL"),
        0xFD => Some("REVERT"),
        0xFE => Some("INVALID"),
        0xFF => Some("SELFDESTRUCT"),
        _ => None,
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

        // Required: pc, op, gas, gasCost, memSize, stack, depth, returnData, refund = 9
        // Always-emitted optional: opName = 1
        // Conditional optional: error, memory, storage
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

        // Required fields, in EIP-3155 spec order.
        map.serialize_entry("pc", &self.pc)?;
        // op: numeric opcode byte (spec: Number).
        map.serialize_entry("op", &self.op)?;
        // gas, gasCost, refund: spec type "Hex-Number" — JSON string of form "0xN".
        map.serialize_entry("gas", &format!("{:#x}", self.gas))?;
        map.serialize_entry("gasCost", &format!("{:#x}", self.gas_cost))?;
        map.serialize_entry("memSize", &self.mem_size)?;

        // stack: always an array (spec: "MUST be initialized to empty arrays NOT to null").
        // Bottom-first ordering, U256 values formatted as "0xN" via `geth_uint256_hex`.
        struct StackSerializer<'a>(&'a Option<Vec<U256>>);
        impl serde::Serialize for StackSerializer<'_> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                use serde::ser::SerializeSeq;
                let vec_ref: &[U256] = self.0.as_deref().unwrap_or(&[]);
                let mut seq = serializer.serialize_seq(Some(vec_ref.len()))?;
                for v in vec_ref {
                    seq.serialize_element(&geth_uint256_hex(v))?;
                }
                seq.end()
            }
        }
        map.serialize_entry("stack", &StackSerializer(&self.stack))?;

        map.serialize_entry("depth", &self.depth)?;
        map.serialize_entry(
            "returnData",
            &format!("0x{}", hex::encode(&self.return_data)),
        )?;
        map.serialize_entry("refund", &format!("{:#x}", self.refund))?;

        // Optional fields, in EIP-3155 spec order: opName, error, memory, storage.
        // opName always emitted: every byte has either a known mnemonic or a stable
        // "opcode 0xNN not defined" fallback.
        match opcode_name(self.op) {
            Some(name) => map.serialize_entry("opName", name)?,
            None => {
                map.serialize_entry("opName", &format!("opcode 0x{:02x} not defined", self.op))?
            }
        }

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
        // `failed` is the inverse of `pass` — matches the conventional wire shape.
        map.serialize_entry("failed", &!self.pass)?;
        map.serialize_entry("gas", &self.gas_used)?;
        map.serialize_entry("returnValue", &format!("0x{}", hex::encode(&self.output)))?;
        map.serialize_entry("structLogs", &self.steps)?;
        map.end()
    }
}
