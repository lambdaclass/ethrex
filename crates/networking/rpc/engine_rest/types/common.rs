//! Fork-invariant SSZ types for the engine REST API.

use std::str::FromStr;

use libssz::{SszDecode, SszEncode};
use libssz_derive::{HashTreeRoot, SszDecode, SszEncode};
use libssz_merkle::{HashTreeRoot, Sha256Hasher};
use libssz_types::{SszList, SszVector};

/// Spec limits shared across all forks.
pub const MAX_EXTRA_DATA_BYTES: usize = 32;
pub const MAX_BYTES_PER_TRANSACTION: usize = 1_073_741_824;
pub const MAX_TRANSACTIONS_PER_PAYLOAD: usize = 1_048_576;
/// `MAX_WITHDRAWALS_PER_PAYLOAD` — Capella SSZ list limit (`2**4`), per
/// execution-apis #793 (`refactor-ssz.md`) and the Capella beacon-chain spec.
pub const MAX_WITHDRAWALS_PER_PAYLOAD: usize = 16;
/// `MAX_EXECUTION_REQUESTS_PER_PAYLOAD` (`2**8`) per refactor-ssz.md; matches the
/// consensoor CL.
pub const MAX_EXECUTION_REQUESTS_PER_PAYLOAD: usize = 256;
/// Spec limit on bytes per single execution-request payload (type-prefix + body).
pub const MAX_REQUEST_BYTES: usize = 16_777_216; // 16 MiB

/// `BYTES_PER_LOGS_BLOOM` from the CL spec.
pub const BYTES_PER_LOGS_BLOOM: usize = 256;

/// `ByteVector[256]` — the logs bloom as a fixed-size SSZ vector.
pub type LogsBloom = SszVector<u8, BYTES_PER_LOGS_BLOOM>;

// ── Bytes20 wrapper (address) ──────────────────────────────────────
//
// libssz implements `SszEncode`/`SszDecode` for `[u8; 20]` but NOT
// `HashTreeRoot`. Per the SSZ spec, a 20-byte basic value is
// right-padded with zeros to 32 bytes for its tree hash leaf.

/// A 20-byte value (e.g. an execution address) with SSZ + HTR support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct Bytes20(pub [u8; 20]);

impl SszEncode for Bytes20 {
    fn is_fixed_size() -> bool {
        true
    }
    fn fixed_size() -> usize {
        20
    }
    fn encoded_len(&self) -> usize {
        20
    }
    fn ssz_append(&self, buf: &mut Vec<u8>) {
        self.0.ssz_append(buf);
    }
}

impl SszDecode for Bytes20 {
    fn is_fixed_size() -> bool {
        true
    }
    fn fixed_size() -> usize {
        20
    }
    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, libssz::DecodeError> {
        <[u8; 20]>::from_ssz_bytes(bytes).map(Self)
    }
}

impl HashTreeRoot for Bytes20 {
    fn hash_tree_root(&self, _hasher: &impl Sha256Hasher) -> libssz_merkle::Node {
        let mut node = [0u8; 32];
        node[..20].copy_from_slice(&self.0);
        node
    }
}

impl From<[u8; 20]> for Bytes20 {
    fn from(bytes: [u8; 20]) -> Self {
        Self(bytes)
    }
}

impl From<Bytes20> for [u8; 20] {
    fn from(b: Bytes20) -> Self {
        b.0
    }
}

/// Spec limit on raw block_access_list bytes per payload (EIP-7928).
pub const MAX_BLOCK_ACCESS_LIST_BYTES: usize = 16_777_216; // 16 MiB

/// Spec limit for `validation_error` strings (`MAX_ERROR_BYTES`).
pub const MAX_ERROR_BYTES: usize = 1024;

/// SSZ `String` ≡ `List[byte, MAX_ERROR_BYTES]` (refactor.md convention).
pub type ErrorString = SszList<u8, MAX_ERROR_BYTES>;

/// Numeric status codes used in `PayloadStatus`.
///
/// `Accepted` (3) is only valid for `/payloads`. `/forkchoice` MUST NOT return 3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PayloadStatusCode {
    Valid = 0,
    Invalid = 1,
    Syncing = 2,
    Accepted = 3,
}

// ── Optional[T] ≡ List[T, 1] ───────────────────────────────────────────────
//
// refactor.md SSZ encoding conventions: `Optional[T]` is encoded as an SSZ
// `List[T, 1]` — an empty list means absent, a single element means present.
// (NOT a union with a selector byte.) These helpers bridge the ergonomic
// Rust `Option<T>` used by handlers and the `SszList<T, 1>` wire field.

/// `Option<T>` → `List[T, 1]`.
pub(crate) fn to_optional<T>(opt: Option<T>) -> SszList<T, 1> {
    opt.into_iter()
        .collect::<Vec<T>>()
        .try_into()
        .unwrap_or_else(|_| unreachable!("0 or 1 element always fits List[T, 1]"))
}

/// `List[T, 1]` → `Option<T>`.
pub(crate) fn from_optional<T: Clone>(list: &SszList<T, 1>) -> Option<T> {
    list.iter().next().cloned()
}

// ── PayloadStatus ────────────────────────────────────────────────────────────

/// SSZ payload status — `/payloads` response body (refactor-ssz.md § PayloadStatus).
///
/// Wire layout (SSZ container): `status: uint8`, `latest_valid_hash: List[Bytes32, 1]`,
/// `validation_error: List[String, 1]`.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct PayloadStatus {
    pub status: u8,
    pub latest_valid_hash: SszList<[u8; 32], 1>,
    pub validation_error: SszList<ErrorString, 1>,
}

impl PayloadStatus {
    /// Build from the ergonomic `Option` representation. A `validation_error`
    /// longer than `MAX_ERROR_BYTES` is truncated to fit the SSZ bound.
    pub fn new(
        status: u8,
        latest_valid_hash: Option<[u8; 32]>,
        validation_error: Option<String>,
    ) -> Self {
        let ve = validation_error.map(|s| {
            let mut bytes = s.into_bytes();
            bytes.truncate(MAX_ERROR_BYTES);
            bytes
                .try_into()
                .unwrap_or_else(|_| unreachable!("truncated error fits MAX_ERROR_BYTES"))
        });
        PayloadStatus {
            status,
            latest_valid_hash: to_optional(latest_valid_hash),
            validation_error: to_optional(ve),
        }
    }

    /// The latest valid hash, if present.
    pub fn latest_valid_hash(&self) -> Option<[u8; 32]> {
        from_optional(&self.latest_valid_hash)
    }

    /// The validation error decoded as a (lossy) UTF-8 string, if present.
    pub fn validation_error_string(&self) -> Option<String> {
        from_optional(&self.validation_error)
            .map(|e| String::from_utf8_lossy(e.as_ref()).into_owned())
    }
}

// ── ForkchoiceState ───────────────────────────────────────────────────────────

/// SSZ ForkchoiceState — the heads/safe/finalized triple submitted to `/forkchoice`.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct ForkchoiceState {
    pub head_block_hash: [u8; 32],
    pub safe_block_hash: [u8; 32],
    pub finalized_block_hash: [u8; 32],
}

// ── PayloadId ─────────────────────────────────────────────────────────────────

/// 8-byte build-job identifier returned by `/forkchoice` and consumed by `/payloads/{id}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
#[ssz(transparent)]
pub struct PayloadId(pub [u8; 8]);

impl PayloadId {
    /// Render as `0x`-prefixed lowercase hex (16 hex chars).
    pub fn to_hex_string(&self) -> String {
        format!("0x{}", hex::encode(self.0))
    }

    /// Big-endian u64 view (for legacy interop with `Blockchain::get_payload`).
    pub fn as_u64(&self) -> u64 {
        u64::from_be_bytes(self.0)
    }

    /// Build from a big-endian u64.
    pub fn from_u64(v: u64) -> Self {
        Self(v.to_be_bytes())
    }
}

impl FromStr for PayloadId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let hex = s
            .strip_prefix("0x")
            .ok_or_else(|| "PayloadId must start with 0x".to_string())?;
        if hex.len() != 16 {
            return Err(format!(
                "PayloadId must be exactly 8 bytes (16 hex chars), got {}",
                hex.len()
            ));
        }
        let mut bytes = [0u8; 8];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
                .map_err(|e| format!("invalid hex: {e}"))?;
        }
        Ok(Self(bytes))
    }
}

// ── ForkchoiceResponse ────────────────────────────────────────────────────────

/// `/forkchoice` response carrying the resulting status and (if attributes were
/// supplied) the payload-build id (`Optional[Bytes8]` ≡ `List[Bytes8, 1]`).
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct ForkchoiceResponse {
    pub payload_status: PayloadStatus,
    pub payload_id: SszList<PayloadId, 1>,
}

impl ForkchoiceResponse {
    /// Build from the ergonomic `Option` representation.
    pub fn new(payload_status: PayloadStatus, payload_id: Option<PayloadId>) -> Self {
        ForkchoiceResponse {
            payload_status,
            payload_id: to_optional(payload_id),
        }
    }

    /// The payload-build id, if present.
    pub fn payload_id(&self) -> Option<PayloadId> {
        from_optional(&self.payload_id)
    }
}
