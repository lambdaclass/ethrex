use ethrex_common::types::block_execution_witness::ExecutionWitness;

#[cfg(not(feature = "eip-8025"))]
use ethrex_common::types::Block;

/// Input for the L1 stateless validation program.
#[cfg(not(feature = "eip-8025"))]
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

/// Input for the L1 stateless validation program (EIP-8025).
///
/// Under EIP-8025, the input is a single `NewPayloadRequest` (SSZ container)
/// plus the execution witness needed for stateless validation.
///
/// Serialization uses a length-prefixed format:
///   `[ssz_len: u32 LE] [ssz_bytes] [rkyv_bytes]`
/// where `ssz_bytes` is the SSZ encoding of `NewPayloadRequest` and
/// `rkyv_bytes` is the rkyv encoding of `ExecutionWitness`.
#[cfg(feature = "eip-8025")]
pub struct ProgramInput {
    /// The new-payload request from the consensus layer.
    pub new_payload_request: ethrex_common::types::eip8025_ssz::NewPayloadRequest,
    /// Database containing all the data necessary to execute.
    pub execution_witness: ExecutionWitness,
}

#[cfg(feature = "eip-8025")]
impl ProgramInput {
    /// Creates a new ProgramInput.
    pub fn new(
        new_payload_request: ethrex_common::types::eip8025_ssz::NewPayloadRequest,
        execution_witness: ExecutionWitness,
    ) -> Self {
        Self {
            new_payload_request,
            execution_witness,
        }
    }

    /// Encode to bytes: `[ssz_len: u32 LE][ssz_bytes][rkyv_bytes]`.
    pub fn encode(&self) -> Vec<u8> {
        use ssz::SszEncode;

        let ssz_bytes = self.new_payload_request.to_ssz();
        let ssz_len = ssz_bytes.len() as u32;
        let rkyv_bytes =
            rkyv::to_bytes::<rkyv::rancor::Error>(&self.execution_witness).expect("rkyv encode");

        let mut out = Vec::with_capacity(4 + ssz_bytes.len() + rkyv_bytes.len());
        out.extend_from_slice(&ssz_len.to_le_bytes());
        out.extend_from_slice(&ssz_bytes);
        out.extend_from_slice(&rkyv_bytes);
        out
    }

    /// Decode from bytes produced by [`encode`](Self::encode).
    pub fn decode(bytes: &[u8]) -> Result<Self, ProgramInputDecodeError> {
        use ssz::SszDecode;

        if bytes.len() < 4 {
            return Err(ProgramInputDecodeError::TooShort);
        }
        let ssz_len =
            u32::from_le_bytes(bytes[..4].try_into().expect("4 bytes")) as usize;
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

        Ok(Self {
            new_payload_request,
            execution_witness,
        })
    }
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
