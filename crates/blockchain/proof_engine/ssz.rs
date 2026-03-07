//! SSZ containers for EIP-8025 hash_tree_root computation.
//!
//! These types mirror the CL SSZ definitions for `NewPayloadRequest`,
//! `NewPayloadRequestHeader`, `ExecutionPayloadHeader`, and `PublicInput`.
//! They exist solely for Merkleization (hash_tree_root) and are NOT used
//! for JSON-RPC serialization — see `types.rs` for the Engine API types.

use ssz_rs::prelude::*;

// Max list lengths from the consensus spec (Electra/Fulu).
// These are upper bounds for SSZ List types.
const MAX_TRANSACTIONS_PER_PAYLOAD: usize = 1_048_576;
const MAX_WITHDRAWALS_PER_PAYLOAD: usize = 16;
const MAX_BYTES_PER_TRANSACTION: usize = 1_073_741_824; // 2^30
const MAX_EXTRA_DATA_BYTES: usize = 32;
const MAX_BLOB_COMMITMENTS_PER_BLOCK: usize = 4096;
const MAX_REQUEST_DATA_BYTES: usize = 8_388_608; // 2^23
const MAX_EXECUTION_REQUESTS: usize = 16;
const BYTES_PER_LOGS_BLOOM: usize = 256;

/// SSZ `ExecutionPayload` container (Electra/Fulu fork).
///
/// This matches the CL `ExecutionPayload` container used inside
/// `NewPayloadRequest`. Transactions and withdrawals are full lists.
#[derive(Debug, Default, Clone, PartialEq, Eq, SimpleSerialize)]
pub struct SszExecutionPayload {
    pub parent_hash: [u8; 32],
    pub fee_recipient: [u8; 20],
    pub state_root: [u8; 32],
    pub receipts_root: [u8; 32],
    pub logs_bloom: Vector<u8, BYTES_PER_LOGS_BLOOM>,
    pub prev_randao: [u8; 32],
    pub block_number: u64,
    pub gas_limit: u64,
    pub gas_used: u64,
    pub timestamp: u64,
    pub extra_data: List<u8, MAX_EXTRA_DATA_BYTES>,
    pub base_fee_per_gas: U256,
    pub block_hash: [u8; 32],
    pub transactions: List<List<u8, MAX_BYTES_PER_TRANSACTION>, MAX_TRANSACTIONS_PER_PAYLOAD>,
    pub withdrawals: List<SszWithdrawal, MAX_WITHDRAWALS_PER_PAYLOAD>,
    pub blob_gas_used: u64,
    pub excess_blob_gas: u64,
}

/// SSZ `Withdrawal` container.
#[derive(Debug, Default, Clone, PartialEq, Eq, SimpleSerialize)]
pub struct SszWithdrawal {
    pub index: u64,
    pub validator_index: u64,
    pub address: [u8; 20],
    pub amount: u64,
}

/// SSZ `ExecutionPayloadHeader` container (Electra/Fulu fork).
///
/// Same as `ExecutionPayload` but with `transactions_root` and
/// `withdrawals_root` instead of the full lists.
#[derive(Debug, Default, Clone, PartialEq, Eq, SimpleSerialize)]
pub struct SszExecutionPayloadHeader {
    pub parent_hash: [u8; 32],
    pub fee_recipient: [u8; 20],
    pub state_root: [u8; 32],
    pub receipts_root: [u8; 32],
    pub logs_bloom: Vector<u8, BYTES_PER_LOGS_BLOOM>,
    pub prev_randao: [u8; 32],
    pub block_number: u64,
    pub gas_limit: u64,
    pub gas_used: u64,
    pub timestamp: u64,
    pub extra_data: List<u8, MAX_EXTRA_DATA_BYTES>,
    pub base_fee_per_gas: U256,
    pub block_hash: [u8; 32],
    pub transactions_root: [u8; 32],
    pub withdrawals_root: [u8; 32],
    pub blob_gas_used: u64,
    pub excess_blob_gas: u64,
}

/// SSZ `ExecutionRequests` — a list of opaque request byte strings.
pub type SszExecutionRequests = List<List<u8, MAX_REQUEST_DATA_BYTES>, MAX_EXECUTION_REQUESTS>;

/// SSZ `NewPayloadRequest` container.
///
/// This is the container whose `hash_tree_root` is committed to as
/// the `new_payload_request_root` in the proof's public input.
#[derive(Debug, Default, Clone, PartialEq, Eq, SimpleSerialize)]
pub struct SszNewPayloadRequest {
    pub execution_payload: SszExecutionPayload,
    pub versioned_hashes: List<[u8; 32], MAX_BLOB_COMMITMENTS_PER_BLOCK>,
    pub parent_beacon_block_root: [u8; 32],
    pub execution_requests: SszExecutionRequests,
}

/// SSZ `NewPayloadRequestHeader` container.
///
/// Same structure as `NewPayloadRequest` but uses the header
/// variant of the execution payload.
#[derive(Debug, Default, Clone, PartialEq, Eq, SimpleSerialize)]
pub struct SszNewPayloadRequestHeader {
    pub execution_payload_header: SszExecutionPayloadHeader,
    pub versioned_hashes: List<[u8; 32], MAX_BLOB_COMMITMENTS_PER_BLOCK>,
    pub parent_beacon_block_root: [u8; 32],
    pub execution_requests: SszExecutionRequests,
}

/// SSZ `PublicInput` container.
///
/// The public input committed to by the ZK proof. Contains the
/// `hash_tree_root` of the `NewPayloadRequest`.
#[derive(Debug, Default, Clone, PartialEq, Eq, SimpleSerialize)]
pub struct SszPublicInput {
    pub new_payload_request_root: [u8; 32],
}

/// Compute the `hash_tree_root` of a `NewPayloadRequest`.
pub fn new_payload_request_root(
    request: &mut SszNewPayloadRequest,
) -> Result<Node, MerkleizationError> {
    request.hash_tree_root()
}

/// Compute the `hash_tree_root` of a `NewPayloadRequestHeader`.
pub fn new_payload_request_header_root(
    header: &mut SszNewPayloadRequestHeader,
) -> Result<Node, MerkleizationError> {
    header.hash_tree_root()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_new_payload_request_hashes() {
        let mut req = SszNewPayloadRequest::default();
        let root = req.hash_tree_root();
        assert!(root.is_ok(), "hash_tree_root should succeed on default");
    }

    #[test]
    fn empty_new_payload_request_header_hashes() {
        let mut header = SszNewPayloadRequestHeader::default();
        let root = header.hash_tree_root();
        assert!(root.is_ok(), "hash_tree_root should succeed on default");
    }

    #[test]
    fn public_input_round_trip() {
        let mut req = SszNewPayloadRequest::default();
        let root = req.hash_tree_root().expect("hash_tree_root failed");

        let mut root_bytes = [0u8; 32];
        root_bytes.copy_from_slice(root.as_ref());

        let mut pi = SszPublicInput {
            new_payload_request_root: root_bytes,
        };
        let pi_root = pi.hash_tree_root();
        assert!(pi_root.is_ok());
    }
}
