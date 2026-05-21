//! Fork-invariant SSZ types and constants.

use libssz_derive::{SszDecode, SszEncode};
use libssz_types::SszList;
use thiserror::Error;

pub const MAX_BYTES_PER_TRANSACTION: usize = 1 << 30; // 2**30
pub const MAX_TRANSACTIONS_PER_PAYLOAD: usize = 1 << 20; // 2**20
pub const MAX_WITHDRAWALS_PER_PAYLOAD: usize = 16; // 2**4
pub const BYTES_PER_LOGS_BLOOM: usize = 256;
pub const MAX_EXTRA_DATA_BYTES: usize = 32; // 2**5
pub const MAX_BLOB_COMMITMENTS_PER_BLOCK: usize = 4096; // 2**12
pub const FIELD_ELEMENTS_PER_BLOB: usize = 4096;
pub const BYTES_PER_FIELD_ELEMENT: usize = 32;
pub const CELLS_PER_EXT_BLOB: usize = 128;
// Also the SSZ `List` length bound on the `block_hashes` / `payload_bodies`
// containers, so larger requests can't be decoded over this transport.
pub const MAX_PAYLOAD_BODIES_REQUEST: usize = 32; // 2**5
pub const MAX_BLOB_HASHES_REQUEST: usize = 128;
pub const MAX_EXECUTION_REQUESTS: usize = 256; // 2**8
pub const MAX_ERROR_MESSAGE_LENGTH: usize = 1024;
pub const MAX_CLIENT_CODE_LENGTH: usize = 2;
pub const MAX_CLIENT_NAME_LENGTH: usize = 64;
pub const MAX_CLIENT_VERSION_LENGTH: usize = 64;
pub const MAX_CLIENT_VERSIONS: usize = 4;
pub const MAX_CAPABILITY_NAME_LENGTH: usize = 64;
pub const MAX_CAPABILITIES: usize = 64;
pub const BLOB_SIZE: usize = FIELD_ELEMENTS_PER_BLOB * BYTES_PER_FIELD_ELEMENT; // 131_072
pub const MAX_BLOB_PROOFS_PER_BUNDLE: usize = MAX_BLOB_COMMITMENTS_PER_BLOCK * CELLS_PER_EXT_BLOB;

pub type Bytes4 = [u8; 4];
pub type Bytes8 = [u8; 8];
pub type Bytes20 = [u8; 20];
pub type Bytes32 = [u8; 32];
pub type Bytes48 = [u8; 48];
pub type LogsBloom = [u8; BYTES_PER_LOGS_BLOOM];
pub type Blob = [u8; BLOB_SIZE];

/// SSZ uint256 encoded as little-endian Bytes32.
pub type Uint256 = [u8; 32];

/// Convert a u64 to the SSZ uint256 (little-endian, 32-byte) representation.
pub fn u64_to_uint256_le(v: u64) -> Uint256 {
    let mut out = [0u8; 32];
    out[..8].copy_from_slice(&v.to_le_bytes());
    out
}

/// Convert an ethrex_common::U256 to the SSZ uint256 (little-endian) representation.
pub fn u256_to_uint256_le(v: ethrex_common::U256) -> Uint256 {
    v.to_little_endian()
}

/// Decode SSZ uint256 (little-endian) back to ethrex_common::U256.
pub fn uint256_le_to_u256(v: &Uint256) -> ethrex_common::U256 {
    ethrex_common::U256::from_little_endian(v)
}

/// Decode SSZ uint256 (little-endian) to u64. Returns None if any high byte is non-zero.
pub fn uint256_le_to_u64(v: &Uint256) -> Option<u64> {
    if v[8..].iter().any(|&b| b != 0) {
        return None;
    }
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&v[..8]);
    Some(u64::from_le_bytes(bytes))
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadStatusCode {
    Valid = 0,
    Invalid = 1,
    Syncing = 2,
    Accepted = 3,
}

// `latest_valid_hash` uses nullable encoding (`List[Bytes32, 1]`);
// `validation_error` is a ByteList where empty = absent.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct PayloadStatusV1 {
    pub status: u8,
    pub latest_valid_hash: SszList<Bytes32, 1>,
    pub validation_error: SszList<u8, MAX_ERROR_MESSAGE_LENGTH>,
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, Default)]
pub struct ForkchoiceStateV1 {
    pub head_block_hash: Bytes32,
    pub safe_block_hash: Bytes32,
    pub finalized_block_hash: Bytes32,
}

// `payload_id` uses nullable encoding (`List[Bytes8, 1]`).
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
pub struct ForkchoiceUpdatedResponseV1 {
    pub payload_status: PayloadStatusV1,
    pub payload_id: SszList<Bytes8, 1>,
}

/// Hex-encoded `Bytes8` path parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PayloadId(pub Bytes8);

impl PayloadId {
    pub fn as_u64(self) -> u64 {
        u64::from_be_bytes(self.0)
    }

    pub fn from_u64(v: u64) -> Self {
        PayloadId(v.to_be_bytes())
    }
}

#[derive(Debug, Error)]
pub enum PayloadIdParseError {
    #[error("payload_id must be 0x-prefixed")]
    MissingPrefix,
    #[error("payload_id must be 16 hex chars (8 bytes), got {0}")]
    WrongLength(usize),
    #[error("invalid hex: {0}")]
    InvalidHex(#[from] hex::FromHexError),
}

impl core::str::FromStr for PayloadId {
    type Err = PayloadIdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let hex = s
            .strip_prefix("0x")
            .ok_or(PayloadIdParseError::MissingPrefix)?;
        if hex.len() != 16 {
            return Err(PayloadIdParseError::WrongLength(hex.len()));
        }
        let bytes = hex::decode(hex)?;
        let mut arr = [0u8; 8];
        arr.copy_from_slice(&bytes);
        Ok(PayloadId(arr))
    }
}

/// SSZ wire encoding of `Option<T>` as `List[T, 1]`: 0 elements = `None`,
/// 1 element = `Some`. The list-ness is a wire-format convention only —
/// semantically this is an `Option<T>`.
pub type SszOption<T> = SszList<T, 1>;

pub fn ssz_some<T: Clone>(v: T) -> SszOption<T>
where
    SszOption<T>: TryFrom<Vec<T>>,
    <SszOption<T> as TryFrom<Vec<T>>>::Error: core::fmt::Debug,
{
    vec![v]
        .try_into()
        .expect("single element fits in SszOption<T>")
}

pub fn ssz_none<T: Clone>() -> SszOption<T>
where
    SszOption<T>: TryFrom<Vec<T>>,
    <SszOption<T> as TryFrom<Vec<T>>>::Error: core::fmt::Debug,
{
    Vec::<T>::new()
        .try_into()
        .expect("empty list fits in SszOption<T>")
}

/// Read an SSZ-encoded `Option<T>` back to `Option<T>`.
pub fn ssz_into_option<T: Clone, const N: usize>(list: &SszList<T, N>) -> Option<T> {
    list.first().cloned()
}
