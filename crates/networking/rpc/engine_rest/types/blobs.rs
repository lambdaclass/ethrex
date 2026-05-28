//! SSZ wire types for the engine REST blob endpoints.
//!
//! Shapes follow execution-apis PR #793 (`src/engine/refactor-ssz.md`):
//! each response is a `List[BlobV*Entry]`, where an entry carries an
//! `available` boolean plus `contents`. When `available` is false the
//! `contents` are zero-valued and CLs MUST ignore them. `/blobs/v2` is
//! all-or-nothing (the handler returns 204 when any blob is missing rather
//! than emitting unavailable entries); `/blobs/v3` surfaces missing blobs
//! per entry.

use libssz::{DecodeError, SszDecode, SszEncode};
use libssz_derive::{HashTreeRoot, SszDecode, SszEncode};
use libssz_types::{SszBitvector, SszList, SszVector};

/// Spec / KZG-derived blob constants.
pub const BYTES_PER_BLOB: usize = 131_072;
pub const BYTES_PER_PROOF: usize = 48;
pub const CELLS_PER_EXT_BLOB: usize = 128;
/// `BYTES_PER_CELL` per #793 (`BYTES_PER_BLOB / CELLS_PER_EXT_BLOB`). NOTE:
/// EIP-7594 itself uses 2048-byte cells (over the *extended* blob); the #793
/// draft derives 1024 and flags the value as an open question. We track the
/// spec document here since ethrex never emits cell data today (see `/blobs/v4`).
pub const BYTES_PER_CELL: usize = 1_024;
/// Spec cap on versioned hashes per blobs request (`MAX_BLOBS_REQUEST`).
pub const MAX_BLOBS_REQUEST: usize = 128;

// ── Requests ──────────────────────────────────────────────────────────────────

/// `/blobs/v1`, `/blobs/v2`, `/blobs/v3` request: versioned hashes only.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BlobsRequest {
    pub versioned_hashes: SszList<[u8; 32], MAX_BLOBS_REQUEST>,
}

/// `/blobs/v4` request: versioned hashes + a `CELLS_PER_EXT_BLOB`-bit bitarray
/// selecting which cell indices to return.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BlobsRequestV4 {
    pub versioned_hashes: SszList<[u8; 32], MAX_BLOBS_REQUEST>,
    pub indices_bitarray: SszBitvector<CELLS_PER_EXT_BLOB>,
}

// ── BlobAndProof leaf types ───────────────────────────────────────────────────

/// `/blobs/v1` contents: whole blob + single KZG proof (Cancun).
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BlobAndProofV1 {
    pub blob: SszVector<u8, BYTES_PER_BLOB>,
    pub proof: [u8; BYTES_PER_PROOF],
}

/// `/blobs/v2` and `/blobs/v3` contents: whole blob + cell proofs (Osaka).
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BlobAndProofV2 {
    pub blob: SszVector<u8, BYTES_PER_BLOB>,
    pub proofs: SszList<[u8; BYTES_PER_PROOF], CELLS_PER_EXT_BLOB>,
}

impl BlobAndProofV1 {
    /// A zero-valued instance used as the `contents` of an unavailable entry.
    pub fn zeroed() -> Self {
        BlobAndProofV1 {
            blob: vec![0u8; BYTES_PER_BLOB]
                .try_into()
                .expect("BYTES_PER_BLOB zero blob fits SszVector"),
            proof: [0u8; BYTES_PER_PROOF],
        }
    }
}

impl BlobAndProofV2 {
    /// A zero-valued instance used as the `contents` of an unavailable entry.
    pub fn zeroed() -> Self {
        BlobAndProofV2 {
            blob: vec![0u8; BYTES_PER_BLOB]
                .try_into()
                .expect("BYTES_PER_BLOB zero blob fits SszVector"),
            proofs: Vec::new().try_into().expect("empty proofs fits SszList"),
        }
    }
}

// ── Entry types ───────────────────────────────────────────────────────────────

/// `/blobs/v1` response entry. When `available` is false, `contents` is zeroed
/// and CLs MUST ignore it.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BlobV1Entry {
    pub available: bool,
    pub contents: BlobAndProofV1,
}

/// `/blobs/v2` and `/blobs/v3` response entry.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BlobV2Entry {
    pub available: bool,
    pub contents: BlobAndProofV2,
}

impl BlobV1Entry {
    pub fn available(contents: BlobAndProofV1) -> Self {
        BlobV1Entry {
            available: true,
            contents,
        }
    }
    pub fn unavailable() -> Self {
        BlobV1Entry {
            available: false,
            contents: BlobAndProofV1::zeroed(),
        }
    }
}

impl BlobV2Entry {
    pub fn available(contents: BlobAndProofV2) -> Self {
        BlobV2Entry {
            available: true,
            contents,
        }
    }
    pub fn unavailable() -> Self {
        BlobV2Entry {
            available: false,
            contents: BlobAndProofV2::zeroed(),
        }
    }
}

// ── Response containers ───────────────────────────────────────────────────────

/// `/blobs/v1` response.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BlobsV1Response {
    pub entries: SszList<BlobV1Entry, MAX_BLOBS_REQUEST>,
}

/// `/blobs/v2` response (all-or-nothing; every entry is `available`).
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BlobsV2Response {
    pub entries: SszList<BlobV2Entry, MAX_BLOBS_REQUEST>,
}

/// `/blobs/v3` response (partial; missing blobs surface as `available == false`).
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BlobsV3Response {
    pub entries: SszList<BlobV2Entry, MAX_BLOBS_REQUEST>,
}

// ── `/blobs/v4` types ─────────────────────────────────────────────────────────
//
// V4 carries per-cell nullable data. SSZ `Optional[T]` is a union (selector
// 0 = None, 1 = Some(T)); libssz has no blanket `Option<T>` impl, so the two
// element wrappers below implement SSZ encode/decode by hand (mirroring the
// union wrappers elsewhere in this module). HashTreeRoot is intentionally not
// implemented for the V4 types — they are encode/decode only, like the other
// hand-written response containers.

/// SSZ `Optional[ByteVector[BYTES_PER_CELL]]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionalCell(pub Option<SszVector<u8, BYTES_PER_CELL>>);

impl SszEncode for OptionalCell {
    fn is_fixed_size() -> bool {
        false
    }
    fn fixed_size() -> usize {
        0
    }
    fn encoded_len(&self) -> usize {
        match &self.0 {
            None => 1,
            Some(v) => 1 + v.encoded_len(),
        }
    }
    fn ssz_append(&self, buf: &mut Vec<u8>) {
        match &self.0 {
            None => buf.push(0),
            Some(v) => {
                buf.push(1);
                v.ssz_append(buf);
            }
        }
    }
}

impl SszDecode for OptionalCell {
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
                Ok(OptionalCell(None))
            }
            1 => {
                let v = SszVector::<u8, BYTES_PER_CELL>::from_ssz_bytes(&bytes[1..])?;
                Ok(OptionalCell(Some(v)))
            }
            s => Err(DecodeError::InvalidUnionSelector(s)),
        }
    }
}

/// SSZ `Optional[Bytes48]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionalProof(pub Option<[u8; BYTES_PER_PROOF]>);

impl SszEncode for OptionalProof {
    fn is_fixed_size() -> bool {
        false
    }
    fn fixed_size() -> usize {
        0
    }
    fn encoded_len(&self) -> usize {
        match &self.0 {
            None => 1,
            Some(p) => 1 + p.encoded_len(),
        }
    }
    fn ssz_append(&self, buf: &mut Vec<u8>) {
        match &self.0 {
            None => buf.push(0),
            Some(p) => {
                buf.push(1);
                p.ssz_append(buf);
            }
        }
    }
}

impl SszDecode for OptionalProof {
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
                Ok(OptionalProof(None))
            }
            1 => {
                let p = <[u8; BYTES_PER_PROOF]>::from_ssz_bytes(&bytes[1..])?;
                Ok(OptionalProof(Some(p)))
            }
            s => Err(DecodeError::InvalidUnionSelector(s)),
        }
    }
}

/// `/blobs/v4` contents: per-cell nullable cells + proofs.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct BlobCellsAndProofs {
    pub blob_cells: SszList<OptionalCell, CELLS_PER_EXT_BLOB>,
    pub proofs: SszList<OptionalProof, CELLS_PER_EXT_BLOB>,
}

/// `/blobs/v4` response entry.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct BlobV4Entry {
    pub available: bool,
    pub contents: BlobCellsAndProofs,
}

/// `/blobs/v4` response.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct BlobsV4Response {
    pub entries: SszList<BlobV4Entry, MAX_BLOBS_REQUEST>,
}
