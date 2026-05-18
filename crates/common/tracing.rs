//! Trace data types and their wire-format serializers.
//!
//! ## Architecture
//!
//! Capture, data, and output format are separated:
//!
//! - **Capture** lives in `ethrex-levm` (`LevmOpcodeTracer`, the dispatch-loop hook).
//!   It runs once per tx and produces a [`Vec<OpcodeStep>`] plus the trailing
//!   metadata in [`OpcodeTraceResult`].
//! - **Data** are the bare structs [`OpcodeStep`] and [`OpcodeTraceResult`] in this
//!   module. They carry no `Serialize` impl — they're consumer-agnostic. The same
//!   captured data feeds every downstream wire format.
//! - **Wire format** is a newtype wrapper around one of those data structs with its
//!   own `Serialize` impl. Two shapes coexist:
//!     - [`StructLoggerStep`] / [`StructLoggerResult`] — the geth-RPC `debug_traceTransaction`
//!       structLogger shape: `op` as string mnemonic, no `opName`, decimal `gas`, etc.
//!       Used by the RPC handler and matches what every major client (geth, besu, …) emits
//!       from this endpoint. Consumers: Blockscout, Foundry, Tenderly, anything reading
//!       `debug_traceTransaction`.
//!     - [`Eip3155Step`] — strict [EIP-3155](https://eips.ethereum.org/EIPS/eip-3155)
//!       shape: numeric `op` byte + separate `opName`, `"0xN"` hex `gas`/`gasCost`/`refund`,
//!       `stack:[]` (never null) when disabled. Used by streaming sinks that want
//!       spec-conformant per-step JSONL — e.g. the `ef-tests-statev2 statetest` subcommand
//!       feeding goevmlab.
//!
//! Adding a third format (Parity-style flat call, opcode-count tracers, …) means another
//! newtype with its own `Serialize` impl. No changes to the data types or capture layer.
//!
//! ## Why not match geth-RPC everywhere
//!
//! `debug_traceTransaction` predates EIP-3155 by years and its de-facto shape diverges
//! from the spec on three points: `op` is a string, `opName` is absent, and `gas`/`gasCost`
//! are decimal numbers instead of `"0xN"` hex strings. Every major client matches geth's
//! shape there for tooling compat, not EIP-3155. So:
//! - RPC consumer expects structLogger → use [`StructLoggerStep`]/[`StructLoggerResult`].
//! - EIP-3155-conformant CLI consumer (goevmlab, fuzzers) → use [`Eip3155Step`].

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

/// Per-opcode trace entry — pure data, no `Serialize` impl.
///
/// To get this on the wire, wrap in one of the format newtypes:
/// - [`StructLoggerStep`] for geth-RPC `debug_traceTransaction` shape.
/// - [`Eip3155Step`] for EIP-3155 spec shape.
///
/// See the module-level doc for why both formats coexist.
#[derive(Debug)]
pub struct OpcodeStep {
    pub pc: u64,
    /// Raw opcode byte value (e.g. 0x60 for PUSH1). Each format serializer decides
    /// how to render this (numeric byte, hex string, mnemonic string).
    pub op: u8,
    pub gas: u64,
    pub gas_cost: u64,
    /// Current memory size in bytes.
    pub mem_size: u64,
    pub depth: u32,
    /// Return data from the previous sub-call.
    pub return_data: bytes::Bytes,
    /// Gas refund counter.
    pub refund: u64,
    /// `Some(vec)` when stack capture is enabled (bottom-first); `None` when disabled.
    /// Each format serializer decides how to render `None`: structLogger emits JSON null,
    /// EIP-3155 emits `[]` (per spec's "MUST initialize to empty array" rule).
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

/// Top-level result of one opcode-traced transaction — pure data, no `Serialize` impl.
///
/// Wrap in [`StructLoggerResult`] to get the geth-RPC `{failed, gas, returnValue, structLogs}`
/// wire shape. EIP-3155-conformant CLI consumers stream per-step [`OpcodeStep`]s
/// directly (via [`Eip3155Step`]) and emit their own summary line, so there's no
/// EIP-3155 wrapper newtype for the result.
#[derive(Debug)]
pub struct OpcodeTraceResult {
    pub gas_used: u64,
    /// True iff the transaction completed without error.
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

// Shared utilities used by both wire-format serializers below.

fn serialize_storage_map<S: serde::Serializer>(
    serializer: S,
    storage: &BTreeMap<H256, H256>,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeMap;
    let mut m = serializer.serialize_map(Some(storage.len()))?;
    for (k, v) in storage {
        let k_str = format!("0x{}", hex::encode(k.as_bytes()));
        let v_str = format!("0x{}", hex::encode(v.as_bytes()));
        m.serialize_entry(&k_str, &v_str)?;
    }
    m.end()
}

/// Mnemonic string for an opcode byte, falling back to `"opcode 0xNN not defined"`
/// for bytes outside the assigned table.
fn opcode_name_or_fallback(byte: u8) -> String {
    opcode_name(byte)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("opcode 0x{byte:02x} not defined"))
}

// ─── Wire format: geth-RPC structLogger ───────────────────────────────────
//
// The de-facto `debug_traceTransaction` response shape, emitted by every major
// execution client (geth, besu, reth, erigon, nethermind). Predates EIP-3155
// and diverges from it on three per-step fields:
//
//   - `op`: string mnemonic (`"PUSH1"`), not the numeric opcode byte.
//   - No separate `opName` field.
//   - `gas`, `gasCost`, `refund`: decimal JSON numbers, not `"0xN"` hex strings.
//
// `stack` is serialized as JSON `null` when capture is disabled — also a divergence
// from EIP-3155, which mandates `[]` — but it matches geth's RPC behavior so we
// preserve it on this code path.
//
// Verified against geth and besu on a kurtosis localnet via `debug_traceTransaction`:
// byte-for-byte identical to the StructLogger output.

/// Controls which always-populated per-step fields the structLogger wire format emits.
///
/// `mem_size`, `return_data`, and `refund` are always present in the captured
/// [`OpcodeStep`] (the capture layer just defaults them to zero/empty when the
/// corresponding capture config is off). geth's `debug_traceTransaction` *suppresses*
/// these fields unless their data is actually captured. To match geth byte-for-byte
/// we honor the caller's intent explicitly here.
///
/// Typical mapping at the RPC layer:
///
/// ```ignore
/// let emit = StructLoggerEmit {
///     mem_size: cfg.enable_memory,        // memSize travels with memory
///     return_data: cfg.enable_return_data,
///     refund: false,                      // no equivalent geth flag; off by default
/// };
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct StructLoggerEmit {
    /// Emit `memSize` even when its value is meaningful at every step.
    /// Geth ties this to memory capture; default `false` matches geth's default config.
    pub mem_size: bool,
    /// Emit `returnData` (as `"0x..."` hex). Default `false` matches geth.
    pub return_data: bool,
    /// Emit `refund` (decimal number). Default `false` matches geth's empirical output.
    pub refund: bool,
}

/// Wraps an [`OpcodeStep`] to serialize in the geth-RPC `structLogger` shape used by
/// `debug_traceTransaction`. See module-level docs and the comment above this type
/// for the field-shape divergences from EIP-3155.
pub struct StructLoggerStep<'a> {
    pub step: &'a OpcodeStep,
    pub emit: StructLoggerEmit,
}

impl serde::Serialize for StructLoggerStep<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let step = self.step;
        let emit = self.emit;

        // pc, op, gas, gasCost, depth, stack are always emitted (6 base fields).
        let mut field_count = 6;
        if emit.mem_size {
            field_count += 1;
        }
        if emit.return_data {
            field_count += 1;
        }
        if emit.refund {
            field_count += 1;
        }
        if step.error.is_some() {
            field_count += 1;
        }
        if step.memory.is_some() {
            field_count += 1;
        }
        if step.storage.is_some() {
            field_count += 1;
        }

        let mut map = serializer.serialize_map(Some(field_count))?;

        map.serialize_entry("pc", &step.pc)?;
        // op: string mnemonic, matching geth's wire output (NOT EIP-3155's numeric form).
        map.serialize_entry("op", &opcode_name_or_fallback(step.op))?;
        // gas/gasCost/refund: decimal JSON numbers, matching geth's wire output.
        map.serialize_entry("gas", &step.gas)?;
        map.serialize_entry("gasCost", &step.gas_cost)?;
        map.serialize_entry("depth", &step.depth)?;

        // stack: JSON null when disabled, array of `"0xN"` hex strings when enabled.
        // Matches geth's RPC behavior; diverges from EIP-3155's "MUST be []" rule.
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
        map.serialize_entry("stack", &StackSerializer(&step.stack))?;

        if emit.mem_size {
            map.serialize_entry("memSize", &step.mem_size)?;
        }
        if emit.return_data {
            map.serialize_entry(
                "returnData",
                &format!("0x{}", hex::encode(&step.return_data)),
            )?;
        }
        if emit.refund {
            map.serialize_entry("refund", &step.refund)?;
        }

        if let Some(err) = &step.error {
            map.serialize_entry("error", err)?;
        }
        if let Some(mem) = &step.memory {
            map.serialize_entry("memory", mem)?;
        }
        if let Some(storage) = &step.storage {
            struct Wrap<'a>(&'a BTreeMap<H256, H256>);
            impl serde::Serialize for Wrap<'_> {
                fn serialize<S: serde::Serializer>(
                    &self,
                    serializer: S,
                ) -> Result<S::Ok, S::Error> {
                    serialize_storage_map(serializer, self.0)
                }
            }
            map.serialize_entry("storage", &Wrap(storage))?;
        }

        map.end()
    }
}

/// Wraps an [`OpcodeTraceResult`] to serialize as the geth-RPC `debug_traceTransaction`
/// response: `{failed, gas, returnValue, structLogs: [...]}`. Each step inside
/// `structLogs` is itself serialized via [`StructLoggerStep`] using the same `emit` flags.
pub struct StructLoggerResult<'a> {
    pub result: &'a OpcodeTraceResult,
    pub emit: StructLoggerEmit,
}

impl serde::Serialize for StructLoggerResult<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{SerializeMap, SerializeSeq};
        let r = self.result;
        let emit = self.emit;

        // structLogs uses StructLoggerStep for each entry, with the same emit options.
        struct Steps<'a> {
            steps: &'a [OpcodeStep],
            emit: StructLoggerEmit,
        }
        impl serde::Serialize for Steps<'_> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let mut seq = serializer.serialize_seq(Some(self.steps.len()))?;
                for s in self.steps {
                    seq.serialize_element(&StructLoggerStep {
                        step: s,
                        emit: self.emit,
                    })?;
                }
                seq.end()
            }
        }

        let mut map = serializer.serialize_map(Some(4))?;
        // `failed` is the inverse of `pass` — matches the geth wire shape.
        map.serialize_entry("failed", &!r.pass)?;
        map.serialize_entry("gas", &r.gas_used)?;
        map.serialize_entry("returnValue", &format!("0x{}", hex::encode(&r.output)))?;
        map.serialize_entry(
            "structLogs",
            &Steps {
                steps: &r.steps,
                emit,
            },
        )?;
        map.end()
    }
}

// ─── Wire format: EIP-3155 ────────────────────────────────────────────────
//
// The shape defined by EIP-3155 §"Required Fields":
//
//   - `op`: numeric opcode byte (e.g. `96` for PUSH1).
//   - `opName`: separate string mnemonic, always emitted (technically optional per spec).
//   - `gas`, `gasCost`, `refund`: `"0xN"` hex strings ("Hex-Number" per spec).
//   - `stack`: always an array, never null (spec: "All array attributes MUST be
//     initialized to empty arrays NOT to null").
//
// Field order matches the spec's listed order. Used by streaming sinks that feed
// EIP-3155-conformant tooling (goevmlab, fuzzers). NOT used by `debug_traceTransaction`,
// where existing tooling expects the structLogger shape above.

/// Wraps an [`OpcodeStep`] to serialize in strict EIP-3155 shape. See module-level
/// docs and the comment above this type for the field-shape choices.
pub struct Eip3155Step<'a>(pub &'a OpcodeStep);

impl serde::Serialize for Eip3155Step<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let step = self.0;

        let mut field_count = 10; // required 9 + always-emitted opName
        if step.error.is_some() {
            field_count += 1;
        }
        if step.memory.is_some() {
            field_count += 1;
        }
        if step.storage.is_some() {
            field_count += 1;
        }

        let mut map = serializer.serialize_map(Some(field_count))?;

        // Required fields in spec order.
        map.serialize_entry("pc", &step.pc)?;
        map.serialize_entry("op", &step.op)?;
        map.serialize_entry("gas", &format!("{:#x}", step.gas))?;
        map.serialize_entry("gasCost", &format!("{:#x}", step.gas_cost))?;
        map.serialize_entry("memSize", &step.mem_size)?;

        // stack: always an array; `None` (disabled) becomes `[]`.
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
        map.serialize_entry("stack", &StackSerializer(&step.stack))?;

        map.serialize_entry("depth", &step.depth)?;
        map.serialize_entry(
            "returnData",
            &format!("0x{}", hex::encode(&step.return_data)),
        )?;
        map.serialize_entry("refund", &format!("{:#x}", step.refund))?;

        // Optional fields in spec order: opName, error, memory, storage.
        // opName always emitted (covers both known and unknown opcode bytes).
        map.serialize_entry("opName", &opcode_name_or_fallback(step.op))?;

        if let Some(err) = &step.error {
            map.serialize_entry("error", err)?;
        }
        if let Some(mem) = &step.memory {
            map.serialize_entry("memory", mem)?;
        }
        if let Some(storage) = &step.storage {
            struct Wrap<'a>(&'a BTreeMap<H256, H256>);
            impl serde::Serialize for Wrap<'_> {
                fn serialize<S: serde::Serializer>(
                    &self,
                    serializer: S,
                ) -> Result<S::Ok, S::Error> {
                    serialize_storage_map(serializer, self.0)
                }
            }
            map.serialize_entry("storage", &Wrap(storage))?;
        }

        map.end()
    }
}
