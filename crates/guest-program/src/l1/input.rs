//! `ProgramInput` is a `struct` without the `eip-8025` feature and an `enum`
//! with it. The `new(...)` constructor and `Default` exist under both, but
//! pattern-matching on `Wire(...)`/`Direct { .. }` only compiles when
//! `eip-8025` is on.

use ethrex_common::types::Block;
use ethrex_common::types::block_execution_witness::ExecutionWitness;

/// Input for the L1 stateless validation program.
#[cfg(not(feature = "eip-8025"))]
#[derive(
    Clone,
    Default,
    serde::Serialize,
    serde::Deserialize,
    rkyv::Deserialize,
    rkyv::Serialize,
    rkyv::Archive,
)]
pub struct ProgramInput {
    /// Blocks to execute.
    pub blocks: Vec<Block>,
    /// Database containing all the data necessary to execute.
    pub execution_witness: ExecutionWitness,
}

#[cfg(not(feature = "eip-8025"))]
impl ProgramInput {
    /// Creates a new ProgramInput with the given blocks and execution witness.
    pub fn new(blocks: Vec<Block>, execution_witness: ExecutionWitness) -> Self {
        Self {
            blocks,
            execution_witness,
        }
    }
}

/// Input for the L1 stateless validation program (EIP-8025 build).
///
/// `Direct` carries in-memory blocks + witness (test path). `Wire` carries an
/// already-decoded EIP-8025 stateless input from spec wire bytes.
#[cfg(feature = "eip-8025")]
pub enum ProgramInput {
    Direct {
        blocks: Vec<Block>,
        execution_witness: ExecutionWitness,
    },
    Wire(DecodedEip8025),
}

#[cfg(feature = "eip-8025")]
impl Default for ProgramInput {
    fn default() -> Self {
        Self::Direct {
            blocks: Vec::new(),
            execution_witness: ExecutionWitness::default(),
        }
    }
}

#[cfg(feature = "eip-8025")]
impl ProgramInput {
    /// Creates a `Direct` ProgramInput from in-memory blocks and execution witness.
    pub fn new(blocks: Vec<Block>, execution_witness: ExecutionWitness) -> Self {
        Self::Direct {
            blocks,
            execution_witness,
        }
    }

    /// Creates a `Wire` ProgramInput from an already-decoded EIP-8025 payload.
    pub fn wire(decoded: DecodedEip8025) -> Self {
        Self::Wire(decoded)
    }
}

/// Wire-format version byte for the legacy EIP-8025 framing.
#[cfg(feature = "eip-8025")]
pub const EIP8025_VERSION_LEGACY: u8 = 0x00;

/// Wire-format version byte for the canonical EIP-8025 framing.
#[cfg(feature = "eip-8025")]
pub const EIP8025_VERSION_CANONICAL: u8 = 0x01;

/// Encode a `NewPayloadRequest` (SSZ) and `ExecutionWitness` (rkyv) into the
/// legacy EIP-8025 length-prefixed wire format:
///
///   `[version=0x00] [ssz_len: u32 LE] [ssz_bytes] [rkyv_bytes]`
///
/// Returns an error if rkyv serialization of the execution witness fails.
#[cfg(feature = "eip-8025")]
pub fn encode_eip8025(
    new_payload_request: &ethrex_common::types::eip8025_ssz::NewPayloadRequest,
    execution_witness: &ExecutionWitness,
) -> Result<Vec<u8>, ProgramInputEncodeError> {
    use libssz::SszEncode;

    let ssz_bytes = new_payload_request.to_ssz();
    let ssz_len = ssz_bytes.len() as u32;
    let rkyv_bytes = rkyv::to_bytes::<rkyv::rancor::Error>(execution_witness)
        .map_err(|e| ProgramInputEncodeError::Rkyv(e.to_string()))?;

    let mut out = Vec::with_capacity(1 + 4 + ssz_bytes.len() + rkyv_bytes.len());
    out.push(EIP8025_VERSION_LEGACY);
    out.extend_from_slice(&ssz_len.to_le_bytes());
    out.extend_from_slice(&ssz_bytes);
    out.extend_from_slice(&rkyv_bytes);
    Ok(out)
}

// ── canonical SSZ schema ───────────────────────────────────────────

#[cfg(feature = "eip-8025")]
const MAX_WITNESS_NODES: usize = 1 << 20;
#[cfg(feature = "eip-8025")]
const MAX_WITNESS_CODES: usize = 1 << 16;
#[cfg(feature = "eip-8025")]
const MAX_WITNESS_HEADERS: usize = 256;
#[cfg(feature = "eip-8025")]
const MAX_BYTES_PER_WITNESS_NODE: usize = 1 << 20;
#[cfg(feature = "eip-8025")]
const MAX_BYTES_PER_CODE: usize = 1 << 24;
#[cfg(feature = "eip-8025")]
const MAX_BYTES_PER_HEADER: usize = 1 << 10;
#[cfg(feature = "eip-8025")]
const MAX_PUBLIC_KEYS: usize = 1 << 20;
#[cfg(feature = "eip-8025")]
const BYTES_PER_PUBLIC_KEY: usize = 65;

/// SSZ shape of the per-tx public key list in `CanonicalStatelessInput`:
/// one fixed-size 65-byte uncompressed secp256k1 key per transaction.
#[cfg(feature = "eip-8025")]
pub type PublicKeysList =
    libssz_types::SszList<libssz_types::SszVector<u8, BYTES_PER_PUBLIC_KEY>, MAX_PUBLIC_KEYS>;
#[cfg(feature = "eip-8025")]
const MAX_OPTIONAL_FORK_ACTIVATION_VALUES: usize = 1;

/// Big-endian schema-id prefix on canonical `SszStatelessInput` wire bytes.
/// Per EELS `stateless_ssz.py`, `schema_id = (fork_index << 8) | revision`.
/// Amsterdam is fork `0x15`, revision `0x01` (SSZ `SszStatelessInput` payload).
#[cfg(feature = "eip-8025")]
pub const STATELESS_INPUT_SCHEMA_ID: u16 = 0x1501;
/// Byte length of [`STATELESS_INPUT_SCHEMA_ID`] on the wire.
#[cfg(feature = "eip-8025")]
pub const STATELESS_INPUT_SCHEMA_ID_SIZE: usize = 2;

/// Mirrors `SszForkActivation` from the Amsterdam stateless-validation spec.
#[cfg(feature = "eip-8025")]
#[derive(Debug, Clone, PartialEq, Eq, libssz_derive::SszEncode, libssz_derive::SszDecode)]
pub struct CanonicalForkActivation {
    pub block_number: libssz_types::SszList<u64, MAX_OPTIONAL_FORK_ACTIVATION_VALUES>,
    pub timestamp: libssz_types::SszList<u64, MAX_OPTIONAL_FORK_ACTIVATION_VALUES>,
}

/// Mirrors `SszForkConfig` from the Amsterdam stateless-validation spec.
/// As of glamsterdam-devnet-7 this carries only `activation`; the earlier
/// `fork` id and per-fork `blob_schedule` fields were dropped upstream.
#[cfg(feature = "eip-8025")]
#[derive(Debug, Clone, PartialEq, Eq, libssz_derive::SszEncode, libssz_derive::SszDecode)]
pub struct CanonicalForkConfig {
    pub activation: CanonicalForkActivation,
}

/// Mirrors `SszChainConfig` from the Amsterdam stateless-validation spec.
#[cfg(feature = "eip-8025")]
#[derive(Debug, Clone, PartialEq, Eq, libssz_derive::SszEncode, libssz_derive::SszDecode)]
pub struct CanonicalChainConfig {
    pub chain_id: u64,
    pub active_fork: CanonicalForkConfig,
}

/// Mirrors `SszExecutionWitness` from the Amsterdam stateless-validation spec.
#[cfg(feature = "eip-8025")]
#[derive(Debug, Clone, PartialEq, Eq, libssz_derive::SszEncode, libssz_derive::SszDecode)]
pub struct CanonicalExecutionWitness {
    pub state: libssz_types::SszList<
        libssz_types::SszList<u8, MAX_BYTES_PER_WITNESS_NODE>,
        MAX_WITNESS_NODES,
    >,
    pub codes:
        libssz_types::SszList<libssz_types::SszList<u8, MAX_BYTES_PER_CODE>, MAX_WITNESS_CODES>,
    pub headers:
        libssz_types::SszList<libssz_types::SszList<u8, MAX_BYTES_PER_HEADER>, MAX_WITNESS_HEADERS>,
}

/// Mirrors `SszStatelessInput` from the Amsterdam stateless-validation spec.
#[cfg(feature = "eip-8025")]
#[derive(Debug, Clone, PartialEq, Eq, libssz_derive::SszEncode, libssz_derive::SszDecode)]
pub struct CanonicalStatelessInput {
    pub new_payload_request: ethrex_common::types::eip8025_ssz::NewPayloadRequestAmsterdam,
    pub witness: CanonicalExecutionWitness,
    pub chain_config: CanonicalChainConfig,
    /// Per-transaction public keys (uncompressed secp256k1, 65 bytes each).
    /// Mirrors `SszList[ByteVector[PUBLIC_KEY_BYTES], MAX_PUBLIC_KEYS]` in the spec.
    pub public_keys: PublicKeysList,
}

/// Decoded EIP-8025 wire payload, dispatched by version byte.
#[cfg(feature = "eip-8025")]
pub enum DecodedEip8025 {
    /// Legacy framing (`version = 0x00`).
    Legacy {
        new_payload_request: ethrex_common::types::eip8025_ssz::NewPayloadRequest,
        execution_witness: ExecutionWitness,
    },
    /// Canonical-input framing (`version = 0x01`).
    Canonical {
        stateless_input: CanonicalStatelessInput,
        chain_config: ethrex_common::types::ChainConfig,
    },
}

#[cfg(feature = "eip-8025")]
impl core::fmt::Debug for DecodedEip8025 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            DecodedEip8025::Legacy { .. } => f.write_str("DecodedEip8025::Legacy"),
            DecodedEip8025::Canonical { .. } => f.write_str("DecodedEip8025::Canonical"),
        }
    }
}

/// Decode an EIP-8025 wire blob.
///
/// The first byte is a version discriminator:
/// - `0x00` → legacy framing
///   (`[ssz_len: u32 LE] [ssz_bytes] [rkyv ExecutionWitness]`).
/// - `0x01` → canonical-input framing
///   (`[ssz_len: u32 LE] [ssz_bytes] [cfg_len: u32 LE] [rkyv ChainConfig]`).
///
/// Anything else surfaces as [`ProgramInputDecodeError::UnknownVersion`].
#[cfg(feature = "eip-8025")]
pub fn decode_eip8025(bytes: &[u8]) -> Result<DecodedEip8025, ProgramInputDecodeError> {
    let (version, rest) = bytes
        .split_first()
        .ok_or(ProgramInputDecodeError::TooShort)?;
    match *version {
        EIP8025_VERSION_LEGACY => {
            let (new_payload_request, execution_witness) = decode_eip8025_legacy(rest)?;
            Ok(DecodedEip8025::Legacy {
                new_payload_request,
                execution_witness,
            })
        }
        EIP8025_VERSION_CANONICAL => {
            let (stateless_input, chain_config) = decode_eip8025_canonical(rest)?;
            Ok(DecodedEip8025::Canonical {
                stateless_input,
                chain_config,
            })
        }
        v => Err(ProgramInputDecodeError::UnknownVersion(v)),
    }
}

/// Decode a spec-format canonical stateless input blob:
/// `[BE u16 STATELESS_INPUT_SCHEMA_ID][SSZ-encoded CanonicalStatelessInput]`.
/// Caller supplies `chain_config` out-of-band (unlike [`decode_eip8025`]).
#[cfg(feature = "eip-8025")]
pub fn decode_canonical_stateless_input_bytes(
    bytes: &[u8],
) -> Result<CanonicalStatelessInput, ProgramInputDecodeError> {
    use libssz::SszDecode;

    if bytes.len() < STATELESS_INPUT_SCHEMA_ID_SIZE {
        return Err(ProgramInputDecodeError::TooShort);
    }
    let (schema_bytes, ssz_bytes) = bytes.split_at(STATELESS_INPUT_SCHEMA_ID_SIZE);
    let schema_id = u16::from_be_bytes([schema_bytes[0], schema_bytes[1]]);
    if schema_id != STATELESS_INPUT_SCHEMA_ID {
        return Err(ProgramInputDecodeError::UnknownSchemaId(schema_id));
    }
    CanonicalStatelessInput::from_ssz_bytes(ssz_bytes).map_err(ProgramInputDecodeError::Ssz)
}

#[cfg(feature = "eip-8025")]
fn decode_eip8025_legacy(
    bytes: &[u8],
) -> Result<
    (
        ethrex_common::types::eip8025_ssz::NewPayloadRequest,
        ExecutionWitness,
    ),
    ProgramInputDecodeError,
> {
    use libssz::SszDecode;

    if bytes.len() < 4 {
        return Err(ProgramInputDecodeError::TooShort);
    }
    // Safety: we already checked bytes.len() >= 4 above, so this slice is exactly 4 bytes.
    let ssz_len = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    if bytes.len() < 4 + ssz_len {
        return Err(ProgramInputDecodeError::TooShort);
    }
    let ssz_bytes = &bytes[4..4 + ssz_len];
    let rkyv_bytes = &bytes[4 + ssz_len..];

    let new_payload_request =
        ethrex_common::types::eip8025_ssz::NewPayloadRequest::from_ssz_bytes(ssz_bytes)
            .map_err(ProgramInputDecodeError::Ssz)?;
    let execution_witness = rkyv::from_bytes::<ExecutionWitness, rkyv::rancor::Error>(rkyv_bytes)
        .map_err(|e| ProgramInputDecodeError::Rkyv(e.to_string()))?;

    Ok((new_payload_request, execution_witness))
}

#[cfg(feature = "eip-8025")]
fn decode_eip8025_canonical(
    bytes: &[u8],
) -> Result<(CanonicalStatelessInput, ethrex_common::types::ChainConfig), ProgramInputDecodeError> {
    use libssz::SszDecode;

    if bytes.len() < 4 {
        return Err(ProgramInputDecodeError::TooShort);
    }
    let ssz_len = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    let cfg_len_off = 4usize
        .checked_add(ssz_len)
        .ok_or(ProgramInputDecodeError::TooShort)?;
    if bytes.len() < cfg_len_off + 4 {
        return Err(ProgramInputDecodeError::TooShort);
    }
    let ssz_bytes = &bytes[4..cfg_len_off];

    let cfg_len = u32::from_le_bytes([
        bytes[cfg_len_off],
        bytes[cfg_len_off + 1],
        bytes[cfg_len_off + 2],
        bytes[cfg_len_off + 3],
    ]) as usize;
    let cfg_off = cfg_len_off + 4;
    let cfg_end = cfg_off
        .checked_add(cfg_len)
        .ok_or(ProgramInputDecodeError::TooShort)?;
    if bytes.len() < cfg_end {
        return Err(ProgramInputDecodeError::TooShort);
    }
    let cfg_bytes = &bytes[cfg_off..cfg_end];

    let stateless_input =
        CanonicalStatelessInput::from_ssz_bytes(ssz_bytes).map_err(ProgramInputDecodeError::Ssz)?;
    let chain_config =
        rkyv::from_bytes::<ethrex_common::types::ChainConfig, rkyv::rancor::Error>(cfg_bytes)
            .map_err(|e| ProgramInputDecodeError::Rkyv(e.to_string()))?;

    Ok((stateless_input, chain_config))
}

#[cfg(feature = "eip-8025")]
#[derive(Debug)]
pub enum ProgramInputEncodeError {
    Rkyv(String),
}

#[cfg(feature = "eip-8025")]
impl core::fmt::Display for ProgramInputEncodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Rkyv(e) => write!(f, "rkyv encode error: {e}"),
        }
    }
}

#[cfg(feature = "eip-8025")]
#[derive(Debug)]
pub enum ProgramInputDecodeError {
    TooShort,
    Ssz(libssz::DecodeError),
    Rkyv(String),
    UnknownVersion(u8),
    UnknownSchemaId(u16),
}

#[cfg(feature = "eip-8025")]
impl core::fmt::Display for ProgramInputDecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooShort => write!(f, "input too short"),
            Self::Ssz(e) => write!(f, "SSZ decode error: {e}"),
            Self::Rkyv(e) => write!(f, "rkyv decode error: {e}"),
            Self::UnknownVersion(v) => write!(f, "unknown EIP-8025 wire version: {v:#04x}"),
            Self::UnknownSchemaId(v) => {
                write!(f, "unknown stateless input schema id: {v:#06x}")
            }
        }
    }
}
