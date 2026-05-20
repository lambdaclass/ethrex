//! SSZ wire types for engine REST blob endpoints.

use libssz::{DecodeError, SszDecode, SszEncode};
use libssz_derive::{HashTreeRoot, SszDecode, SszEncode};
use libssz_types::SszList;

/// Spec / KZG-derived blob constants.
pub const BYTES_PER_BLOB: usize = 131_072;
pub const BYTES_PER_PROOF: usize = 48;
pub const NUM_CELLS_PER_BLOB: usize = 128;
pub const BYTES_PER_CELL: usize = 2048;
pub const MAX_BLOBS_PER_REQUEST: usize = 128;
pub const MAX_PROOFS_PER_BLOB: usize = NUM_CELLS_PER_BLOB;

/// `/blobs/v1`, `/blobs/v2`, `/blobs/v3` request: just versioned_hashes.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BlobsRequest {
    pub versioned_hashes: SszList<[u8; 32], MAX_BLOBS_PER_REQUEST>,
}

/// `/blobs/v4` request: versioned_hashes + cell_indices to filter.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BlobsRequestV4 {
    pub versioned_hashes: SszList<[u8; 32], MAX_BLOBS_PER_REQUEST>,
    pub cell_indices: SszList<u8, NUM_CELLS_PER_BLOB>,
}

/// V1 response item: single blob + single KZG proof.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BlobAndProofV1 {
    pub blob: SszList<u8, BYTES_PER_BLOB>,
    pub proof: [u8; BYTES_PER_PROOF],
}

/// V2 / V3 response item: blob + N cell proofs.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BlobAndProofV2 {
    pub blob: SszList<u8, BYTES_PER_BLOB>,
    pub proofs: SszList<[u8; BYTES_PER_PROOF], MAX_PROOFS_PER_BLOB>,
}

/// V4 response item: blob + sparse cells with their indices and proofs.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BlobAndCellsV4 {
    pub blob: SszList<u8, BYTES_PER_BLOB>,
    pub cell_indices: SszList<u8, NUM_CELLS_PER_BLOB>,
    pub cells: SszList<SszList<u8, BYTES_PER_CELL>, NUM_CELLS_PER_BLOB>,
    pub proofs: SszList<[u8; BYTES_PER_PROOF], NUM_CELLS_PER_BLOB>,
}

// ── `/blobs/v1` response wrappers ─────────────────────────────────────────────
//
// The response is `Vec<Option<BlobAndProofV1>>`. libssz has no blanket
// `Option<NestedStruct>` impl, so we use the SSZ union encoding manually:
//
//   selector = 0x00  → None (no further bytes)
//   selector = 0x01  → Some, followed by the inner item's SSZ encoding
//
// `OptBlobAndProofV1` wraps a single slot; `BlobsResponseV1` wraps the Vec.

/// SSZ-union wrapper for `Option<BlobAndProofV1>`. Selector 0 = None, 1 = Some.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptBlobAndProofV1(pub Option<BlobAndProofV1>);

impl OptBlobAndProofV1 {
    pub fn none() -> Self {
        OptBlobAndProofV1(None)
    }
    pub fn some(inner: BlobAndProofV1) -> Self {
        OptBlobAndProofV1(Some(inner))
    }
}

impl SszEncode for OptBlobAndProofV1 {
    fn is_fixed_size() -> bool {
        false
    }
    fn fixed_size() -> usize {
        0
    }
    fn encoded_len(&self) -> usize {
        match &self.0 {
            None => 1,
            Some(b) => 1 + b.encoded_len(),
        }
    }
    fn ssz_append(&self, buf: &mut Vec<u8>) {
        match &self.0 {
            None => buf.push(0),
            Some(b) => {
                buf.push(1);
                b.ssz_append(buf);
            }
        }
    }
}

impl SszDecode for OptBlobAndProofV1 {
    fn is_fixed_size() -> bool {
        false
    }
    fn fixed_size() -> usize {
        0
    }
    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        if bytes.is_empty() {
            return Err(DecodeError::EmptyInput);
        }
        match bytes[0] {
            0 => {
                if bytes.len() != 1 {
                    return Err(DecodeError::AdditionalBytes {
                        expected: 1,
                        got: bytes.len(),
                    });
                }
                Ok(OptBlobAndProofV1(None))
            }
            1 => {
                let inner = BlobAndProofV1::from_ssz_bytes(&bytes[1..])?;
                Ok(OptBlobAndProofV1(Some(inner)))
            }
            s => Err(DecodeError::InvalidUnionSelector(s)),
        }
    }
}

/// `/blobs/v1` response: a list of `Option<BlobAndProofV1>` slots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobsResponseV1 {
    pub items: Vec<OptBlobAndProofV1>,
}

impl SszEncode for BlobsResponseV1 {
    fn is_fixed_size() -> bool {
        false
    }
    fn fixed_size() -> usize {
        0
    }
    fn encoded_len(&self) -> usize {
        self.items.encoded_len()
    }
    fn ssz_append(&self, buf: &mut Vec<u8>) {
        self.items.ssz_append(buf);
    }
}

impl SszDecode for BlobsResponseV1 {
    fn is_fixed_size() -> bool {
        false
    }
    fn fixed_size() -> usize {
        0
    }
    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let items = Vec::<OptBlobAndProofV1>::from_ssz_bytes(bytes)?;
        Ok(BlobsResponseV1 { items })
    }
}

// ── `/blobs/v2` + `/blobs/v3` response wrappers ──────────────────────────────
//
// Same SSZ union encoding as V1:
//   selector = 0x00  → None
//   selector = 0x01  → Some, followed by the inner item's SSZ encoding

/// SSZ-union wrapper for `Option<BlobAndProofV2>`. Selector 0 = None, 1 = Some.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptBlobAndProofV2(pub Option<BlobAndProofV2>);

impl OptBlobAndProofV2 {
    pub fn none() -> Self {
        OptBlobAndProofV2(None)
    }
    pub fn some(inner: BlobAndProofV2) -> Self {
        OptBlobAndProofV2(Some(inner))
    }
}

impl SszEncode for OptBlobAndProofV2 {
    fn is_fixed_size() -> bool {
        false
    }
    fn fixed_size() -> usize {
        0
    }
    fn encoded_len(&self) -> usize {
        match &self.0 {
            None => 1,
            Some(b) => 1 + b.encoded_len(),
        }
    }
    fn ssz_append(&self, buf: &mut Vec<u8>) {
        match &self.0 {
            None => buf.push(0),
            Some(b) => {
                buf.push(1);
                b.ssz_append(buf);
            }
        }
    }
}

impl SszDecode for OptBlobAndProofV2 {
    fn is_fixed_size() -> bool {
        false
    }
    fn fixed_size() -> usize {
        0
    }
    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        if bytes.is_empty() {
            return Err(DecodeError::EmptyInput);
        }
        match bytes[0] {
            0 => {
                if bytes.len() != 1 {
                    return Err(DecodeError::AdditionalBytes {
                        expected: 1,
                        got: bytes.len(),
                    });
                }
                Ok(OptBlobAndProofV2(None))
            }
            1 => {
                let inner = BlobAndProofV2::from_ssz_bytes(&bytes[1..])?;
                Ok(OptBlobAndProofV2(Some(inner)))
            }
            s => Err(DecodeError::InvalidUnionSelector(s)),
        }
    }
}

/// `/blobs/v2` + `/blobs/v3` response: a list of `Option<BlobAndProofV2>` slots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobsResponseV2 {
    pub items: Vec<OptBlobAndProofV2>,
}

impl SszEncode for BlobsResponseV2 {
    fn is_fixed_size() -> bool {
        false
    }
    fn fixed_size() -> usize {
        0
    }
    fn encoded_len(&self) -> usize {
        self.items.encoded_len()
    }
    fn ssz_append(&self, buf: &mut Vec<u8>) {
        self.items.ssz_append(buf);
    }
}

impl SszDecode for BlobsResponseV2 {
    fn is_fixed_size() -> bool {
        false
    }
    fn fixed_size() -> usize {
        0
    }
    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let items = Vec::<OptBlobAndProofV2>::from_ssz_bytes(bytes)?;
        Ok(BlobsResponseV2 { items })
    }
}

// ── `/blobs/v4` response wrappers ─────────────────────────────────────────────
//
// Same SSZ union encoding as V1/V2:
//   selector = 0x00  → None
//   selector = 0x01  → Some, followed by the inner item's SSZ encoding

/// SSZ-union wrapper for `Option<BlobAndCellsV4>`. Selector 0 = None, 1 = Some.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptBlobAndCellsV4(pub Option<BlobAndCellsV4>);

impl OptBlobAndCellsV4 {
    pub fn none() -> Self {
        OptBlobAndCellsV4(None)
    }
    pub fn some(inner: BlobAndCellsV4) -> Self {
        OptBlobAndCellsV4(Some(inner))
    }
}

impl SszEncode for OptBlobAndCellsV4 {
    fn is_fixed_size() -> bool {
        false
    }
    fn fixed_size() -> usize {
        0
    }
    fn encoded_len(&self) -> usize {
        match &self.0 {
            None => 1,
            Some(b) => 1 + b.encoded_len(),
        }
    }
    fn ssz_append(&self, buf: &mut Vec<u8>) {
        match &self.0 {
            None => buf.push(0),
            Some(b) => {
                buf.push(1);
                b.ssz_append(buf);
            }
        }
    }
}

impl SszDecode for OptBlobAndCellsV4 {
    fn is_fixed_size() -> bool {
        false
    }
    fn fixed_size() -> usize {
        0
    }
    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        if bytes.is_empty() {
            return Err(DecodeError::EmptyInput);
        }
        match bytes[0] {
            0 => {
                if bytes.len() != 1 {
                    return Err(DecodeError::AdditionalBytes {
                        expected: 1,
                        got: bytes.len(),
                    });
                }
                Ok(OptBlobAndCellsV4(None))
            }
            1 => {
                let inner = BlobAndCellsV4::from_ssz_bytes(&bytes[1..])?;
                Ok(OptBlobAndCellsV4(Some(inner)))
            }
            s => Err(DecodeError::InvalidUnionSelector(s)),
        }
    }
}

/// `/blobs/v4` response: a list of `Option<BlobAndCellsV4>` slots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobsResponseV4 {
    pub items: Vec<OptBlobAndCellsV4>,
}

impl SszEncode for BlobsResponseV4 {
    fn is_fixed_size() -> bool {
        false
    }
    fn fixed_size() -> usize {
        0
    }
    fn encoded_len(&self) -> usize {
        self.items.encoded_len()
    }
    fn ssz_append(&self, buf: &mut Vec<u8>) {
        self.items.ssz_append(buf);
    }
}

impl SszDecode for BlobsResponseV4 {
    fn is_fixed_size() -> bool {
        false
    }
    fn fixed_size() -> usize {
        0
    }
    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let items = Vec::<OptBlobAndCellsV4>::from_ssz_bytes(bytes)?;
        Ok(BlobsResponseV4 { items })
    }
}
