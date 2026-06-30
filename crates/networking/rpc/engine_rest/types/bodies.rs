//! SSZ wire types for the engine REST bodies endpoints (execution-apis #793).
//!
//! Request (`POST /bodies/hash`) is a bare `List[Hash32, MAX_BODIES_REQUEST]`.
//! Response is a bare `List[BodyEntry, MAX_BODIES_REQUEST]` — NOT wrapped in a
//! named container — where `BodyEntry { available: Boolean, body: ExecutionPayloadBody }`.
//! When `available == false` the `body` is zero-valued (every list empty) and CLs
//! MUST ignore it. Each fork URL returns only its own era's blocks.

use libssz_derive::{HashTreeRoot, SszDecode, SszEncode};
use libssz_types::SszList;

use super::common::{
    MAX_BLOCK_ACCESS_LIST_BYTES, MAX_BYTES_PER_TRANSACTION, MAX_TRANSACTIONS_PER_PAYLOAD,
    MAX_WITHDRAWALS_PER_PAYLOAD,
};
use super::shanghai::Withdrawal;

/// Spec cap on hashes per `/bodies/hash` request and on entries in any
/// bodies response (`MAX_BODIES_REQUEST = 2**5`); matches the consensoor CL.
pub const MAX_BODIES_PER_REQUEST: usize = 32;

/// Inner block-hash list wrapped by `BodiesByHashRequest`.
pub type BlockHashList = SszList<[u8; 32], MAX_BODIES_PER_REQUEST>;

/// `POST /bodies/hash` request. Per execution-apis #793 the request is a
/// single-field SSZ **container** wrapping the list, NOT a bare top-level list.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BodiesByHashRequest {
    pub block_hashes: BlockHashList,
}

// ── Per-fork ExecutionPayloadBody ─────────────────────────────────────────────

/// Paris body: transactions only.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BodyParis {
    pub transactions: SszList<SszList<u8, MAX_BYTES_PER_TRANSACTION>, MAX_TRANSACTIONS_PER_PAYLOAD>,
}

/// Shanghai/Cancun/Prague/Osaka body: transactions + withdrawals.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BodyShanghai {
    pub transactions: SszList<SszList<u8, MAX_BYTES_PER_TRANSACTION>, MAX_TRANSACTIONS_PER_PAYLOAD>,
    pub withdrawals: SszList<Withdrawal, MAX_WITHDRAWALS_PER_PAYLOAD>,
}

/// Amsterdam body: BodyShanghai + raw block_access_list bytes.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BodyAmsterdam {
    pub transactions: SszList<SszList<u8, MAX_BYTES_PER_TRANSACTION>, MAX_TRANSACTIONS_PER_PAYLOAD>,
    pub withdrawals: SszList<Withdrawal, MAX_WITHDRAWALS_PER_PAYLOAD>,
    pub block_access_list: SszList<u8, MAX_BLOCK_ACCESS_LIST_BYTES>,
}

impl BodyParis {
    /// Zero-valued body for an `available == false` entry (CLs MUST ignore it).
    pub fn empty() -> Self {
        BodyParis {
            transactions: Vec::new().try_into().expect("empty list fits"),
        }
    }
}

impl BodyShanghai {
    /// Zero-valued body for an `available == false` entry (CLs MUST ignore it).
    pub fn empty() -> Self {
        BodyShanghai {
            transactions: Vec::new().try_into().expect("empty list fits"),
            withdrawals: Vec::new().try_into().expect("empty list fits"),
        }
    }
}

impl BodyAmsterdam {
    /// Zero-valued body for an `available == false` entry (CLs MUST ignore it).
    pub fn empty() -> Self {
        BodyAmsterdam {
            transactions: Vec::new().try_into().expect("empty list fits"),
            withdrawals: Vec::new().try_into().expect("empty list fits"),
            block_access_list: Vec::new().try_into().expect("empty list fits"),
        }
    }
}

// ── Per-fork BodyEntry { available, body } ────────────────────────────────────

/// Paris bodies response entry.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BodyEntryParis {
    pub available: bool,
    pub body: BodyParis,
}

/// Shanghai/Cancun/Prague/Osaka bodies response entry.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BodyEntryShanghai {
    pub available: bool,
    pub body: BodyShanghai,
}

/// Amsterdam bodies response entry.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BodyEntryAmsterdam {
    pub available: bool,
    pub body: BodyAmsterdam,
}

impl BodyEntryParis {
    pub fn available(body: BodyParis) -> Self {
        Self {
            available: true,
            body,
        }
    }
    pub fn unavailable() -> Self {
        Self {
            available: false,
            body: BodyParis::empty(),
        }
    }
}

impl BodyEntryShanghai {
    pub fn available(body: BodyShanghai) -> Self {
        Self {
            available: true,
            body,
        }
    }
    pub fn unavailable() -> Self {
        Self {
            available: false,
            body: BodyShanghai::empty(),
        }
    }
}

impl BodyEntryAmsterdam {
    pub fn available(body: BodyAmsterdam) -> Self {
        Self {
            available: true,
            body,
        }
    }
    pub fn unavailable() -> Self {
        Self {
            available: false,
            body: BodyAmsterdam::empty(),
        }
    }
}

// ── Response containers (single-field, per execution-apis #793) ───────────────
//
// Shared by both `POST /bodies/hash` and `GET /bodies` (range).

/// Paris bodies response: `{ entries: List[BodyEntryParis, N] }`.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BodiesResponseParis {
    pub entries: SszList<BodyEntryParis, MAX_BODIES_PER_REQUEST>,
}
/// Shanghai/Cancun/Prague/Osaka bodies response.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BodiesResponseShanghai {
    pub entries: SszList<BodyEntryShanghai, MAX_BODIES_PER_REQUEST>,
}
/// Amsterdam bodies response.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BodiesResponseAmsterdam {
    pub entries: SszList<BodyEntryAmsterdam, MAX_BODIES_PER_REQUEST>,
}
