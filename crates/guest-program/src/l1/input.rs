use ethrex_common::types::Block;
use ethrex_common::types::block_execution_witness::ExecutionWitness;

/// Input for the L1 stateless validation program.
#[derive(
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

impl ProgramInput {
    /// Creates a new ProgramInput with the given blocks and execution witness.
    pub fn new(blocks: Vec<Block>, execution_witness: ExecutionWitness) -> Self {
        Self {
            blocks,
            execution_witness,
        }
    }
}

/// Encode a `NewPayloadRequest` (SSZ) and `ExecutionWitness` (rkyv) into the
/// EIP-8025 length-prefixed wire format:
///
///   `[ssz_len: u32 LE] [ssz_bytes] [rkyv_bytes]`
#[cfg(feature = "eip-8025")]
pub fn encode_eip8025(
    new_payload_request: &ethrex_common::types::eip8025_ssz::NewPayloadRequest,
    execution_witness: &ExecutionWitness,
) -> Vec<u8> {
    use ssz::SszEncode;

    let ssz_bytes = new_payload_request.to_ssz();
    let ssz_len = ssz_bytes.len() as u32;
    let rkyv_bytes =
        rkyv::to_bytes::<rkyv::rancor::Error>(execution_witness).expect("rkyv encode");

    let mut out = Vec::with_capacity(4 + ssz_bytes.len() + rkyv_bytes.len());
    out.extend_from_slice(&ssz_len.to_le_bytes());
    out.extend_from_slice(&ssz_bytes);
    out.extend_from_slice(&rkyv_bytes);
    out
}

/// Decode the EIP-8025 length-prefixed wire format into a `NewPayloadRequest`
/// and `ExecutionWitness`.
///
/// The caller is responsible for converting the `NewPayloadRequest` into blocks
/// and constructing a `ProgramInput`.
#[cfg(feature = "eip-8025")]
pub fn decode_eip8025(
    bytes: &[u8],
) -> Result<
    (
        ethrex_common::types::eip8025_ssz::NewPayloadRequest,
        ExecutionWitness,
    ),
    ProgramInputDecodeError,
> {
    use ssz::SszDecode;

    if bytes.len() < 4 {
        return Err(ProgramInputDecodeError::TooShort);
    }
    let ssz_len = u32::from_le_bytes(bytes[..4].try_into().expect("4 bytes")) as usize;
    if bytes.len() < 4 + ssz_len {
        return Err(ProgramInputDecodeError::TooShort);
    }
    let ssz_bytes = &bytes[4..4 + ssz_len];
    let rkyv_bytes = &bytes[4 + ssz_len..];

    let new_payload_request =
        ethrex_common::types::eip8025_ssz::NewPayloadRequest::from_ssz_bytes(ssz_bytes)
            .map_err(ProgramInputDecodeError::Ssz)?;
    let execution_witness =
        rkyv::from_bytes::<ExecutionWitness, rkyv::rancor::Error>(rkyv_bytes)
            .map_err(|e| ProgramInputDecodeError::Rkyv(e.to_string()))?;

    Ok((new_payload_request, execution_witness))
}

#[cfg(feature = "eip-8025")]
#[derive(Debug)]
pub enum ProgramInputDecodeError {
    TooShort,
    Ssz(ssz::DecodeError),
    Rkyv(String),
}

#[cfg(feature = "eip-8025")]
impl core::fmt::Display for ProgramInputDecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooShort => write!(f, "input too short"),
            Self::Ssz(e) => write!(f, "SSZ decode error: {e}"),
            Self::Rkyv(e) => write!(f, "rkyv decode error: {e}"),
        }
    }
}
