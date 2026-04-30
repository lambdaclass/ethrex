//! SSZ containers for EIP-8025 (Execution Layer Triggerable Proofs).
//!
//! These types mirror the CL-side SSZ definitions used for tree-hashing
//! `NewPayloadRequest` and producing the `PublicInput` committed to by
//! execution proofs.

use bytes::Bytes;
use libssz::{SszDecode, SszEncode};
use libssz_derive::{HashTreeRoot, SszDecode, SszEncode};
use libssz_merkle::{HashTreeRoot, Sha256Hasher};
use libssz_types::{SszList, SszVector};

use super::requests::EncodedRequests;

// ── Spec limits (Electra) ──────────────────────────────────────────

/// `MAX_TRANSACTIONS_PER_PAYLOAD` (Electra).
const MAX_TRANSACTIONS_PER_PAYLOAD: usize = 1_048_576;
/// `MAX_WITHDRAWALS_PER_PAYLOAD` (Electra).
/// TODO: the specs have a non-compliant value compared to consensus
/// specs. Whenever the specs can resolve an underlying issue,
/// this value should be updated.
/// See https://github.com/ethereum/execution-specs/blob/ec23140720d6a9257a907c470ba1874623bd7b50/src/ethereum/forks/amsterdam/stateless_ssz.py#L40-L43
const MAX_WITHDRAWALS_PER_PAYLOAD: usize = 65536;
/// `MAX_BYTES_PER_TRANSACTION`.
const MAX_BYTES_PER_TRANSACTION: usize = 1_073_741_824;
/// `MAX_EXTRA_DATA_BYTES`.
const MAX_EXTRA_DATA_BYTES: usize = 32;
/// `MAX_DEPOSIT_REQUESTS_PER_PAYLOAD` (Electra).
const MAX_DEPOSIT_REQUESTS_PER_PAYLOAD: usize = 8192;
/// `MAX_WITHDRAWAL_REQUESTS_PER_PAYLOAD` (Electra).
const MAX_WITHDRAWAL_REQUESTS_PER_PAYLOAD: usize = 16;
/// `MAX_CONSOLIDATION_REQUESTS_PER_PAYLOAD` (Electra).
const MAX_CONSOLIDATION_REQUESTS_PER_PAYLOAD: usize = 2;
/// `MAX_BLOB_COMMITMENTS_PER_BLOCK` (Electra).
const MAX_BLOB_COMMITMENTS_PER_BLOCK: usize = 4096;
/// `MAX_BLOCK_ACCESS_LIST_BYTES` (Amsterdam).
const MAX_BLOCK_ACCESS_LIST_BYTES: usize = 16777216;

// ── EIP-7685 request type prefixes ─────────────────────────────────

const DEPOSIT_REQUEST_TYPE: u8 = 0x00;
const WITHDRAWAL_REQUEST_TYPE: u8 = 0x01;
const CONSOLIDATION_REQUEST_TYPE: u8 = 0x02;

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

/// `BYTES_PER_LOGS_BLOOM` from the CL spec.
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
    pub transactions: SszList<SszList<u8, MAX_BYTES_PER_TRANSACTION>, MAX_TRANSACTIONS_PER_PAYLOAD>,
    pub withdrawals: SszList<Withdrawal, MAX_WITHDRAWALS_PER_PAYLOAD>,
    pub blob_gas_used: u64,
    pub excess_blob_gas: u64,
}

/// SSZ `ExecutionPayload` execution payload V4.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct ExecutionPayloadV4 {
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
    pub transactions: SszList<SszList<u8, MAX_BYTES_PER_TRANSACTION>, MAX_TRANSACTIONS_PER_PAYLOAD>,
    pub withdrawals: SszList<Withdrawal, MAX_WITHDRAWALS_PER_PAYLOAD>,
    pub blob_gas_used: u64,
    pub excess_blob_gas: u64,
    pub block_access_list: SszList<u8, MAX_BLOCK_ACCESS_LIST_BYTES>,
    pub slot_number: u64,
}

// ── ExecutionRequests ──────────────────────────────────────────────

/// SSZ `ExecutionRequests` container (Electra) — the typed EIP-7685 bundle
/// that the CL commits to alongside `ExecutionPayload`.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct ExecutionRequests {
    pub deposits: SszList<DepositRequest, MAX_DEPOSIT_REQUESTS_PER_PAYLOAD>,
    pub withdrawals: SszList<WithdrawalRequest, MAX_WITHDRAWAL_REQUESTS_PER_PAYLOAD>,
    pub consolidations: SszList<ConsolidationRequest, MAX_CONSOLIDATION_REQUESTS_PER_PAYLOAD>,
}

impl ExecutionRequests {
    /// Produce the EIP-7685 encoded form: three `EncodedRequests` entries,
    /// one per request type, each `[type_byte] ++ concat(ssz_encode(item))`.
    ///
    /// The three request types are all fixed-size SSZ containers, so their
    /// SSZ encoding is byte-for-byte the EL wire concatenation that
    /// `compute_requests_hash` expects.
    pub fn to_encoded_requests(&self) -> Vec<EncodedRequests> {
        fn encode<T: SszEncode>(
            type_byte: u8,
            items: impl IntoIterator<Item = T>,
        ) -> EncodedRequests {
            let mut buf = Vec::new();
            buf.push(type_byte);
            for item in items {
                item.ssz_append(&mut buf);
            }
            EncodedRequests(Bytes::from(buf))
        }

        vec![
            encode(DEPOSIT_REQUEST_TYPE, self.deposits.iter().cloned()),
            encode(WITHDRAWAL_REQUEST_TYPE, self.withdrawals.iter().cloned()),
            encode(
                CONSOLIDATION_REQUEST_TYPE,
                self.consolidations.iter().cloned(),
            ),
        ]
    }
}

// ── NewPayloadRequest ──────────────────────────────────────────────

/// SSZ `NewPayloadRequest` — the key container whose `hash_tree_root` is
/// the public input committed to by an execution proof.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct NewPayloadRequest {
    pub execution_payload: ExecutionPayload,
    pub versioned_hashes: SszList<[u8; 32], MAX_BLOB_COMMITMENTS_PER_BLOCK>,
    pub parent_beacon_block_root: [u8; 32],
    pub execution_requests: ExecutionRequests,
}

/// SSZ `NewPayloadRequest` for the Amsterdam fork.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct NewPayloadRequestAmsterdam {
    pub execution_payload: ExecutionPayloadV4,
    pub versioned_hashes: SszList<[u8; 32], MAX_BLOB_COMMITMENTS_PER_BLOCK>,
    pub parent_beacon_block_root: [u8; 32],
    pub execution_requests: ExecutionRequests,
}

// ── PublicInput ────────────────────────────────────────────────────

/// The public input for an execution proof: the `hash_tree_root` of the
/// `NewPayloadRequest`.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct PublicInput {
    pub new_payload_request_root: [u8; 32],
}

impl NewPayloadRequest {
    /// Compute the `hash_tree_root` of this request — the value that
    /// becomes the execution proof's public input.
    pub fn public_input(&self, hasher: &impl Sha256Hasher) -> PublicInput {
        PublicInput {
            new_payload_request_root: self.hash_tree_root(hasher),
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
            logs_bloom: vec![0u8; 256].try_into().expect("logs_bloom length"),
            prev_randao: [5u8; 32],
            block_number: 42,
            gas_limit: 30_000_000,
            gas_used: 21_000,
            timestamp: 1_700_000_000,
            extra_data: vec![0xAB, 0xCD].try_into().expect("extra_data fits"),
            base_fee_per_gas: {
                let mut b = [0u8; 32];
                b[0] = 7; // 7 in LE
                b
            },
            block_hash: [6u8; 32],
            transactions: vec![
                vec![0xDE, 0xAD, 0xBE, 0xEF]
                    .try_into()
                    .expect("tx bytes fit"),
            ]
            .try_into()
            .expect("txs fit"),
            withdrawals: vec![Withdrawal {
                index: 0,
                validator_index: 1,
                address: Bytes20([7u8; 20]),
                amount: 1_000_000,
            }]
            .try_into()
            .expect("withdrawals fit"),
            blob_gas_used: 0,
            excess_blob_gas: 0,
        }
    }

    fn empty_requests() -> ExecutionRequests {
        ExecutionRequests {
            deposits: vec![].try_into().expect("empty deposits"),
            withdrawals: vec![].try_into().expect("empty withdrawals"),
            consolidations: vec![].try_into().expect("empty consolidations"),
        }
    }

    fn sample_request() -> NewPayloadRequest {
        NewPayloadRequest {
            execution_payload: sample_payload(),
            versioned_hashes: vec![].try_into().expect("empty versioned_hashes"),
            parent_beacon_block_root: [8u8; 32],
            execution_requests: empty_requests(),
        }
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
    fn test_ssz_root_is_deterministic() {
        let request = sample_request();
        let root1 = request.hash_tree_root(&HASHER);
        let root2 = request.hash_tree_root(&HASHER);
        assert_eq!(root1, root2, "Same request must produce same root");
    }

    #[test]
    fn test_execution_requests_to_encoded_bytes() {
        let requests = ExecutionRequests {
            deposits: vec![DepositRequest {
                pubkey: [0x11; 48],
                withdrawal_credentials: [0x22; 32],
                amount: 32_000_000_000,
                signature: [0x33; 96],
                index: 7,
            }]
            .try_into()
            .expect("one deposit fits"),
            withdrawals: vec![WithdrawalRequest {
                source_address: Bytes20([0x44; 20]),
                validator_pubkey: [0x55; 48],
                amount: 1_000_000,
            }]
            .try_into()
            .expect("one withdrawal fits"),
            consolidations: vec![ConsolidationRequest {
                source_address: Bytes20([0x66; 20]),
                source_pubkey: [0x77; 48],
                target_pubkey: [0x88; 48],
            }]
            .try_into()
            .expect("one consolidation fits"),
        };

        let encoded = requests.to_encoded_requests();
        assert_eq!(encoded.len(), 3, "must emit 3 EIP-7685 entries");

        // Deposit: [0x00] ++ 192 bytes
        assert_eq!(encoded[0].0[0], DEPOSIT_REQUEST_TYPE);
        assert_eq!(encoded[0].0.len(), 1 + 192);

        // Withdrawal: [0x01] ++ 76 bytes
        assert_eq!(encoded[1].0[0], WITHDRAWAL_REQUEST_TYPE);
        assert_eq!(encoded[1].0.len(), 1 + 76);

        // Consolidation: [0x02] ++ 116 bytes
        assert_eq!(encoded[2].0[0], CONSOLIDATION_REQUEST_TYPE);
        assert_eq!(encoded[2].0.len(), 1 + 116);
    }
}
