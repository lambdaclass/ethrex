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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine_rest::types::common::{from_optional, to_optional};
    use libssz::{SszDecode, SszEncode};

    #[test]
    fn paris_forkchoice_update_roundtrips_none_attrs() {
        let update = ParisForkchoiceUpdate {
            state: ForkchoiceState {
                head_block_hash: [1; 32],
                safe_block_hash: [2; 32],
                finalized_block_hash: [3; 32],
            },
            payload_attributes: to_optional(None),
        };
        let bytes = update.to_ssz();
        let back = ParisForkchoiceUpdate::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back, update);
        assert!(from_optional(&back.payload_attributes).is_none());
    }

    #[test]
    fn paris_forkchoice_update_roundtrips_some_attrs() {
        use crate::engine_rest::types::common::Bytes20;
        let update = ParisForkchoiceUpdate {
            state: ForkchoiceState {
                head_block_hash: [0xFF; 32],
                safe_block_hash: [0; 32],
                finalized_block_hash: [0; 32],
            },
            payload_attributes: to_optional(Some(paris::PayloadAttributes {
                timestamp: 1_700_000_001,
                prev_randao: [9; 32],
                suggested_fee_recipient: Bytes20([10; 20]),
            })),
        };
        let bytes = update.to_ssz();
        let back = ParisForkchoiceUpdate::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back, update);
        let attrs = from_optional(&back.payload_attributes).unwrap();
        assert_eq!(attrs.timestamp, 1_700_000_001);
    }

    #[test]
    fn cancun_forkchoice_update_roundtrips_some_attrs() {
        use crate::engine_rest::types::common::Bytes20;
        let update = CancunForkchoiceUpdate {
            state: ForkchoiceState {
                head_block_hash: [0xAA; 32],
                safe_block_hash: [0xBB; 32],
                finalized_block_hash: [0xCC; 32],
            },
            payload_attributes: to_optional(Some(cancun::PayloadAttributes {
                timestamp: 9_999,
                prev_randao: [7; 32],
                suggested_fee_recipient: Bytes20([8; 20]),
                withdrawals: vec![].try_into().unwrap(),
                parent_beacon_block_root: [0xDD; 32],
            })),
        };
        let bytes = update.to_ssz();
        let back = CancunForkchoiceUpdate::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back, update);
    }

    #[test]
    fn malformed_bytes_returns_error() {
        let result = CancunForkchoiceUpdate::from_ssz_bytes(&[0u8; 10]);
        assert!(result.is_err());
    }

    #[test]
    fn amsterdam_forkchoice_update_roundtrips_with_custody_columns() {
        use crate::engine_rest::types::common::Bytes20;

        let mut custody = SszBitvector::<CELLS_PER_EXT_BLOB>::new();
        custody.set(3, true).unwrap();
        custody.set(127, true).unwrap();

        let update = AmsterdamForkchoiceUpdate {
            state: ForkchoiceState {
                head_block_hash: [0x11; 32],
                safe_block_hash: [0x22; 32],
                finalized_block_hash: [0x33; 32],
            },
            payload_attributes: to_optional(Some(amsterdam::PayloadAttributes {
                timestamp: 1_700_000_123,
                prev_randao: [4; 32],
                suggested_fee_recipient: Bytes20([5; 20]),
                withdrawals: vec![].try_into().unwrap(),
                parent_beacon_block_root: [0xEE; 32],
                slot_number: 9_001,
                target_gas_limit: 30_000_000,
            })),
            custody_columns: to_optional(Some(custody)),
        };
        let bytes = update.to_ssz();
        let back = AmsterdamForkchoiceUpdate::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back, update);
        let attrs = from_optional(&back.payload_attributes).unwrap();
        assert_eq!(attrs.slot_number, 9_001);
        assert_eq!(attrs.target_gas_limit, 30_000_000);
        let c = from_optional(&back.custody_columns).unwrap();
        assert_eq!(c.get(3), Some(true));
        assert_eq!(c.get(127), Some(true));
        assert_eq!(c.get(0), Some(false));
    }

    #[test]
    fn amsterdam_forkchoice_update_roundtrips_none() {
        let update = AmsterdamForkchoiceUpdate {
            state: ForkchoiceState {
                head_block_hash: [0; 32],
                safe_block_hash: [0; 32],
                finalized_block_hash: [0; 32],
            },
            payload_attributes: to_optional(None),
            custody_columns: to_optional(None),
        };
        let bytes = update.to_ssz();
        let back = AmsterdamForkchoiceUpdate::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back, update);
    }
}
