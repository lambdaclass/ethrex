//! SSZ wire types for the engine REST blob endpoints (execution-apis #793).
//!
//! Requests are bare `List[VersionedHash, MAX_BLOBS_REQUEST]` (v1/v2/v3) or a
//! `BlobsV4Request` container (v4). Responses are bare `List[BlobV*Entry,
//! MAX_BLOBS_REQUEST]` — NOT wrapped in a named container — matching the CL.
//! Each entry carries an `available` boolean plus `contents`; when `available`
//! is false the `contents` are zero-valued and CLs MUST ignore them. `/blobs/v2`
//! is all-or-nothing (204 when any blob is missing); `/blobs/v3` surfaces missing
//! blobs per entry.

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

/// `/blobs/v1`, `/blobs/v2`, `/blobs/v3` request: a bare `List[VersionedHash, N]`.
pub type VersionedHashList = SszList<[u8; 32], MAX_BLOBS_REQUEST>;

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

// ── Response aliases (bare top-level lists, per the CL) ────────────────────────

/// `/blobs/v1` response.
pub type BlobsV1Response = SszList<BlobV1Entry, MAX_BLOBS_REQUEST>;
/// `/blobs/v2` response (all-or-nothing; every entry is `available`).
pub type BlobsV2Response = SszList<BlobV2Entry, MAX_BLOBS_REQUEST>;
/// `/blobs/v3` response (partial; missing blobs surface as `available == false`).
pub type BlobsV3Response = SszList<BlobV2Entry, MAX_BLOBS_REQUEST>;

// ── `/blobs/v4` types ─────────────────────────────────────────────────────────
//
// TODO: these are the spec's `200 OK` response containers for `/blobs/v4`
// (execution-apis #793). They are not constructed in production yet: the handler
// returns `204 No Content` because the mempool does not store the per-cell blob
// data (EIP-7594/PeerDAS) these responses require. Kept (with round-trip tests)
// so the wire shape is ready when per-cell storage lands; remove or wire up once
// that exists. The request type `BlobsRequestV4` IS live (decoded for validation).
//
// Per the spec convention `Optional[T] ≡ List[T, 1]`, the per-cell optional
// values are `List[Cell, 1]` / `List[Bytes48, 1]` — an empty inner list means
// the cell/proof is absent, matching the CL.

/// `/blobs/v4` contents: per-cell nullable cells + proofs.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct BlobCellsAndProofs {
    pub blob_cells: SszList<SszList<SszVector<u8, BYTES_PER_CELL>, 1>, CELLS_PER_EXT_BLOB>,
    pub proofs: SszList<SszList<[u8; BYTES_PER_PROOF], 1>, CELLS_PER_EXT_BLOB>,
}

/// `/blobs/v4` response entry.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct BlobV4Entry {
    pub available: bool,
    pub contents: BlobCellsAndProofs,
}

/// `/blobs/v4` response.
pub type BlobsV4Response = SszList<BlobV4Entry, MAX_BLOBS_REQUEST>;
