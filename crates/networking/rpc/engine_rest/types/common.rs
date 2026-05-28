//! Fork-invariant SSZ types for the engine REST API.

use std::str::FromStr;

use libssz::{ContainerDecoder, ContainerEncoder, DecodeError, SszDecode, SszEncode};
use libssz_derive::{HashTreeRoot, SszDecode, SszEncode};
use libssz_merkle::{HashTreeRoot, Sha256Hasher};
use libssz_types::SszVector;

/// Spec limits shared across all forks.
pub const MAX_EXTRA_DATA_BYTES: usize = 32;
pub const MAX_BYTES_PER_TRANSACTION: usize = 1_073_741_824;
pub const MAX_TRANSACTIONS_PER_PAYLOAD: usize = 1_048_576;
/// `MAX_WITHDRAWALS_PER_PAYLOAD` — Capella SSZ list limit (`2**4`), per
/// execution-apis #793 (`refactor-ssz.md`) and the Capella beacon-chain spec.
pub const MAX_WITHDRAWALS_PER_PAYLOAD: usize = 16;
/// Spec limit on number of distinct execution request types per payload (EIP-7685).
pub const MAX_EXECUTION_REQUESTS_PER_PAYLOAD: usize = 16;
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
/// Spec limit on custody_columns entries per payload_attributes (PeerDAS).
pub const MAX_CUSTODY_COLUMNS: usize = 128;

/// Spec limit for `validation_error` strings.
pub const MAX_ERROR_BYTES: usize = 1024;

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

// ── Option<[u8; 32]> SSZ union helpers ──────────────────────────────────────
//
// libssz does not provide blanket impls for `Option<T>`. We implement
// SszEncode / SszDecode directly for the two `Option` variants used in this
// module (selector 0 = None, selector 1 = Some(value)).

fn encode_option_hash(opt: &Option<[u8; 32]>, buf: &mut Vec<u8>) {
    match opt {
        None => buf.push(0),
        Some(h) => {
            buf.push(1);
            buf.extend_from_slice(h);
        }
    }
}

fn encoded_len_option_hash(opt: &Option<[u8; 32]>) -> usize {
    match opt {
        None => 1,
        Some(_) => 1 + 32,
    }
}

fn decode_option_hash(bytes: &[u8]) -> Result<Option<[u8; 32]>, DecodeError> {
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
            if bytes.len() != 33 {
                return Err(DecodeError::InvalidFixedLength {
                    expected: 33,
                    got: bytes.len(),
                });
            }
            let mut h = [0u8; 32];
            h.copy_from_slice(&bytes[1..33]);
            Ok(Some(h))
        }
        s => Err(DecodeError::InvalidUnionSelector(s)),
    }
}

fn encode_option_string(opt: &Option<String>, buf: &mut Vec<u8>) {
    match opt {
        None => buf.push(0),
        Some(s) => {
            buf.push(1);
            buf.extend_from_slice(s.as_bytes());
        }
    }
}

fn encoded_len_option_string(opt: &Option<String>) -> usize {
    match opt {
        None => 1,
        Some(s) => 1 + s.len(),
    }
}

fn decode_option_string(bytes: &[u8]) -> Result<Option<String>, DecodeError> {
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
            let payload = &bytes[1..];
            if payload.len() > MAX_ERROR_BYTES {
                return Err(DecodeError::InvalidByteLength {
                    expected: MAX_ERROR_BYTES,
                    got: payload.len(),
                });
            }
            let s = std::str::from_utf8(payload)
                .map_err(|_| DecodeError::InvalidByteLength {
                    expected: 0,
                    got: payload.len(),
                })?
                .to_string();
            Ok(Some(s))
        }
        s => Err(DecodeError::InvalidUnionSelector(s)),
    }
}

// ── PayloadStatus ────────────────────────────────────────────────────────────

/// SSZ payload status — `/payloads` response body.
///
/// Manual SSZ impl because libssz has no blanket `Option<T>` or `String` support.
/// Wire layout (SSZ container):
///   status              : u8          (fixed, 1 byte)
///   latest_valid_hash   : Offset(4)   (variable, union: 0x00 | 0x01 ++ [u8;32])
///   validation_error    : Offset(4)   (variable, union: 0x00 | 0x01 ++ utf8)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayloadStatus {
    pub status: u8,
    pub latest_valid_hash: Option<[u8; 32]>,
    pub validation_error: Option<String>,
}

impl SszEncode for PayloadStatus {
    fn is_fixed_size() -> bool {
        false
    }

    fn fixed_size() -> usize {
        0
    }

    fn encoded_len(&self) -> usize {
        // fixed part: 1 (status) + 4 (offset for latest_valid_hash) + 4 (offset for validation_error)
        // variable part: encoded lengths of both option fields
        1 + 4
            + 4
            + encoded_len_option_hash(&self.latest_valid_hash)
            + encoded_len_option_string(&self.validation_error)
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        // fixed_part_len = 1 (status) + 4 (offset lvh) + 4 (offset ve) = 9
        let fixed_part_len = 1 + 4 + 4;
        let mut enc = ContainerEncoder::with_capacity(buf, fixed_part_len, self.encoded_len());
        enc.append_fixed(&self.status);
        enc.append_variable(&OptionHashProxy(&self.latest_valid_hash));
        enc.append_variable(&OptionStringProxy(&self.validation_error));
        enc.finalize();
    }
}

impl SszDecode for PayloadStatus {
    fn is_fixed_size() -> bool {
        false
    }

    fn fixed_size() -> usize {
        0
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        // fixed_part_len = 1 (status) + 4 (offset lvh) + 4 (offset ve) = 9
        let fixed_part_len = 1 + 4 + 4;
        let mut dec = ContainerDecoder::new(bytes, fixed_part_len)?;
        let status: u8 = dec.decode_fixed()?;
        dec.read_variable_offset()?;
        dec.read_variable_offset()?;
        let latest_valid_hash = decode_option_hash(&dec.decode_variable::<Vec<u8>>()?)?;
        let validation_error = decode_option_string(&dec.decode_variable::<Vec<u8>>()?)?;
        Ok(PayloadStatus {
            status,
            latest_valid_hash,
            validation_error,
        })
    }
}

// ── Proxy types for ContainerEncoder::append_variable ────────────────────────

struct OptionHashProxy<'a>(&'a Option<[u8; 32]>);

impl SszEncode for OptionHashProxy<'_> {
    fn is_fixed_size() -> bool {
        false
    }
    fn fixed_size() -> usize {
        0
    }
    fn encoded_len(&self) -> usize {
        encoded_len_option_hash(self.0)
    }
    fn ssz_append(&self, buf: &mut Vec<u8>) {
        encode_option_hash(self.0, buf);
    }
}

struct OptionStringProxy<'a>(&'a Option<String>);

impl SszEncode for OptionStringProxy<'_> {
    fn is_fixed_size() -> bool {
        false
    }
    fn fixed_size() -> usize {
        0
    }
    fn encoded_len(&self) -> usize {
        encoded_len_option_string(self.0)
    }
    fn ssz_append(&self, buf: &mut Vec<u8>) {
        encode_option_string(self.0, buf);
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

/// `/forkchoice` response carrying the resulting status and (if attributes were supplied)
/// the payload-build id.
///
/// Manual SSZ impl because `Option<PayloadId>` requires union encoding.
/// Wire layout (SSZ container):
///   payload_status : Offset(4)  (variable — PayloadStatus is variable-length)
///   payload_id     : Offset(4)  (variable, union: 0x00 | 0x01 ++ [u8;8])
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForkchoiceResponse {
    pub payload_status: PayloadStatus,
    pub payload_id: Option<PayloadId>,
}

impl SszEncode for ForkchoiceResponse {
    fn is_fixed_size() -> bool {
        false
    }

    fn fixed_size() -> usize {
        0
    }

    fn encoded_len(&self) -> usize {
        // fixed part: 4 (offset ps) + 4 (offset pid) = 8
        4 + 4 + self.payload_status.encoded_len() + encoded_len_option_payload_id(&self.payload_id)
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        let fixed_part_len = 4 + 4;
        let mut enc = ContainerEncoder::with_capacity(buf, fixed_part_len, self.encoded_len());
        enc.append_variable(&self.payload_status);
        enc.append_variable(&OptionPayloadIdProxy(&self.payload_id));
        enc.finalize();
    }
}

impl SszDecode for ForkchoiceResponse {
    fn is_fixed_size() -> bool {
        false
    }

    fn fixed_size() -> usize {
        0
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let fixed_part_len = 4 + 4;
        let mut dec = ContainerDecoder::new(bytes, fixed_part_len)?;
        dec.read_variable_offset()?;
        dec.read_variable_offset()?;
        let payload_status = dec.decode_variable::<PayloadStatus>()?;
        let pid_bytes = dec.decode_variable::<Vec<u8>>()?;
        let payload_id = decode_option_payload_id(&pid_bytes)?;
        Ok(ForkchoiceResponse {
            payload_status,
            payload_id,
        })
    }
}

fn encode_option_payload_id(opt: &Option<PayloadId>, buf: &mut Vec<u8>) {
    match opt {
        None => buf.push(0),
        Some(id) => {
            buf.push(1);
            buf.extend_from_slice(&id.0);
        }
    }
}

fn encoded_len_option_payload_id(opt: &Option<PayloadId>) -> usize {
    match opt {
        None => 1,
        Some(_) => 1 + 8,
    }
}

fn decode_option_payload_id(bytes: &[u8]) -> Result<Option<PayloadId>, DecodeError> {
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
            if bytes.len() != 9 {
                return Err(DecodeError::InvalidFixedLength {
                    expected: 9,
                    got: bytes.len(),
                });
            }
            let mut id = [0u8; 8];
            id.copy_from_slice(&bytes[1..9]);
            Ok(Some(PayloadId(id)))
        }
        s => Err(DecodeError::InvalidUnionSelector(s)),
    }
}

struct OptionPayloadIdProxy<'a>(&'a Option<PayloadId>);

impl SszEncode for OptionPayloadIdProxy<'_> {
    fn is_fixed_size() -> bool {
        false
    }
    fn fixed_size() -> usize {
        0
    }
    fn encoded_len(&self) -> usize {
        encoded_len_option_payload_id(self.0)
    }
    fn ssz_append(&self, buf: &mut Vec<u8>) {
        encode_option_payload_id(self.0, buf);
    }
}
