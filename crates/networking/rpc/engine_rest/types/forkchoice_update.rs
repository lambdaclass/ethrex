//! Per-fork ForkchoiceUpdate SSZ types — wrap fork-specific PayloadAttributes.
//!
//! Each `*ForkchoiceUpdate` carries a `ForkchoiceState` plus an
//! `Option<PayloadAttributes>` for the matching fork. Because libssz-derive
//! does not provide a blanket impl for `Option<T>` where T is a variable-length
//! container, we implement SSZ manually for all six types, mirroring the
//! manual `ForkchoiceResponse` impl in `common.rs`.
//!
//! Wire layout for every `*ForkchoiceUpdate` (SSZ container):
//!   state               : fixed 96 bytes  (ForkchoiceState = three [u8;32] fields)
//!   payload_attributes  : Offset(4)       (variable, union: 0x00 | 0x01 ++ <attrs>)

use libssz::{ContainerDecoder, ContainerEncoder, DecodeError, SszDecode, SszEncode};

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
forkchoice_update!(AmsterdamForkchoiceUpdate, amsterdam::PayloadAttributes);

/// Osaka uses Prague-shaped attributes (no new fields).
pub type OsakaForkchoiceUpdate = PragueForkchoiceUpdate;

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
}
