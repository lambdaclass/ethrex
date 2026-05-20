//! SSZ wire types for the engine REST bodies endpoints.

use libssz::{DecodeError, SszDecode, SszEncode};
use libssz_derive::{HashTreeRoot, SszDecode, SszEncode};
use libssz_types::SszList;

use super::common::{
    MAX_BLOCK_ACCESS_LIST_BYTES, MAX_BYTES_PER_TRANSACTION, MAX_TRANSACTIONS_PER_PAYLOAD,
    MAX_WITHDRAWALS_PER_PAYLOAD,
};
use super::shanghai::Withdrawal;

/// Spec cap on the number of block hashes in a `/{fork}/bodies/hash` request.
pub const MAX_BODIES_PER_REQUEST: usize = 128;

/// `/{fork}/bodies/hash` request body.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BodiesByHashRequest {
    pub hashes: SszList<[u8; 32], MAX_BODIES_PER_REQUEST>,
}

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

// ── `/{fork}/bodies/hash` response wrappers ───────────────────────────────────
//
// The response is `Vec<Option<Body<fork>>>`. libssz has no blanket
// `Option<NestedStruct>` impl, so we use the SSZ union encoding manually:
//
//   selector = 0x00  → None (no further bytes)
//   selector = 0x01  → Some, followed by the inner body's SSZ encoding
//
// The outer Vec<_> encodes as a standard SSZ list of variable-length elements
// (4-byte little-endian offsets followed by the concatenated payloads), which
// `Vec<T: SszEncode>` already handles when T is variable-size.
//
// We define one wrapper type per fork (`OptBodyParis`, `OptBodyShanghai`,
// `OptBodyAmsterdam`) that implements `SszEncode + SszDecode` as a union, and
// a response struct per fork that holds `Vec<OptBody*>`.

// ── OptBodyParis ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptBodyParis(pub Option<BodyParis>);

impl SszEncode for OptBodyParis {
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

impl SszDecode for OptBodyParis {
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
                Ok(OptBodyParis(None))
            }
            1 => {
                let body = BodyParis::from_ssz_bytes(&bytes[1..])?;
                Ok(OptBodyParis(Some(body)))
            }
            s => Err(DecodeError::InvalidUnionSelector(s)),
        }
    }
}

// ── OptBodyShanghai ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptBodyShanghai(pub Option<BodyShanghai>);

impl SszEncode for OptBodyShanghai {
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

impl SszDecode for OptBodyShanghai {
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
                Ok(OptBodyShanghai(None))
            }
            1 => {
                let body = BodyShanghai::from_ssz_bytes(&bytes[1..])?;
                Ok(OptBodyShanghai(Some(body)))
            }
            s => Err(DecodeError::InvalidUnionSelector(s)),
        }
    }
}

// ── OptBodyAmsterdam ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptBodyAmsterdam(pub Option<BodyAmsterdam>);

impl SszEncode for OptBodyAmsterdam {
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

impl SszDecode for OptBodyAmsterdam {
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
                Ok(OptBodyAmsterdam(None))
            }
            1 => {
                let body = BodyAmsterdam::from_ssz_bytes(&bytes[1..])?;
                Ok(OptBodyAmsterdam(Some(body)))
            }
            s => Err(DecodeError::InvalidUnionSelector(s)),
        }
    }
}

// ── Response wrapper structs ──────────────────────────────────────────────────
//
// Each response wrapper holds a Vec of the per-fork Opt* types. Because
// `Vec<T: SszEncode>` is already implemented in libssz (variable-size list
// encoding), we just need to expose `SszEncode` for the wrapper by delegating
// to the inner Vec.

/// `/{fork}/bodies/hash` response for Paris.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodiesByHashResponseParis {
    pub bodies: Vec<OptBodyParis>,
}

impl SszEncode for BodiesByHashResponseParis {
    fn is_fixed_size() -> bool {
        false
    }
    fn fixed_size() -> usize {
        0
    }
    fn encoded_len(&self) -> usize {
        self.bodies.encoded_len()
    }
    fn ssz_append(&self, buf: &mut Vec<u8>) {
        self.bodies.ssz_append(buf);
    }
}

impl SszDecode for BodiesByHashResponseParis {
    fn is_fixed_size() -> bool {
        false
    }
    fn fixed_size() -> usize {
        0
    }
    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let bodies = Vec::<OptBodyParis>::from_ssz_bytes(bytes)?;
        Ok(BodiesByHashResponseParis { bodies })
    }
}

/// `/{fork}/bodies/hash` response for Shanghai/Cancun/Prague/Osaka.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodiesByHashResponseShanghai {
    pub bodies: Vec<OptBodyShanghai>,
}

impl SszEncode for BodiesByHashResponseShanghai {
    fn is_fixed_size() -> bool {
        false
    }
    fn fixed_size() -> usize {
        0
    }
    fn encoded_len(&self) -> usize {
        self.bodies.encoded_len()
    }
    fn ssz_append(&self, buf: &mut Vec<u8>) {
        self.bodies.ssz_append(buf);
    }
}

impl SszDecode for BodiesByHashResponseShanghai {
    fn is_fixed_size() -> bool {
        false
    }
    fn fixed_size() -> usize {
        0
    }
    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let bodies = Vec::<OptBodyShanghai>::from_ssz_bytes(bytes)?;
        Ok(BodiesByHashResponseShanghai { bodies })
    }
}

/// `/{fork}/bodies/hash` response for Amsterdam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodiesByHashResponseAmsterdam {
    pub bodies: Vec<OptBodyAmsterdam>,
}

impl SszEncode for BodiesByHashResponseAmsterdam {
    fn is_fixed_size() -> bool {
        false
    }
    fn fixed_size() -> usize {
        0
    }
    fn encoded_len(&self) -> usize {
        self.bodies.encoded_len()
    }
    fn ssz_append(&self, buf: &mut Vec<u8>) {
        self.bodies.ssz_append(buf);
    }
}

impl SszDecode for BodiesByHashResponseAmsterdam {
    fn is_fixed_size() -> bool {
        false
    }
    fn fixed_size() -> usize {
        0
    }
    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let bodies = Vec::<OptBodyAmsterdam>::from_ssz_bytes(bytes)?;
        Ok(BodiesByHashResponseAmsterdam { bodies })
    }
}
