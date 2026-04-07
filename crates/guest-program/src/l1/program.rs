use std::sync::Arc;

use ethrex_crypto::Crypto;

use crate::common::{ExecutionError, execute_blocks};
#[cfg(not(feature = "eip-8025"))]
use crate::l1::input::ProgramInput;
use crate::l1::output::ProgramOutput;

use ethrex_common::types::ELASTICITY_MULTIPLIER;
use ethrex_vm::Evm;

#[cfg(not(feature = "eip-8025"))]
use crate::common::BatchExecutionResult;

/// Execute the L1 stateless validation program.
///
/// This validates and executes a batch of L1 blocks, verifying state transitions
/// without access to the full blockchain state.
#[cfg(not(feature = "eip-8025"))]
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

/// Execute the L1 stateless validation program (EIP-8025).
///
/// This transforms the SSZ `NewPayloadRequest` into a `Block`, validates it,
/// executes it statelessly, and produces the `hash_tree_root` commitment.
///
/// Takes the raw `NewPayloadRequest` and `ExecutionWitness` decoded from the
/// EIP-8025 wire format (see [`decode_eip8025`](super::decode_eip8025)).
#[cfg(feature = "eip-8025")]
pub fn execution_program(
    new_payload_request: ethrex_common::types::eip8025_ssz::NewPayloadRequest,
    execution_witness: ethrex_common::types::block_execution_witness::ExecutionWitness,
    crypto: Arc<dyn Crypto>,
) -> Result<ProgramOutput, ExecutionError> {
    use libssz_merkle::{HashTreeRoot, Sha2Hasher};

    // Compute the hash_tree_root before consuming the payload.
    let request_root = new_payload_request.hash_tree_root(&Sha2Hasher);

    // Transform SSZ NewPayloadRequest → Block
    let block = new_payload_request_to_block(&new_payload_request, crypto.as_ref())
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
    validate_versioned_hashes(&block, &new_payload_request)?;

    // Execute statelessly — reuse the common `execute_blocks` infrastructure
    let _result = execute_blocks(
        &[block],
        execution_witness,
        ELASTICITY_MULTIPLIER,
        |db, _| Ok(Evm::new_for_l1(db.clone(), crypto.clone())),
        crypto.clone(),
    )?;

    Ok(ProgramOutput {
        new_payload_request_root: request_root,
        valid: true,
    })
}

/// Transform an SSZ `NewPayloadRequest` into a `Block`.
#[cfg(feature = "eip-8025")]
fn new_payload_request_to_block(
    req: &ethrex_common::types::eip8025_ssz::NewPayloadRequest,
    crypto: &dyn Crypto,
) -> Result<ethrex_common::types::Block, String> {
    use bytes::Bytes;
    use ethrex_common::constants::DEFAULT_OMMERS_HASH;
    use ethrex_common::types::requests::{EncodedRequests, compute_requests_hash};
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

    // Build execution_requests from the SSZ field for requests_hash
    let execution_requests: Vec<EncodedRequests> = req
        .execution_requests
        .iter()
        .map(|r| {
            let raw: Vec<u8> = r.iter().copied().collect();
            EncodedRequests(Bytes::from(raw))
        })
        .collect();
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

    let header = BlockHeader {
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

    Ok(Block::new(header, body))
}

/// Validate that the blob versioned hashes in the `NewPayloadRequest` match
/// the blob commitments in the block's transactions.
#[cfg(feature = "eip-8025")]
fn validate_versioned_hashes(
    block: &ethrex_common::types::Block,
    req: &ethrex_common::types::eip8025_ssz::NewPayloadRequest,
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
