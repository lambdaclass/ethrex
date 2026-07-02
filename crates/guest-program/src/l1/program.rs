use std::sync::Arc;

use ethrex_crypto::Crypto;

use crate::common::{ExecutionError, execute_blocks};
#[cfg(not(feature = "experimental-devnet"))]
use crate::l1::input::ProgramInput;
use crate::l1::output::ProgramOutput;

use ethrex_common::types::ELASTICITY_MULTIPLIER;
use ethrex_vm::Evm;

#[cfg(not(feature = "experimental-devnet"))]
use crate::common::BatchExecutionResult;

/// Execute the L1 stateless validation program.
///
/// This validates and executes a batch of L1 blocks, verifying state transitions
/// without access to the full blockchain state.
#[cfg(not(feature = "experimental-devnet"))]
pub fn execution_program(
    input: ProgramInput,
    crypto: Arc<dyn Crypto>,
) -> Result<ProgramOutput, ExecutionError> {
    let ProgramInput {
        blocks,
        execution_witness,
    } = input;

    let BatchExecutionResult {
        receipts: _,
        initial_state_hash,
        final_state_hash,
        last_block_hash,
        non_privileged_count,
        chain_id,
    } = execute_blocks(
        &blocks,
        execution_witness,
        ELASTICITY_MULTIPLIER,
        |db, _| {
            // L1 VM factory - simple creation without fee configs
            Ok(Evm::new_for_l1(db.clone(), crypto.clone()))
        },
        crypto.clone(),
    )?;

    Ok(ProgramOutput {
        initial_state_hash,
        final_state_hash,
        last_block_hash,
        chain_id: chain_id.into(),
        transaction_count: non_privileged_count,
    })
}

/// Decode and execute the L1 stateless validation program from EIP-8025 wire
/// bytes.
///
/// The wire format is `[ssz_len: u32 LE][ssz_bytes][rkyv_bytes]`, matching
/// [`decode_eip8025`](super::decode_eip8025).
#[cfg(feature = "experimental-devnet")]
pub fn execution_program(
    bytes: &[u8],
    crypto: Arc<dyn Crypto>,
) -> Result<ProgramOutput, ExecutionError> {
    use libssz_merkle::{HashTreeRoot, Sha2Hasher};

    let (new_payload_request, execution_witness) = super::decode_eip8025(bytes).map_err(|err| {
        ExecutionError::Internal(format!("failed to decode EIP-8025 input: {err}"))
    })?;

    let request_root = new_payload_request.hash_tree_root(&Sha2Hasher);
    let valid = validate_eip8025_execution(&new_payload_request, execution_witness, crypto).is_ok();

    Ok(ProgramOutput {
        new_payload_request_root: request_root,
        valid,
    })
}

/// Transform an SSZ `NewPayloadRequest` into a `Block`.
#[cfg(feature = "experimental-devnet")]
pub fn new_payload_request_to_block(
    req: &ethrex_common::types::stateless_ssz::NewPayloadRequest,
    crypto: &dyn Crypto,
) -> Result<ethrex_common::types::Block, String> {
    use bytes::Bytes;
    use ethrex_common::constants::DEFAULT_OMMERS_HASH;
    use ethrex_common::types::requests::compute_requests_hash;
    use ethrex_common::types::{
        Block, BlockBody, BlockHeader, Transaction, Withdrawal, compute_transactions_root,
        compute_withdrawals_root,
    };
    use ethrex_common::{Address, Bloom, H256};

    let payload = &req.execution_payload;

    // Decode transactions from raw bytes
    let transactions: Vec<Transaction> = payload
        .transactions
        .iter()
        .map(|tx_bytes| {
            let raw: Vec<u8> = tx_bytes.iter().copied().collect();
            Transaction::decode_canonical(&raw).map_err(|e| format!("tx decode: {e}"))
        })
        .collect::<Result<Vec<_>, _>>()?;

    // Convert SSZ withdrawals to ethrex Withdrawals
    let withdrawals: Vec<Withdrawal> = payload
        .withdrawals
        .iter()
        .map(|w| Withdrawal {
            index: w.index,
            validator_index: w.validator_index,
            address: Address::from_slice(&w.address.0),
            amount: w.amount,
        })
        .collect();

    // Build execution_requests from the SSZ typed ExecutionRequests field
    let execution_requests = req.execution_requests.to_encoded_requests();
    let requests_hash = compute_requests_hash(&execution_requests);

    // Convert base_fee_per_gas from [u8; 32] LE uint256 to u64
    // (base_fee fits in u64 for all practical purposes)
    let base_fee_per_gas = u64::from_le_bytes(
        payload.base_fee_per_gas[..8]
            .try_into()
            .map_err(|_| "base_fee_per_gas conversion")?,
    );

    // Build logs_bloom from SszVector<u8, 256>
    let bloom_bytes: Vec<u8> = payload.logs_bloom.iter().copied().collect();
    let logs_bloom = Bloom::from_slice(&bloom_bytes);

    let body = BlockBody {
        transactions: transactions.clone(),
        ommers: vec![],
        withdrawals: Some(withdrawals.clone()),
    };

    let mut header = BlockHeader {
        parent_hash: H256::from_slice(&payload.parent_hash),
        ommers_hash: *DEFAULT_OMMERS_HASH,
        coinbase: Address::from_slice(&payload.fee_recipient.0),
        state_root: H256::from_slice(&payload.state_root),
        transactions_root: compute_transactions_root(&body.transactions, crypto),
        receipts_root: H256::from_slice(&payload.receipts_root),
        logs_bloom,
        difficulty: 0.into(),
        number: payload.block_number,
        gas_limit: payload.gas_limit,
        gas_used: payload.gas_used,
        timestamp: payload.timestamp,
        extra_data: Bytes::from(payload.extra_data.iter().copied().collect::<Vec<u8>>()),
        prev_randao: H256::from_slice(&payload.prev_randao),
        nonce: 0,
        base_fee_per_gas: Some(base_fee_per_gas),
        withdrawals_root: Some(compute_withdrawals_root(&withdrawals, crypto)),
        blob_gas_used: Some(payload.blob_gas_used),
        excess_blob_gas: Some(payload.excess_blob_gas),
        parent_beacon_block_root: Some(H256::from_slice(&req.parent_beacon_block_root)),
        requests_hash: Some(requests_hash),
        ..Default::default()
    };

    // EIP-7928: when the payload carries a Block Access List, derive the header
    // commitment from it. ethrex encodes the BAL as RLP (the preimage of
    // block_access_list_hash), so decode-then-compute_hash reproduces the exact
    // hash the producer set — honoring the empty-BAL special case. An empty
    // field means pre-Amsterdam: leave block_access_list_hash as None.
    if !payload.block_access_list.is_empty() {
        use ethrex_rlp::decode::RLPDecode;
        let bal_bytes: Vec<u8> = payload.block_access_list.iter().copied().collect();
        let bal = ethrex_common::types::block_access_list::BlockAccessList::decode(&bal_bytes)
            .map_err(|e| format!("block_access_list decode: {e}"))?;
        header.block_access_list_hash = Some(bal.compute_hash());
    }

    Ok(Block::new(header, body))
}

/// Validate that the blob versioned hashes in the `NewPayloadRequest` match
/// the blob commitments in the block's transactions.
#[cfg(feature = "experimental-devnet")]
fn validate_versioned_hashes(
    block: &ethrex_common::types::Block,
    req: &ethrex_common::types::stateless_ssz::NewPayloadRequest,
) -> Result<(), ExecutionError> {
    use ethrex_common::H256;

    // Collect all versioned hashes from blob transactions in order
    let tx_hashes: Vec<H256> = block
        .body
        .transactions
        .iter()
        .flat_map(|tx| tx.blob_versioned_hashes())
        .collect();

    let req_hashes: Vec<H256> = req
        .versioned_hashes
        .iter()
        .map(|h| H256::from_slice(h))
        .collect();

    if tx_hashes != req_hashes {
        return Err(ExecutionError::Internal(
            "versioned hashes mismatch between NewPayloadRequest and transactions".to_string(),
        ));
    }

    Ok(())
}

#[cfg(feature = "experimental-devnet")]
fn validate_eip8025_execution(
    new_payload_request: &ethrex_common::types::stateless_ssz::NewPayloadRequest,
    execution_witness: ethrex_common::types::block_execution_witness::ExecutionWitness,
    crypto: Arc<dyn Crypto>,
) -> Result<(), ExecutionError> {
    // Transform SSZ NewPayloadRequest → Block
    let block = new_payload_request_to_block(new_payload_request, crypto.as_ref())
        .map_err(|e| ExecutionError::Internal(format!("payload conversion: {e}")))?;

    // Validate block_hash: the SSZ payload carries block_hash which must match
    // the hash of the reconstructed block header.
    let computed_hash = block.hash();
    let expected_hash =
        ethrex_common::H256::from_slice(&new_payload_request.execution_payload.block_hash);
    if computed_hash != expected_hash {
        return Err(ExecutionError::Internal(format!(
            "block_hash mismatch: expected {expected_hash:?}, got {computed_hash:?}"
        )));
    }

    // Validate blob versioned hashes
    validate_versioned_hashes(&block, new_payload_request)?;

    // Execute statelessly — reuse the common `execute_blocks` infrastructure
    let _result = execute_blocks(
        &[block],
        execution_witness,
        ELASTICITY_MULTIPLIER,
        |db, _| Ok(Evm::new_for_l1(db.clone(), crypto.clone())),
        crypto.clone(),
    )?;

    Ok(())
}

#[cfg(all(test, feature = "experimental-devnet"))]
mod tests {
    use std::sync::Arc;

    use crate::{common::ExecutionError, crypto::NativeCrypto, l1::execution_program};

    #[test]
    fn execution_program_rejects_invalid_eip8025_wire_bytes() {
        let err = match execution_program(&[], Arc::new(NativeCrypto)) {
            Ok(_) => panic!("expected invalid EIP-8025 input to fail decoding"),
            Err(err) => err,
        };

        match err {
            ExecutionError::Internal(msg) => {
                assert_eq!(msg, "failed to decode EIP-8025 input: input too short");
            }
            other => panic!("expected internal decode error, got {other:?}"),
        }
    }

    #[test]
    fn reconstruction_derives_block_access_list_hash() {
        use ethrex_common::types::block_access_list::{AccountChanges, BlockAccessList};
        use ethrex_common::Address;
        use ethrex_rlp::encode::RLPEncode;

        let crypto = crate::crypto::NativeCrypto;

        // Empty field → None (pre-Amsterdam, unchanged behavior).
        let req_empty = build_minimal_new_payload_request(Vec::new());
        let block = super::new_payload_request_to_block(&req_empty, &crypto)
            .expect("reconstruct empty");
        assert_eq!(block.header.block_access_list_hash, None);

        // Non-empty field → Some(compute_hash(decoded)).
        let bal =
            BlockAccessList::from_accounts(vec![AccountChanges::new(Address::from([0x22u8; 20]))]);
        let bytes = bal.encode_to_vec();
        let req = build_minimal_new_payload_request(bytes);
        let block = super::new_payload_request_to_block(&req, &crypto)
            .expect("reconstruct with bal");
        assert_eq!(block.header.block_access_list_hash, Some(bal.compute_hash()));
    }

    fn build_minimal_new_payload_request(
        block_access_list_bytes: Vec<u8>,
    ) -> ethrex_common::types::stateless_ssz::NewPayloadRequest {
        use ethrex_common::types::stateless_ssz::{
            Bytes20, ExecutionPayload, ExecutionRequests, NewPayloadRequest,
        };
        use libssz_types::SszList;

        NewPayloadRequest {
            execution_payload: ExecutionPayload {
                parent_hash: [0u8; 32],
                fee_recipient: Bytes20([0u8; 20]),
                state_root: [0u8; 32],
                receipts_root: [0u8; 32],
                logs_bloom: vec![0u8; 256].try_into().expect("bloom"),
                prev_randao: [0u8; 32],
                block_number: 0,
                gas_limit: 0,
                gas_used: 0,
                timestamp: 0,
                extra_data: SszList::new(),
                base_fee_per_gas: [0u8; 32],
                block_hash: [0u8; 32],
                transactions: SszList::new(),
                withdrawals: SszList::new(),
                blob_gas_used: 0,
                excess_blob_gas: 0,
                // Type inferred from ExecutionPayload field: SszList<u8, MAX_BLOCK_ACCESS_LIST_BYTES>.
                block_access_list: {
                    let mut l = SszList::new();
                    for b in block_access_list_bytes {
                        let _ = l.push(b);
                    }
                    l
                },
            },
            versioned_hashes: SszList::new(),
            parent_beacon_block_root: [0u8; 32],
            execution_requests: ExecutionRequests {
                deposits: SszList::new(),
                withdrawals: SszList::new(),
                consolidations: SszList::new(),
            },
        }
    }
}
