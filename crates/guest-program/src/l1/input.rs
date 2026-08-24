//! Decoding of `statelessInputBytes`, the L1 guest's only input.
//!
//! The wire format is a 2-byte big-endian schema id followed by the SSZ-encoded
//! `SszStatelessInput`, matching `deserialize_stateless_input` in
//! `stateless_guest.py` at execution-specs `3c3b6f4af` (#3248 + #3278).
//!
//! The containers themselves live in `ethrex_common::types::stateless_ssz`,
//! shared with the EXECUTE precompile path — this module is only the framing.
//! Amsterdam is the sole schema the spec defines, so there is no fork dispatch:
//! the id fully determines both the fork rules and the encoding.

use ethrex_common::types::stateless_ssz::{
    STATELESS_INPUT_SCHEMA_ID, STATELESS_INPUT_SCHEMA_ID_SIZE, SszStatelessInput,
};

/// Decode schema-prefixed `statelessInputBytes`.
///
/// Rejects any schema id other than [`STATELESS_INPUT_SCHEMA_ID`]. That check is
/// load-bearing rather than defensive: since #3278 no chain configuration crosses
/// the wire, so the prefix is the only thing identifying which fork's rules the
/// guest should apply. Upstream rejects it the same way.
pub fn decode_stateless_input(
    bytes: &[u8],
) -> Result<SszStatelessInput, StatelessInputDecodeError> {
    use libssz::SszDecode;

    let (id_bytes, body) = bytes
        .split_first_chunk::<STATELESS_INPUT_SCHEMA_ID_SIZE>()
        .ok_or(StatelessInputDecodeError::MissingSchemaId)?;
    let schema_id = u16::from_be_bytes(*id_bytes);
    if schema_id != STATELESS_INPUT_SCHEMA_ID {
        return Err(StatelessInputDecodeError::UnsupportedSchemaId(schema_id));
    }

    SszStatelessInput::from_ssz_bytes(body).map_err(StatelessInputDecodeError::Ssz)
}

/// Why a `statelessInputBytes` blob could not be decoded.
///
/// Every variant is a decode failure, which the guest commits as the all-zero
/// default result rather than surfacing as an error — see
/// [`super::run_stateless_guest`].
#[derive(Debug)]
pub enum StatelessInputDecodeError {
    /// Fewer than two bytes, so there is no schema id to read.
    MissingSchemaId,
    /// A well-formed id that is not the one schema this guest implements.
    UnsupportedSchemaId(u16),
    /// The id matched but the SSZ body did not decode.
    Ssz(libssz::DecodeError),
}

impl core::fmt::Display for StatelessInputDecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::MissingSchemaId => write!(f, "input too short to contain a schema id"),
            Self::UnsupportedSchemaId(id) => {
                write!(f, "unsupported stateless input schema id: {id:#06x}")
            }
            Self::Ssz(e) => write!(f, "SSZ decode error: {e}"),
        }
    }
}
