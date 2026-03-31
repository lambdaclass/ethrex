//! SSZ containers for EIP-8025 (Execution Layer Triggerable Proofs).
//!
//! These types mirror the CL-side SSZ definitions used for tree-hashing
//! `NewPayloadRequest` and producing the `PublicInput` committed to by
//! execution proofs.

use libssz::{SszDecode, SszEncode};
use libssz_derive::{HashTreeRoot, SszDecode, SszEncode};
use libssz_merkle::{HashTreeRoot, Sha256Hasher};
use libssz_types::{SszList, SszVector};

// ── Spec limits (Electra) ──────────────────────────────────────────

/// MAX_TRANSACTIONS_PER_PAYLOAD (Electra).
const MAX_TRANSACTIONS: usize = 1_048_576;
/// MAX_WITHDRAWALS_PER_PAYLOAD (Electra).
const MAX_WITHDRAWALS: usize = 16;
/// MAX_BYTES_PER_TRANSACTION.
const MAX_BYTES_PER_TRANSACTION: usize = 1_073_741_824;
/// MAX_EXTRA_DATA_BYTES.
const MAX_EXTRA_DATA_BYTES: usize = 32;
/// MAX_DEPOSIT_REQUESTS_PER_PAYLOAD (Electra).
const MAX_DEPOSIT_REQUESTS: usize = 8192;
/// MAX_WITHDRAWAL_REQUESTS_PER_PAYLOAD (Electra).
const MAX_WITHDRAWAL_REQUESTS: usize = 16;
/// MAX_CONSOLIDATION_REQUESTS_PER_PAYLOAD (Electra).
const MAX_CONSOLIDATION_REQUESTS: usize = 1;
/// MAX_BLOB_COMMITMENTS_PER_BLOCK (Electra).
const MAX_BLOB_COMMITMENTS: usize = 4096;
/// MAX_EXECUTION_REQUESTS (EIP-7685).
const MAX_EXECUTION_REQUESTS: usize = 16;
/// MAX_EXECUTION_REQUEST_BYTES.
const MAX_EXECUTION_REQUEST_BYTES: usize = 1_073_741_824;

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

// ── LogsBloom type alias ───────────────────────────────────────────
//
// `logs_bloom` is `ByteVector[BYTES_PER_LOGS_BLOOM]` in the CL spec —
// a fixed-length SSZ vector of 256 bytes.

/// BYTES_PER_LOGS_BLOOM from the CL spec.
pub const BYTES_PER_LOGS_BLOOM: usize = 256;

/// `ByteVector[256]` — the logs bloom as a fixed-size SSZ vector.
pub type LogsBloom = SszVector<u8, BYTES_PER_LOGS_BLOOM>;

// ── Sub-containers ─────────────────────────────────────────────────

/// SSZ `Withdrawal` container matching the CL spec.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct Withdrawal {
    pub index: u64,
    pub validator_index: u64,
    pub address: Bytes20,
    pub amount: u64,
}

/// SSZ `DepositRequest` container (EIP-6110).
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct DepositRequest {
    pub pubkey: [u8; 48],
    pub withdrawal_credentials: [u8; 32],
    pub amount: u64,
    pub signature: [u8; 96],
    pub index: u64,
}

/// SSZ `WithdrawalRequest` container (EIP-7002).
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct WithdrawalRequest {
    pub source_address: Bytes20,
    pub validator_pubkey: [u8; 48],
    pub amount: u64,
}

/// SSZ `ConsolidationRequest` container (EIP-7251).
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct ConsolidationRequest {
    pub source_address: Bytes20,
    pub source_pubkey: [u8; 48],
    pub target_pubkey: [u8; 48],
}

// ── ExecutionPayload ───────────────────────────────────────────────

/// SSZ `ExecutionPayload` container matching `ExecutionPayloadElectra` from
/// the consensus spec.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct ExecutionPayload {
    pub parent_hash: [u8; 32],
    pub fee_recipient: Bytes20,
    pub state_root: [u8; 32],
    pub receipts_root: [u8; 32],
    pub logs_bloom: LogsBloom,
    pub prev_randao: [u8; 32],
    pub block_number: u64,
    pub gas_limit: u64,
    pub gas_used: u64,
    pub timestamp: u64,
    pub extra_data: SszList<u8, MAX_EXTRA_DATA_BYTES>,
    /// `base_fee_per_gas` encoded as a 256-bit unsigned integer (little-endian).
    pub base_fee_per_gas: [u8; 32],
    pub block_hash: [u8; 32],
    pub transactions: SszList<SszList<u8, MAX_BYTES_PER_TRANSACTION>, MAX_TRANSACTIONS>,
    pub withdrawals: SszList<Withdrawal, MAX_WITHDRAWALS>,
    pub blob_gas_used: u64,
    pub excess_blob_gas: u64,
    pub deposit_requests: SszList<DepositRequest, MAX_DEPOSIT_REQUESTS>,
    pub withdrawal_requests: SszList<WithdrawalRequest, MAX_WITHDRAWAL_REQUESTS>,
    pub consolidation_requests: SszList<ConsolidationRequest, MAX_CONSOLIDATION_REQUESTS>,
}

// ── ExecutionPayloadHeader ─────────────────────────────────────────

/// Headerized version of `ExecutionPayload`: variable-length lists are
/// replaced by their `hash_tree_root`.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct ExecutionPayloadHeader {
    pub parent_hash: [u8; 32],
    pub fee_recipient: Bytes20,
    pub state_root: [u8; 32],
    pub receipts_root: [u8; 32],
    pub logs_bloom: LogsBloom,
    pub prev_randao: [u8; 32],
    pub block_number: u64,
    pub gas_limit: u64,
    pub gas_used: u64,
    pub timestamp: u64,
    pub extra_data: SszList<u8, MAX_EXTRA_DATA_BYTES>,
    pub base_fee_per_gas: [u8; 32],
    pub block_hash: [u8; 32],
    pub transactions_root: [u8; 32],
    pub withdrawals_root: [u8; 32],
    pub blob_gas_used: u64,
    pub excess_blob_gas: u64,
    pub deposit_requests_root: [u8; 32],
    pub withdrawal_requests_root: [u8; 32],
    pub consolidation_requests_root: [u8; 32],
}

// ── NewPayloadRequest ──────────────────────────────────────────────

/// SSZ `NewPayloadRequest` — the key container whose `hash_tree_root` is
/// the public input committed to by an execution proof.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct NewPayloadRequest {
    pub execution_payload: ExecutionPayload,
    pub versioned_hashes: SszList<[u8; 32], MAX_BLOB_COMMITMENTS>,
    pub parent_beacon_block_root: [u8; 32],
    pub execution_requests:
        SszList<SszList<u8, MAX_EXECUTION_REQUEST_BYTES>, MAX_EXECUTION_REQUESTS>,
}

/// Headerized version of `NewPayloadRequest`.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct NewPayloadRequestHeader {
    pub execution_payload_header: ExecutionPayloadHeader,
    pub versioned_hashes: SszList<[u8; 32], MAX_BLOB_COMMITMENTS>,
    pub parent_beacon_block_root: [u8; 32],
    pub execution_requests:
        SszList<SszList<u8, MAX_EXECUTION_REQUEST_BYTES>, MAX_EXECUTION_REQUESTS>,
}

// ── PublicInput ────────────────────────────────────────────────────

/// The public input for an execution proof: the `hash_tree_root` of the
/// `NewPayloadRequest`.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct PublicInput {
    pub new_payload_request_root: [u8; 32],
}

// ── Conversion helpers ─────────────────────────────────────────────

impl ExecutionPayload {
    /// Produce the headerized version by computing `hash_tree_root` of
    /// each variable-length list field.
    pub fn to_header(&self, hasher: &impl Sha256Hasher) -> ExecutionPayloadHeader {
        ExecutionPayloadHeader {
            parent_hash: self.parent_hash,
            fee_recipient: self.fee_recipient,
            state_root: self.state_root,
            receipts_root: self.receipts_root,
            logs_bloom: self.logs_bloom.clone(),
            prev_randao: self.prev_randao,
            block_number: self.block_number,
            gas_limit: self.gas_limit,
            gas_used: self.gas_used,
            timestamp: self.timestamp,
            extra_data: self.extra_data.clone(),
            base_fee_per_gas: self.base_fee_per_gas,
            block_hash: self.block_hash,
            transactions_root: self.transactions.hash_tree_root(hasher),
            withdrawals_root: self.withdrawals.hash_tree_root(hasher),
            blob_gas_used: self.blob_gas_used,
            excess_blob_gas: self.excess_blob_gas,
            deposit_requests_root: self.deposit_requests.hash_tree_root(hasher),
            withdrawal_requests_root: self.withdrawal_requests.hash_tree_root(hasher),
            consolidation_requests_root: self.consolidation_requests.hash_tree_root(hasher),
        }
    }
}

impl NewPayloadRequest {
    /// Compute the `hash_tree_root` of this request — the value that
    /// becomes the execution proof's public input.
    pub fn public_input(&self, hasher: &impl Sha256Hasher) -> PublicInput {
        PublicInput {
            new_payload_request_root: self.hash_tree_root(hasher),
        }
    }

    /// Produce the headerized version.
    pub fn to_header(&self, hasher: &impl Sha256Hasher) -> NewPayloadRequestHeader {
        NewPayloadRequestHeader {
            execution_payload_header: self.execution_payload.to_header(hasher),
            versioned_hashes: self.versioned_hashes.clone(),
            parent_beacon_block_root: self.parent_beacon_block_root,
            execution_requests: self.execution_requests.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libssz_merkle::Sha2Hasher;

    const HASHER: Sha2Hasher = Sha2Hasher;

    fn sample_payload() -> ExecutionPayload {
        ExecutionPayload {
            parent_hash: [1u8; 32],
            fee_recipient: Bytes20([2u8; 20]),
            state_root: [3u8; 32],
            receipts_root: [4u8; 32],
            logs_bloom: vec![0u8; 256].try_into().unwrap(),
            prev_randao: [5u8; 32],
            block_number: 42,
            gas_limit: 30_000_000,
            gas_used: 21_000,
            timestamp: 1_700_000_000,
            extra_data: vec![0xAB, 0xCD].try_into().unwrap(),
            base_fee_per_gas: {
                let mut b = [0u8; 32];
                b[0] = 7; // 7 in LE
                b
            },
            block_hash: [6u8; 32],
            transactions: vec![vec![0xDE, 0xAD, 0xBE, 0xEF].try_into().unwrap()]
                .try_into()
                .unwrap(),
            withdrawals: vec![Withdrawal {
                index: 0,
                validator_index: 1,
                address: Bytes20([7u8; 20]),
                amount: 1_000_000,
            }]
            .try_into()
            .unwrap(),
            blob_gas_used: 0,
            excess_blob_gas: 0,
            deposit_requests: vec![].try_into().unwrap(),
            withdrawal_requests: vec![].try_into().unwrap(),
            consolidation_requests: vec![].try_into().unwrap(),
        }
    }

    fn sample_request() -> NewPayloadRequest {
        NewPayloadRequest {
            execution_payload: sample_payload(),
            versioned_hashes: vec![].try_into().unwrap(),
            parent_beacon_block_root: [8u8; 32],
            execution_requests: vec![].try_into().unwrap(),
        }
    }

    #[test]
    fn test_ssz_root_roundtrip_payload_vs_header() {
        let request = sample_request();
        let header = request.to_header(&HASHER);

        let request_root = request.hash_tree_root(&HASHER);
        let header_root = header.hash_tree_root(&HASHER);

        assert_eq!(
            request_root, header_root,
            "NewPayloadRequest root must equal NewPayloadRequestHeader root"
        );
    }

    #[test]
    fn test_ssz_root_changes_with_different_data() {
        let request1 = sample_request();
        let mut request2 = sample_request();
        request2.execution_payload.block_number = 99;

        assert_ne!(
            request1.hash_tree_root(&HASHER),
            request2.hash_tree_root(&HASHER),
            "Different payloads must produce different roots"
        );
    }

    #[test]
    fn test_empty_vs_nonempty_list_roots_differ() {
        let payload = sample_payload();
        let header = payload.to_header(&HASHER);

        let empty_payload = ExecutionPayload {
            transactions: vec![].try_into().unwrap(),
            withdrawals: vec![].try_into().unwrap(),
            deposit_requests: vec![].try_into().unwrap(),
            withdrawal_requests: vec![].try_into().unwrap(),
            consolidation_requests: vec![].try_into().unwrap(),
            ..sample_payload()
        };
        let empty_header = empty_payload.to_header(&HASHER);

        // Non-empty lists (sample has 1 tx + 1 withdrawal) produce different roots
        // than empty lists.
        assert_ne!(
            header.transactions_root, empty_header.transactions_root,
            "Non-empty transactions root must differ from empty"
        );
        assert_ne!(
            header.withdrawals_root, empty_header.withdrawals_root,
            "Non-empty withdrawals root must differ from empty"
        );

        // deposit/withdrawal/consolidation requests are empty in both, so roots match.
        assert_eq!(
            header.deposit_requests_root, empty_header.deposit_requests_root,
            "Both-empty deposit_requests roots must match"
        );
    }

    #[test]
    fn test_ssz_root_is_deterministic() {
        let request = sample_request();
        let root1 = request.hash_tree_root(&HASHER);
        let root2 = request.hash_tree_root(&HASHER);
        assert_eq!(root1, root2, "Same request must produce same root");
    }
}
