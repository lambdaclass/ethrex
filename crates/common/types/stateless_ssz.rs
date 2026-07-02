//! SSZ containers for EIP-8025 (Optional Execution Proofs) and the
//! stateless validation flow used by native rollups.
//!
//! The first section mirrors the CL-side SSZ definitions used for
//! tree-hashing `NewPayloadRequest` and producing the `PublicInput`
//! committed to by execution proofs. The second section layers the
//! native-rollup types (`SszStatelessInput`, `SszStatelessValidationResult`,
//! `SszExecutionWitness`, `SszChainConfig`) on top of those.

use bytes::Bytes;
use libssz::{SszDecode, SszEncode};
use libssz_derive::{HashTreeRoot, SszDecode, SszEncode};
use libssz_merkle::{HashTreeRoot, Sha256Hasher};
use libssz_types::{SszList, SszVector};

use super::requests::EncodedRequests;

// ============================================================================
// EIP-8025 containers
// ============================================================================

// ── Spec limits (Electra) ──────────────────────────────────────────

/// `MAX_TRANSACTIONS_PER_PAYLOAD` (Electra).
const MAX_TRANSACTIONS_PER_PAYLOAD: usize = 1_048_576;
/// `MAX_WITHDRAWALS_PER_PAYLOAD` (Electra).
const MAX_WITHDRAWALS_PER_PAYLOAD: usize = 16;
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

/// SSZ container matching the `85fc20ca` `SszExecutionPayload` (Electra fields + EIP-7928 `block_access_list`; 18 fields).
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
    /// EIP-7928 block-level access list (full serialized BAL bytes).
    pub block_access_list: SszList<u8, MAX_BLOCK_ACCESS_LIST_BYTES>,
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

// ============================================================================
// Stateless validation containers (native rollups / EXECUTE precompile)
// ============================================================================

// ── Stateless validation limits ──────────────────────────────────

/// MAX_WITNESS_NODES — max trie-node preimages in an execution witness.
const MAX_WITNESS_NODES: usize = 1_048_576; // 2^20
/// MAX_BYTES_PER_WITNESS_NODE — max size of a single witness node.
const MAX_BYTES_PER_WITNESS_NODE: usize = 1_048_576; // 2^20
/// MAX_WITNESS_CODES — max contract code preimages in an execution witness.
const MAX_WITNESS_CODES: usize = 65_536; // 2^16
/// MAX_BYTES_PER_CODE — max size of a single code preimage (EIP-7954).
const MAX_BYTES_PER_CODE: usize = 16_777_216; // 2^24
/// MAX_WITNESS_HEADERS — max RLP-encoded block headers in witness (up to 256).
const MAX_WITNESS_HEADERS: usize = 256;
/// MAX_BYTES_PER_HEADER — max size of a single RLP-encoded header.
const MAX_BYTES_PER_HEADER: usize = 1_024; // 2^10
/// MAX_PUBLIC_KEYS — max recovered transaction public keys.
const MAX_PUBLIC_KEYS: usize = 1_048_576; // 2^20
/// PUBLIC_KEY_BYTES — an uncompressed secp256k1 public key is 65 bytes.
const PUBLIC_KEY_BYTES: usize = 65;
/// MAX_BLOCK_ACCESS_LIST_BYTES — EIP-7928 BAL byte cap.
const MAX_BLOCK_ACCESS_LIST_BYTES: usize = 16_777_216; // 2^24
/// MAX_BLOB_SCHEDULES_PER_FORK — SSZ Optional[BlobSchedule] as List[T, 1].
const MAX_BLOB_SCHEDULES_PER_FORK: usize = 1;
/// MAX_FORK_ACTIVATION_VALUES — SSZ Optional[uint64] as List[uint64, 1].
const MAX_FORK_ACTIVATION_VALUES: usize = 1;

// ── Stateless validation types ───────────────────────────────────
//
// Mirror the definitions in execution-specs (projects/zkevm branch) at the
// commit EIP-8025 PR #11604 pins:
// https://github.com/ethereum/execution-specs/blob/85fc20ca5937719a854472a87cb48d01ef1dffca/src/ethereum/forks/amsterdam/stateless_ssz.py

/// SSZ Optional[uint64] modelled as `List[uint64, 1]`.
pub type SszOptionalForkActivationValue = SszList<u64, MAX_FORK_ACTIVATION_VALUES>;
/// SSZ Optional[BlobSchedule] modelled as `List[SszBlobSchedule, 1]`.
pub type SszOptionalBlobSchedule = SszList<SszBlobSchedule, MAX_BLOB_SCHEDULES_PER_FORK>;

/// SSZ `BlobSchedule` — effective blob params for a fork.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct SszBlobSchedule {
    pub target: u64,
    pub max: u64,
    pub base_fee_update_fraction: u64,
}

/// SSZ `ForkActivation` — the (optional) block/timestamp a fork activates at.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct SszForkActivation {
    pub block_number: SszOptionalForkActivationValue,
    pub timestamp: SszOptionalForkActivationValue,
}

/// SSZ `ForkConfig` — the active fork id, its activation, and blob schedule.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct SszForkConfig {
    pub fork: u64,
    pub activation: SszForkActivation,
    pub blob_schedule: SszOptionalBlobSchedule,
}

/// SSZ `ChainConfig` container. Variable-size (nests `active_fork`'s lists).
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct SszChainConfig {
    pub chain_id: u64,
    pub active_fork: SszForkConfig,
}

/// SSZ `ExecutionWitness` container matching the execution-specs definition.
///
/// Contains all data needed for stateless execution:
/// - `state`: trie-node preimages
/// - `codes`: contract code preimages
/// - `headers`: RLP-encoded parent block headers (up to 256)
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct SszExecutionWitness {
    pub state: SszList<SszList<u8, MAX_BYTES_PER_WITNESS_NODE>, MAX_WITNESS_NODES>,
    pub codes: SszList<SszList<u8, MAX_BYTES_PER_CODE>, MAX_WITNESS_CODES>,
    pub headers: SszList<SszList<u8, MAX_BYTES_PER_HEADER>, MAX_WITNESS_HEADERS>,
}

/// SSZ `StatelessInput` — the top-level input to `verify_stateless_new_payload`.
///
/// Wraps a `NewPayloadRequest` together with the execution witness,
/// chain configuration, and (optionally) pre-recovered public keys.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct SszStatelessInput {
    pub new_payload_request: NewPayloadRequest,
    pub witness: SszExecutionWitness,
    pub chain_config: SszChainConfig,
    pub public_keys: SszList<SszVector<u8, PUBLIC_KEY_BYTES>, MAX_PUBLIC_KEYS>,
}

/// SSZ `StatelessValidationResult` — the output of `verify_stateless_new_payload`.
#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct SszStatelessValidationResult {
    pub new_payload_request_root: [u8; 32],
    pub successful_validation: bool,
    pub chain_config: SszChainConfig,
}

// ── Conversions to internal types ────────────────────────────────

impl SszExecutionWitness {
    /// Extract raw bytes from SSZ lists for codes.
    pub fn codes_as_vecs(&self) -> Vec<Vec<u8>> {
        self.codes
            .iter()
            .map(|c| c.iter().copied().collect())
            .collect()
    }

    /// Extract raw bytes from SSZ lists for headers.
    pub fn headers_as_vecs(&self) -> Vec<Vec<u8>> {
        self.headers
            .iter()
            .map(|h| h.iter().copied().collect())
            .collect()
    }

    /// Extract raw bytes from SSZ lists for state nodes.
    pub fn state_as_vecs(&self) -> Vec<Vec<u8>> {
        self.state
            .iter()
            .map(|n| n.iter().copied().collect())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libssz_merkle::Sha2Hasher;

    const HASHER: Sha2Hasher = Sha2Hasher;

    // ── EIP-8025 helpers ─────────────────────────────────────────

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
            block_access_list: SszList::new(), // TODO(Plan 02): populate full BAL
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

    // ── Stateless helpers ────────────────────────────────────────

    fn sample_active_fork() -> SszForkConfig {
        SszForkConfig {
            fork: 25, // Amsterdam (spec ProtocolFork index)
            activation: SszForkActivation {
                block_number: SszList::new(),
                timestamp: {
                    let mut v = SszList::new();
                    v.push(0u64).expect("one value fits");
                    v
                },
            },
            blob_schedule: SszList::new(),
        }
    }

    #[test]
    fn ssz_chain_config_with_active_fork_round_trip() {
        round_trip(&SszChainConfig {
            chain_id: 1,
            active_fork: sample_active_fork(),
        });
        round_trip(&SszForkActivation {
            block_number: SszList::new(),
            timestamp: SszList::new(),
        });
        round_trip(&SszBlobSchedule {
            target: 3,
            max: 6,
            base_fee_update_fraction: 3_338_477,
        });
    }

    fn list<T: SszEncode + SszDecode, const N: usize>(items: Vec<T>) -> SszList<T, N> {
        let mut list = SszList::new();
        for item in items {
            list.push(item).expect("test list capacity exceeded");
        }
        list
    }

    fn round_trip<T: SszEncode + SszDecode + PartialEq + std::fmt::Debug>(value: &T) {
        let mut buf = Vec::new();
        value.ssz_append(&mut buf);
        let decoded = T::from_ssz_bytes(&buf).expect("SSZ decode failed");
        assert_eq!(*value, decoded, "round-trip mismatch");
    }

    #[test]
    fn ssz_chain_config_round_trip() {
        round_trip(&SszChainConfig {
            chain_id: 1,
            active_fork: sample_active_fork(),
        });
        round_trip(&SszChainConfig {
            chain_id: 0,
            active_fork: sample_active_fork(),
        });
        round_trip(&SszChainConfig {
            chain_id: u64::MAX,
            active_fork: sample_active_fork(),
        });
    }

    #[test]
    fn ssz_execution_witness_round_trip() {
        let witness = SszExecutionWitness {
            state: list(vec![list(vec![1u8, 2, 3]), list(vec![4u8, 5])]),
            codes: list(vec![list(vec![0x60u8, 0x00, 0x60, 0x00, 0xf3])]),
            headers: list(vec![list(vec![0xf9u8, 0x02, 0x11])]),
        };
        round_trip(&witness);
    }

    #[test]
    fn ssz_execution_witness_empty_round_trip() {
        let witness = SszExecutionWitness {
            state: SszList::new(),
            codes: SszList::new(),
            headers: SszList::new(),
        };
        round_trip(&witness);
    }

    #[test]
    fn ssz_execution_payload_has_block_access_list_round_trip() {
        let payload = ExecutionPayload {
            parent_hash: [0x11; 32],
            fee_recipient: Bytes20([0x22; 20]),
            state_root: [0x33; 32],
            receipts_root: [0x44; 32],
            logs_bloom: vec![0u8; 256].try_into().expect("logs_bloom length"),
            prev_randao: [0x55; 32],
            block_number: 7,
            gas_limit: 30_000_000,
            gas_used: 21_000,
            timestamp: 1_700_000_000,
            extra_data: list(vec![0xde, 0xad]),
            base_fee_per_gas: [0u8; 32],
            block_hash: [0x66; 32],
            transactions: SszList::new(),
            withdrawals: SszList::new(),
            blob_gas_used: 0,
            excess_blob_gas: 0,
            block_access_list: list(vec![0x01, 0x02, 0x03]),
        };
        round_trip(&payload);
        assert_eq!(payload.block_access_list.len(), 3);
    }

    #[test]
    fn ssz_public_keys_are_65_byte_vectors_round_trip() {
        let key: SszVector<u8, PUBLIC_KEY_BYTES> =
            vec![0x04u8; 65].try_into().expect("pubkey length");
        let mut keys: SszList<SszVector<u8, PUBLIC_KEY_BYTES>, MAX_PUBLIC_KEYS> = SszList::new();
        keys.push(key).expect("one key fits");
        round_trip(&keys);
        assert_eq!(keys.first().unwrap().len(), 65);
    }

    #[test]
    fn ssz_stateless_validation_result_round_trip() {
        let result = SszStatelessValidationResult {
            new_payload_request_root: [0xab; 32],
            successful_validation: true,
            chain_config: SszChainConfig {
                chain_id: 42,
                active_fork: sample_active_fork(),
            },
        };
        round_trip(&result);

        let result_false = SszStatelessValidationResult {
            new_payload_request_root: [0x00; 32],
            successful_validation: false,
            chain_config: SszChainConfig {
                chain_id: 1,
                active_fork: sample_active_fork(),
            },
        };
        round_trip(&result_false);
    }

    // ── NativeRollup.sol SSZ offset cross-checks (I17) ───────────────
    //
    // These constants MUST equal NativeRollup.sol's SSZ offset constants
    // (crates/l2/contracts/src/nativeRollup/l1/NativeRollup.sol). If a container
    // field is reordered/resized, this test fails instead of advance() silently
    // reading the wrong bytes on L1.

    const SOL_RESULT_SUCCESS_OFFSET: usize = 32;
    // result bytes 33..36 hold the u32 LE OFFSET to chain_config's variable data (not chain_id itself).
    const SOL_RESULT_CHAIN_CONFIG_OFFSET_POS: usize = 33;
    const SOL_EP_STATE_ROOT_OFFSET: usize = 52;
    const SOL_EP_BLOCK_NUMBER_OFFSET: usize = 404;
    const SOL_EP_GAS_LIMIT_OFFSET: usize = 412;
    const SOL_EP_BLOCK_HASH_OFFSET: usize = 472;
    const SOL_EP_FIXED_PREFIX_LEN: usize = 532;

    fn u32_le(bytes: &[u8], off: usize) -> usize {
        (bytes[off] as usize)
            | ((bytes[off + 1] as usize) << 8)
            | ((bytes[off + 2] as usize) << 16)
            | ((bytes[off + 3] as usize) << 24)
    }

    fn sample_execution_payload() -> ExecutionPayload {
        ExecutionPayload {
            parent_hash: [0x11; 32],
            fee_recipient: Bytes20([0x22; 20]),
            state_root: [0x33; 32],
            receipts_root: [0x44; 32],
            logs_bloom: vec![0u8; 256].try_into().expect("bloom"),
            prev_randao: [0x55; 32],
            block_number: 7,
            gas_limit: 30_000_000,
            gas_used: 21_000,
            timestamp: 1_700_000_000,
            extra_data: SszList::new(),
            base_fee_per_gas: [0u8; 32],
            block_hash: [0x66; 32],
            transactions: SszList::new(),
            withdrawals: SszList::new(),
            blob_gas_used: 0,
            excess_blob_gas: 0,
            block_access_list: SszList::new(),
        }
    }

    #[test]
    fn nativerollup_sol_result_layout_matches() {
        // Encode a StatelessValidationResult and confirm the contract's fixed
        // offsets: successful_validation @32, chain_config offset @33, and chain_id
        // (first field of chain_config) at the dereferenced offset.
        let result = SszStatelessValidationResult {
            new_payload_request_root: [0xAA; 32],
            successful_validation: true,
            chain_config: SszChainConfig {
                chain_id: 0x1122334455667788,
                active_fork: sample_active_fork(),
            },
        };
        let mut buf = Vec::new();
        result.ssz_append(&mut buf);

        assert_eq!(
            buf[SOL_RESULT_SUCCESS_OFFSET], 1,
            "successful_validation must be byte 32"
        );
        let cc_off = u32_le(&buf, SOL_RESULT_CHAIN_CONFIG_OFFSET_POS);
        assert_eq!(
            cc_off, 37,
            "chain_config offset value must be 37 (fixed part length), actual: {}",
            cc_off
        );
        // chain_id is the first field of SszChainConfig, uint64 LE, at cc_off.
        let chain_id = u64::from_le_bytes(buf[cc_off..cc_off + 8].try_into().unwrap());
        assert_eq!(
            chain_id, 0x1122334455667788,
            "chain_id must be readable at the deref offset"
        );
    }

    #[test]
    fn nativerollup_sol_ep_offsets_match() {
        // Encode a StatelessInput and confirm the ExecutionPayload fixed-field
        // offsets the contract reads (relative to the EP absolute offset).
        let ep = sample_execution_payload();
        let npr = NewPayloadRequest {
            execution_payload: ep,
            versioned_hashes: SszList::new(),
            parent_beacon_block_root: [0x00; 32],
            execution_requests: ExecutionRequests {
                deposits: SszList::new(),
                withdrawals: SszList::new(),
                consolidations: SszList::new(),
            },
        };
        let input = SszStatelessInput {
            new_payload_request: npr,
            witness: SszExecutionWitness {
                state: SszList::new(),
                codes: SszList::new(),
                headers: SszList::new(),
            },
            chain_config: SszChainConfig {
                chain_id: 1,
                active_fork: sample_active_fork(),
            },
            public_keys: SszList::new(),
        };
        let mut buf = Vec::new();
        input.ssz_append(&mut buf);

        // StatelessInput fixed part = 4 offsets (16 bytes); new_payload_request is field 0.
        let npr_abs = u32_le(&buf, 0);
        // NewPayloadRequest fixed prefix: execution_payload offset @ npr_abs.
        let ep_abs = npr_abs + u32_le(&buf, npr_abs);
        // The EP fixed FIELD offsets the contract reads must land where expected:
        let actual_block_number = u64::from_le_bytes(
            buf[ep_abs + SOL_EP_BLOCK_NUMBER_OFFSET..ep_abs + SOL_EP_BLOCK_NUMBER_OFFSET + 8]
                .try_into()
                .unwrap(),
        );
        assert_eq!(
            actual_block_number, 7,
            "block_number must be at EP offset 404, actual offset produces: {}",
            actual_block_number
        );
        let actual_gas_limit = u64::from_le_bytes(
            buf[ep_abs + SOL_EP_GAS_LIMIT_OFFSET..ep_abs + SOL_EP_GAS_LIMIT_OFFSET + 8]
                .try_into()
                .unwrap(),
        );
        assert_eq!(
            actual_gas_limit, 30_000_000,
            "gas_limit must be at EP offset 412, actual offset produces: {}",
            actual_gas_limit
        );
        assert_eq!(
            &buf[ep_abs + SOL_EP_STATE_ROOT_OFFSET..ep_abs + SOL_EP_STATE_ROOT_OFFSET + 32],
            &[0x33; 32],
            "state_root @52"
        );
        assert_eq!(
            &buf[ep_abs + SOL_EP_BLOCK_HASH_OFFSET..ep_abs + SOL_EP_BLOCK_HASH_OFFSET + 32],
            &[0x66; 32],
            "block_hash @472"
        );
        // block_access_list's offset slot is the last 4 bytes of the EP fixed prefix
        // (at EP+528). SSZ offsets are container-relative, so with all variable
        // fields empty it must equal the fixed-prefix length (532) — this pins the
        // prefix length that NativeRollup.sol's EP_FIXED_PREFIX_LEN depends on.
        assert_eq!(
            u32_le(&buf, ep_abs + SOL_EP_FIXED_PREFIX_LEN - 4),
            SOL_EP_FIXED_PREFIX_LEN,
            "block_access_list offset slot @EP+528 must be 532 (EP fixed-prefix length)",
        );
    }
}
