use ethrex_common::types::Block;
use ethrex_common::types::block_execution_witness::ExecutionWitness;

#[cfg(feature = "eip-8025")]
use ethrex_common::types::block_execution_witness::{
    MAX_BYTES_PER_CODE, MAX_BYTES_PER_HEADER, MAX_BYTES_PER_PUBLIC_KEY, MAX_BYTES_PER_WITNESS_NODE,
    MAX_PUBLIC_KEYS, MAX_WITNESS_CODES, MAX_WITNESS_HEADERS, MAX_WITNESS_NODES,
};

/// Input for the L1 stateless validation program.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ProgramInput {
    /// Blocks to execute.
    pub blocks: Vec<Block>,
    /// Database containing all the data necessary to execute.
    pub execution_witness: ExecutionWitness,
}

impl ProgramInput {
    /// Creates a new ProgramInput with the given blocks and execution witness.
    pub fn new(blocks: Vec<Block>, execution_witness: ExecutionWitness) -> Self {
        Self {
            blocks,
            execution_witness,
        }
    }
}

/// Wire-format version byte for the legacy EIP-8025 framing.
#[cfg(feature = "eip-8025")]
pub const EIP8025_VERSION_LEGACY: u8 = 0x00;

/// Wire-format version byte for the canonical EIP-8025 framing.
#[cfg(feature = "eip-8025")]
pub const EIP8025_VERSION_CANONICAL: u8 = 0x01;

/// Encode a `NewPayloadRequest` (SSZ) and `ExecutionWitness` (SSZ) into the
/// legacy EIP-8025 length-prefixed wire format:
///
///   `[version=0x00] [ssz_len: u32 LE] [ssz_bytes] [witness_ssz_bytes]`
///
/// Returns an error if SSZ serialization of the execution witness fails.
#[cfg(feature = "eip-8025")]
pub fn encode_eip8025(
    new_payload_request: &ethrex_common::types::eip8025_ssz::NewPayloadRequest,
    execution_witness: &ExecutionWitness,
) -> Result<Vec<u8>, ProgramInputEncodeError> {
    use libssz::SszEncode;

    let ssz_bytes = new_payload_request.to_ssz();
    let ssz_len = ssz_bytes.len() as u32;
    let witness_ssz_bytes = execution_witness
        .to_ssz_bytes()
        .map_err(ProgramInputEncodeError::WitnessSsz)?;

    let mut out = Vec::with_capacity(1 + 4 + ssz_bytes.len() + witness_ssz_bytes.len());
    out.push(EIP8025_VERSION_LEGACY);
    out.extend_from_slice(&ssz_len.to_le_bytes());
    out.extend_from_slice(&ssz_bytes);
    out.extend_from_slice(&witness_ssz_bytes);
    Ok(out)
}

/// Mirrors `SszChainConfig` from the Amsterdam stateless-validation spec.
#[cfg(feature = "eip-8025")]
#[derive(Debug, Clone, PartialEq, Eq, libssz_derive::SszEncode, libssz_derive::SszDecode)]
pub struct CanonicalChainConfig {
    pub chain_id: u64,
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
    // Currently the specs do not include proper values for this field,
    // but it is planned to be supported in the next release.
    pub public_keys:
        libssz_types::SszList<libssz_types::SszList<u8, MAX_BYTES_PER_PUBLIC_KEY>, MAX_PUBLIC_KEYS>,
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
///   (`[ssz_len: u32 LE] [ssz_bytes] [ssz_witness_bytes]`).
/// - `0x01` → canonical-input framing
///   (`[ssz_len: u32 LE] [ssz_bytes] [cfg_len: u32 LE] [ssz ChainConfig]`).
///
/// Anything else surfaces as [`ProgramInputDecodeError::UnknownVersion`].
#[cfg(feature = "eip-8025")]
pub fn decode_eip8025(
    bytes: &[u8],
    crypto: &dyn ethrex_crypto::Crypto,
) -> Result<DecodedEip8025, ProgramInputDecodeError> {
    let (version, rest) = bytes
        .split_first()
        .ok_or(ProgramInputDecodeError::TooShort)?;
    match *version {
        EIP8025_VERSION_LEGACY => {
            let (new_payload_request, execution_witness) = decode_eip8025_legacy(rest, crypto)?;
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

#[cfg(feature = "eip-8025")]
fn decode_eip8025_legacy(
    bytes: &[u8],
    crypto: &dyn ethrex_crypto::Crypto,
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
    let witness_ssz_bytes = &bytes[4 + ssz_len..];

    let new_payload_request =
        ethrex_common::types::eip8025_ssz::NewPayloadRequest::from_ssz_bytes(ssz_bytes)
            .map_err(ProgramInputDecodeError::Ssz)?;
    let execution_witness = ExecutionWitness::from_ssz_bytes(witness_ssz_bytes, crypto)
        .map_err(ProgramInputDecodeError::WitnessSsz)?;

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
    let chain_config = ethrex_common::types::ChainConfig::decode_bytes(cfg_bytes)
        .map_err(ProgramInputDecodeError::ChainConfigSsz)?;

    Ok((stateless_input, chain_config))
}

#[cfg(feature = "eip-8025")]
#[derive(Debug)]
pub enum ProgramInputEncodeError {
    WitnessSsz(ethrex_common::types::block_execution_witness::ExecutionWitnessSszError),
}

#[cfg(feature = "eip-8025")]
impl core::fmt::Display for ProgramInputEncodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::WitnessSsz(e) => write!(f, "execution witness SSZ encode error: {e}"),
        }
    }
}

#[cfg(feature = "eip-8025")]
#[derive(Debug)]
pub enum ProgramInputDecodeError {
    TooShort,
    Ssz(libssz::DecodeError),
    WitnessSsz(ethrex_common::types::block_execution_witness::ExecutionWitnessSszError),
    ChainConfigSsz(String),
    UnknownVersion(u8),
}

#[cfg(feature = "eip-8025")]
impl core::fmt::Display for ProgramInputDecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooShort => write!(f, "input too short"),
            Self::Ssz(e) => write!(f, "SSZ decode error: {e}"),
            Self::WitnessSsz(e) => write!(f, "execution witness SSZ decode error: {e}"),
            Self::ChainConfigSsz(e) => write!(f, "chain config SSZ decode error: {e}"),
            Self::UnknownVersion(v) => write!(f, "unknown EIP-8025 wire version: {v:#04x}"),
        }
    }
}
