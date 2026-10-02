//! Per-fork ForkchoiceUpdate SSZ types — wrap fork-specific PayloadAttributes.
//!
//! Each `*ForkchoiceUpdate` carries a `ForkchoiceState` plus an
//! `Optional[PayloadAttributes]` for the matching fork. Per refactor.md,
//! `Optional[T]` is encoded as `List[T, 1]` (empty = absent, one element =
//! present), so these are plain derived SSZ containers — no hand-written union.
//!
//! `AmsterdamForkchoiceUpdate` adds a third field, `custody_columns`
//! (`Optional[Bitvector[CELLS_PER_EXT_BLOB]]`), per execution-apis #793. It is a
//! sibling of `payload_attributes` in the request body, NOT a `PayloadAttributes`
//! field. `custody_columns` is decode-only: ethrex ignores it until PeerDAS lands.

use libssz_derive::{HashTreeRoot, SszDecode, SszEncode};
use libssz_types::{SszBitvector, SszList};

use super::blobs::CELLS_PER_EXT_BLOB;
use super::common::ForkchoiceState;
use super::{amsterdam, cancun, paris, prague, shanghai};

// ── Macro: generate the Paris..Prague ForkchoiceUpdate types ──────────────────

macro_rules! forkchoice_update {
    ($name:ident, $attrs:ty) => {
        #[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
        pub struct $name {
            pub state: ForkchoiceState,
            pub payload_attributes: SszList<$attrs, 1>,
        }
    };
}

forkchoice_update!(ParisForkchoiceUpdate, paris::PayloadAttributes);
forkchoice_update!(ShanghaiForkchoiceUpdate, shanghai::PayloadAttributes);
forkchoice_update!(CancunForkchoiceUpdate, cancun::PayloadAttributes);
forkchoice_update!(PragueForkchoiceUpdate, prague::PayloadAttributes);

/// Osaka uses Prague-shaped attributes (no new fields).
pub type OsakaForkchoiceUpdate = PragueForkchoiceUpdate;

// ── Amsterdam: state + payload_attributes + custody_columns ───────────────────

/// `Optional[Bitvector[CELLS_PER_EXT_BLOB]]` ≡ `List[Bitvector[..], 1]`.
pub type CustodyColumns = SszBitvector<CELLS_PER_EXT_BLOB>;

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct AmsterdamForkchoiceUpdate {
    pub state: ForkchoiceState,
    pub payload_attributes: SszList<amsterdam::PayloadAttributes, 1>,
    pub custody_columns: SszList<CustodyColumns, 1>,
}
