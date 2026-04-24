use ethrex_common::types::Block;
use ethrex_common::types::block_execution_witness::ExecutionWitness;

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

/// Encode a `NewPayloadRequest` (SSZ) and `ExecutionWitness` (SSZ) into the
/// EIP-8025 length-prefixed wire format:
///
///   `[ssz_len: u32 LE] [ssz_bytes] [witness_ssz_bytes]`
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

    let mut out = Vec::with_capacity(4 + ssz_bytes.len() + witness_ssz_bytes.len());
    out.extend_from_slice(&ssz_len.to_le_bytes());
    out.extend_from_slice(&ssz_bytes);
    out.extend_from_slice(&witness_ssz_bytes);
    Ok(out)
}

/// Decode the EIP-8025 length-prefixed wire format into a `NewPayloadRequest`
/// and `ExecutionWitness`.
///
/// The caller is responsible for converting the `NewPayloadRequest` into blocks
/// and constructing a `ProgramInput`.
#[cfg(feature = "eip-8025")]
pub fn decode_eip8025(
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
}

#[cfg(feature = "eip-8025")]
impl core::fmt::Display for ProgramInputDecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooShort => write!(f, "input too short"),
            Self::Ssz(e) => write!(f, "SSZ decode error: {e}"),
            Self::WitnessSsz(e) => write!(f, "execution witness SSZ decode error: {e}"),
        }
    }
}
