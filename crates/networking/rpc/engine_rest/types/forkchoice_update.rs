//! Per-fork ForkchoiceUpdate SSZ types — wrap fork-specific PayloadAttributes.
//!
//! Each `*ForkchoiceUpdate` carries a `ForkchoiceState` plus an
//! `Option<PayloadAttributes>` for the matching fork. Because libssz-derive
//! does not provide a blanket impl for `Option<T>` where T is a variable-length
//! container, we implement SSZ manually (a macro for Paris..Prague, plus a
//! hand-written Amsterdam impl), mirroring the manual `ForkchoiceResponse` impl
//! in `common.rs`.
//!
//! Wire layout for the Paris..Osaka `*ForkchoiceUpdate` (SSZ container):
//!   state               : fixed 96 bytes  (ForkchoiceState = three [u8;32] fields)
//!   payload_attributes  : Offset(4)       (variable, union: 0x00 | 0x01 ++ <attrs>)
//!
//! `AmsterdamForkchoiceUpdate` adds a third field, `custody_columns`, per
//! execution-apis #793 (it is a sibling of `payload_attributes` in the request
//! body, NOT a `PayloadAttributes` field), so it gets a hand-written impl below.

use libssz::{ContainerDecoder, ContainerEncoder, DecodeError, SszDecode, SszEncode};
use libssz_types::SszBitvector;

use super::blobs::CELLS_PER_EXT_BLOB;
use super::common::ForkchoiceState;
use super::{amsterdam, cancun, paris, prague, shanghai};

// ForkchoiceState is a derived fixed-size SSZ container (3 × 32 = 96 bytes).
const FORKCHOICE_STATE_FIXED: usize = 96;

// ── Option<T> SSZ union helpers ───────────────────────────────────────────────
//
// Selector 0 = None (1 byte total).
// Selector 1 = Some(value) (1 + value.encoded_len() bytes total).

fn encoded_len_option<T: SszEncode>(opt: &Option<T>) -> usize {
    match opt {
        None => 1,
        Some(v) => 1 + v.encoded_len(),
    }
}

fn decode_option<T: SszDecode>(bytes: &[u8]) -> Result<Option<T>, DecodeError> {
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
            Ok(None)
        }
        1 => {
            let inner = T::from_ssz_bytes(&bytes[1..])?;
            Ok(Some(inner))
        }
        s => Err(DecodeError::InvalidUnionSelector(s)),
    }
}

// Proxy wrapper for ContainerEncoder::append_variable on Option<T>.
struct OptionProxy<'a, T>(&'a Option<T>);

impl<T: SszEncode> SszEncode for OptionProxy<'_, T> {
    fn is_fixed_size() -> bool {
        false
    }
    fn fixed_size() -> usize {
        0
    }
    fn encoded_len(&self) -> usize {
        encoded_len_option(self.0)
    }
    fn ssz_append(&self, buf: &mut Vec<u8>) {
        match self.0 {
            None => buf.push(0),
            Some(v) => {
                buf.push(1);
                v.ssz_append(buf);
            }
        }
    }
}

// ── Macro: generate all six ForkchoiceUpdate types ────────────────────────────

macro_rules! forkchoice_update {
    ($name:ident, $attrs:ty) => {
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct $name {
            pub state: ForkchoiceState,
            pub payload_attributes: Option<$attrs>,
        }

        impl SszEncode for $name {
            fn is_fixed_size() -> bool {
                false
            }

            fn fixed_size() -> usize {
                0
            }

            fn encoded_len(&self) -> usize {
                // fixed part: 96 bytes (state) + 4 bytes (offset for payload_attributes)
                // variable part: encoded Option<attrs>
                FORKCHOICE_STATE_FIXED + 4 + encoded_len_option(&self.payload_attributes)
            }

            fn ssz_append(&self, buf: &mut Vec<u8>) {
                // fixed_part_len = 96 (state inlined) + 4 (offset placeholder) = 100
                let fixed_part_len = FORKCHOICE_STATE_FIXED + 4;
                let mut enc =
                    ContainerEncoder::with_capacity(buf, fixed_part_len, self.encoded_len());
                // ForkchoiceState is a fixed-size derived type (3 × [u8;32]).
                enc.append_fixed(&self.state);
                enc.append_variable(&OptionProxy(&self.payload_attributes));
                enc.finalize();
            }
        }

        impl SszDecode for $name {
            fn is_fixed_size() -> bool {
                false
            }

            fn fixed_size() -> usize {
                0
            }

            fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
                // fixed_part_len = 96 (state) + 4 (offset for attrs) = 100
                let fixed_part_len = FORKCHOICE_STATE_FIXED + 4;
                let mut dec = ContainerDecoder::new(bytes, fixed_part_len)?;

                // Decode fixed ForkchoiceState (96 bytes).
                let state = dec.decode_fixed::<ForkchoiceState>()?;

                // Read the offset for the variable payload_attributes field.
                dec.read_variable_offset()?;

                // Decode the variable-length union bytes for Option<attrs>.
                let attrs_bytes = dec.decode_variable::<Vec<u8>>()?;
                let payload_attributes = decode_option::<$attrs>(&attrs_bytes)?;

                Ok($name {
                    state,
                    payload_attributes,
                })
            }
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
//
// Per execution-apis #793 the Amsterdam `ForkchoiceUpdate` carries a third
// field alongside `forkchoice_state` and `payload_attributes`:
//   custody_columns: Optional[Bitvector[CELLS_PER_EXT_BLOB]]   (16 bytes when present)
// Both `payload_attributes` and `custody_columns` are variable-size (Optional →
// union), so the fixed part is 96 (state) + 4 + 4 = 104 bytes. `custody_columns`
// is decode-only: ethrex ignores it until PeerDAS execution lands.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmsterdamForkchoiceUpdate {
    pub state: ForkchoiceState,
    pub payload_attributes: Option<amsterdam::PayloadAttributes>,
    pub custody_columns: Option<SszBitvector<CELLS_PER_EXT_BLOB>>,
}

const AMSTERDAM_FIXED_PART: usize = FORKCHOICE_STATE_FIXED + 4 + 4;

impl SszEncode for AmsterdamForkchoiceUpdate {
    fn is_fixed_size() -> bool {
        false
    }

    fn fixed_size() -> usize {
        0
    }

    fn encoded_len(&self) -> usize {
        AMSTERDAM_FIXED_PART
            + encoded_len_option(&self.payload_attributes)
            + encoded_len_option(&self.custody_columns)
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        let mut enc =
            ContainerEncoder::with_capacity(buf, AMSTERDAM_FIXED_PART, self.encoded_len());
        enc.append_fixed(&self.state);
        enc.append_variable(&OptionProxy(&self.payload_attributes));
        enc.append_variable(&OptionProxy(&self.custody_columns));
        enc.finalize();
    }
}

impl SszDecode for AmsterdamForkchoiceUpdate {
    fn is_fixed_size() -> bool {
        false
    }

    fn fixed_size() -> usize {
        0
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut dec = ContainerDecoder::new(bytes, AMSTERDAM_FIXED_PART)?;

        // Decode fixed ForkchoiceState (96 bytes).
        let state = dec.decode_fixed::<ForkchoiceState>()?;

        // Read the two variable-field offsets (payload_attributes, custody_columns).
        dec.read_variable_offset()?;
        dec.read_variable_offset()?;

        // Decode the variable-length union bytes, in field order.
        let attrs_bytes = dec.decode_variable::<Vec<u8>>()?;
        let custody_bytes = dec.decode_variable::<Vec<u8>>()?;
        let payload_attributes = decode_option::<amsterdam::PayloadAttributes>(&attrs_bytes)?;
        let custody_columns = decode_option::<SszBitvector<CELLS_PER_EXT_BLOB>>(&custody_bytes)?;

        Ok(AmsterdamForkchoiceUpdate {
            state,
            payload_attributes,
            custody_columns,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libssz::{SszDecode, SszEncode};

    #[test]
    fn paris_forkchoice_update_roundtrips_none_attrs() {
        let update = ParisForkchoiceUpdate {
            state: ForkchoiceState {
                head_block_hash: [1; 32],
                safe_block_hash: [2; 32],
                finalized_block_hash: [3; 32],
            },
            payload_attributes: None,
        };
        let bytes = update.to_ssz();
        let back = ParisForkchoiceUpdate::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back, update);
        assert!(back.payload_attributes.is_none());
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
            payload_attributes: Some(paris::PayloadAttributes {
                timestamp: 1_700_000_001,
                prev_randao: [9; 32],
                suggested_fee_recipient: Bytes20([10; 20]),
            }),
        };
        let bytes = update.to_ssz();
        let back = ParisForkchoiceUpdate::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back, update);
        let attrs = back.payload_attributes.unwrap();
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
            payload_attributes: Some(cancun::PayloadAttributes {
                timestamp: 9_999,
                prev_randao: [7; 32],
                suggested_fee_recipient: Bytes20([8; 20]),
                withdrawals: vec![].try_into().unwrap(),
                parent_beacon_block_root: [0xDD; 32],
            }),
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
            payload_attributes: Some(amsterdam::PayloadAttributes {
                timestamp: 1_700_000_123,
                prev_randao: [4; 32],
                suggested_fee_recipient: Bytes20([5; 20]),
                withdrawals: vec![].try_into().unwrap(),
                parent_beacon_block_root: [0xEE; 32],
                slot_number: 9_001,
                target_gas_limit: 30_000_000,
            }),
            custody_columns: Some(custody),
        };
        let bytes = update.to_ssz();
        let back = AmsterdamForkchoiceUpdate::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back, update);
        let attrs = back.payload_attributes.unwrap();
        assert_eq!(attrs.slot_number, 9_001);
        assert_eq!(attrs.target_gas_limit, 30_000_000);
        let c = back.custody_columns.unwrap();
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
            payload_attributes: None,
            custody_columns: None,
        };
        let bytes = update.to_ssz();
        let back = AmsterdamForkchoiceUpdate::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back, update);
    }
}
