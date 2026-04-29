use std::sync::Arc;

use ethrex_crypto::Crypto;

use crate::common::ExecutionError;
use crate::common::execute_blocks;
#[cfg(not(feature = "eip-8025"))]
use crate::l1::input::ProgramInput;
#[cfg(feature = "eip-8025")]
use crate::l1::input::{CanonicalExecutionWitness, CanonicalStatelessInput, DecodedEip8025};
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

/// Decode and execute the L1 stateless validation program from EIP-8025 wire
/// bytes.
///
/// The wire format is version-prefixed; see [`super::decode_eip8025`] for the
/// per-version layout. Legacy and canonical-input payloads both commit to the
/// decoded `NewPayloadRequest` root and report execution validity as a boolean.
#[cfg(feature = "eip-8025")]
pub fn execution_program(
    bytes: &[u8],
    crypto: Arc<dyn Crypto>,
) -> Result<ProgramOutput, ExecutionError> {
    use libssz_merkle::{HashTreeRoot, Sha2Hasher};

    let decoded = super::decode_eip8025(bytes).map_err(|err| {
        ExecutionError::Internal(format!("failed to decode EIP-8025 input: {err}"))
    })?;

    match decoded {
        DecodedEip8025::Legacy {
            new_payload_request,
            execution_witness,
        } => {
            let request_root = new_payload_request.hash_tree_root(&Sha2Hasher);
            let valid =
                validate_eip8025_execution(&new_payload_request, execution_witness, crypto).is_ok();

            Ok(ProgramOutput {
                new_payload_request_root: request_root,
                valid,
            })
        }
        DecodedEip8025::Canonical {
            stateless_input,
            chain_config,
        } => {
            let request_root = stateless_input
                .new_payload_request
                .hash_tree_root(&Sha2Hasher);
            let valid =
                validate_eip8025_canonical_execution(stateless_input, chain_config, crypto).is_ok();

            Ok(ProgramOutput {
                new_payload_request_root: request_root,
                valid,
            })
        }
    }
}

#[cfg(feature = "eip-8025")]
fn decode_payload_transactions<const MAX_TXS: usize, const MAX_BYTES_PER_TX: usize>(
    transactions: &libssz_types::SszList<libssz_types::SszList<u8, MAX_BYTES_PER_TX>, MAX_TXS>,
) -> Result<Vec<ethrex_common::types::Transaction>, String> {
    transactions
        .iter()
        .map(|tx_bytes| {
            let raw: Vec<u8> = tx_bytes.iter().copied().collect();
            ethrex_common::types::Transaction::decode_canonical(&raw)
                .map_err(|e| format!("tx decode: {e}"))
        })
        .collect::<Result<Vec<_>, _>>()
}

#[cfg(feature = "eip-8025")]
fn decode_payload_withdrawals<const MAX_WITHDRAWALS: usize>(
    withdrawals: &libssz_types::SszList<
        ethrex_common::types::eip8025_ssz::Withdrawal,
        MAX_WITHDRAWALS,
    >,
) -> Vec<ethrex_common::types::Withdrawal> {
    use ethrex_common::Address;

    withdrawals
        .iter()
        .map(|w| ethrex_common::types::Withdrawal {
            index: w.index,
            validator_index: w.validator_index,
            address: Address::from_slice(&w.address.0),
            amount: w.amount,
        })
        .collect()
}

#[cfg(feature = "eip-8025")]
fn base_fee_per_gas_from_le_bytes(bytes: &[u8; 32]) -> Result<u64, String> {
    Ok(u64::from_le_bytes(
        bytes[..8]
            .try_into()
            .map_err(|_| "base_fee_per_gas conversion")?,
    ))
}

#[cfg(feature = "eip-8025")]
fn validate_reconstructed_block_hash(
    block: &ethrex_common::types::Block,
    expected_hash: &[u8; 32],
    crypto: &dyn Crypto,
) -> Result<(), String> {
    let computed_hash = block.header.compute_block_hash(crypto);
    let expected_hash = ethrex_common::H256::from_slice(expected_hash);
    if computed_hash != expected_hash {
        return Err(format!(
            "block_hash mismatch: expected {expected_hash:?}, got {computed_hash:?}"
        ));
    }

    Ok(())
}

/// Transform an SSZ `NewPayloadRequest` into a `Block`.
#[cfg(feature = "eip-8025")]
fn new_payload_request_to_block(
    req: &ethrex_common::types::eip8025_ssz::NewPayloadRequest,
    crypto: &dyn Crypto,
) -> Result<ethrex_common::types::Block, String> {
    use bytes::Bytes;
    use ethrex_common::constants::DEFAULT_OMMERS_HASH;
    use ethrex_common::types::requests::compute_requests_hash;
    use ethrex_common::types::{
        Block, BlockBody, BlockHeader, compute_transactions_root, compute_withdrawals_root,
    };
    use ethrex_common::{Address, Bloom, H256};

    let payload = &req.execution_payload;

    let transactions = decode_payload_transactions(&payload.transactions)?;

    let withdrawals = decode_payload_withdrawals(&payload.withdrawals);

    // Build execution_requests from the SSZ typed ExecutionRequests field
    let execution_requests = req.execution_requests.to_encoded_requests();
    let requests_hash = compute_requests_hash(&execution_requests);

    let base_fee_per_gas = base_fee_per_gas_from_le_bytes(&payload.base_fee_per_gas)?;

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

/// Transform an Amsterdam SSZ `NewPayloadRequest` into a `Block`.
#[cfg(feature = "eip-8025")]
fn new_payload_request_amsterdam_to_block(
    req: &ethrex_common::types::eip8025_ssz::NewPayloadRequestAmsterdam,
    crypto: &dyn Crypto,
) -> Result<ethrex_common::types::Block, String> {
    use bytes::Bytes;
    use ethrex_common::constants::DEFAULT_OMMERS_HASH;
    use ethrex_common::types::block_access_list::BlockAccessList;
    use ethrex_common::types::requests::compute_requests_hash;
    use ethrex_common::types::{
        Block, BlockBody, BlockHeader, compute_transactions_root, compute_withdrawals_root,
    };
    use ethrex_common::{Address, Bloom, H256};
    use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};

    let payload = &req.execution_payload;

    let transactions = decode_payload_transactions(&payload.transactions)?;
    let withdrawals = decode_payload_withdrawals(&payload.withdrawals);

    let bal_bytes: Vec<u8> = payload.block_access_list.iter().copied().collect();
    let block_access_list = BlockAccessList::decode(&bal_bytes)
        .map_err(|e| format!("block access list decode: {e}"))?;
    block_access_list
        .validate_ordering()
        .map_err(|e| format!("block access list ordering: {e}"))?;
    if block_access_list.encode_to_vec() != bal_bytes {
        return Err("block access list is not canonically encoded".to_string());
    }

    let execution_requests = req.execution_requests.to_encoded_requests();
    let requests_hash = compute_requests_hash(&execution_requests);
    let base_fee_per_gas = base_fee_per_gas_from_le_bytes(&payload.base_fee_per_gas)?;
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
        block_access_list_hash: Some(block_access_list.compute_hash()),
        slot_number: Some(payload.slot_number),
        ..Default::default()
    };

    let block = Block::new(header, body);
    validate_reconstructed_block_hash(&block, &payload.block_hash, crypto)?;
    Ok(block)
}

/// Validate that the blob versioned hashes in the `NewPayloadRequest` match
/// the blob commitments in the block's transactions.
#[cfg(feature = "eip-8025")]
fn validate_versioned_hashes<'a>(
    block: &ethrex_common::types::Block,
    versioned_hashes: impl IntoIterator<Item = &'a [u8; 32]>,
) -> Result<(), ExecutionError> {
    use ethrex_common::H256;

    // Collect all versioned hashes from blob transactions in order
    let tx_hashes: Vec<H256> = block
        .body
        .transactions
        .iter()
        .flat_map(|tx| tx.blob_versioned_hashes())
        .collect();

    let req_hashes: Vec<H256> = versioned_hashes
        .into_iter()
        .map(|h| H256::from_slice(h))
        .collect();

    if tx_hashes != req_hashes {
        return Err(ExecutionError::Internal(
            "versioned hashes mismatch between NewPayloadRequest and transactions".to_string(),
        ));
    }

    Ok(())
}

#[cfg(feature = "eip-8025")]
fn canonical_execution_witness_to_rpc(
    witness: CanonicalExecutionWitness,
) -> ethrex_common::types::block_execution_witness::RpcExecutionWitness {
    use bytes::Bytes;

    fn copy_ssz_bytes<const MAX_BYTES: usize>(
        bytes: &libssz_types::SszList<u8, MAX_BYTES>,
    ) -> Bytes {
        Bytes::from(bytes.iter().copied().collect::<Vec<u8>>())
    }

    ethrex_common::types::block_execution_witness::RpcExecutionWitness {
        state: witness.state.iter().map(copy_ssz_bytes).collect(),
        keys: Vec::new(),
        codes: witness.codes.iter().map(copy_ssz_bytes).collect(),
        headers: witness.headers.iter().map(copy_ssz_bytes).collect(),
    }
}

#[cfg(feature = "eip-8025")]
fn validate_eip8025_canonical_execution(
    stateless_input: CanonicalStatelessInput,
    chain_config: ethrex_common::types::ChainConfig,
    crypto: Arc<dyn Crypto>,
) -> Result<(), ExecutionError> {
    if stateless_input.chain_config.chain_id != chain_config.chain_id {
        return Err(ExecutionError::Internal(format!(
            "chain_id mismatch between canonical input ({}) and chain config ({})",
            stateless_input.chain_config.chain_id, chain_config.chain_id
        )));
    }

    let block_number = stateless_input
        .new_payload_request
        .execution_payload
        .block_number;
    let rpc_witness = canonical_execution_witness_to_rpc(stateless_input.witness);
    let execution_witness = rpc_witness.into_execution_witness(chain_config, block_number)?;

    validate_eip8025_amsterdam_execution(
        &stateless_input.new_payload_request,
        execution_witness,
        crypto,
    )
}

#[cfg(feature = "eip-8025")]
fn validate_eip8025_execution(
    new_payload_request: &ethrex_common::types::eip8025_ssz::NewPayloadRequest,
    execution_witness: ethrex_common::types::block_execution_witness::ExecutionWitness,
    crypto: Arc<dyn Crypto>,
) -> Result<(), ExecutionError> {
    // Transform SSZ NewPayloadRequest → Block
    let block = new_payload_request_to_block(new_payload_request, crypto.as_ref())
        .map_err(|e| ExecutionError::Internal(format!("payload conversion: {e}")))?;

    validate_reconstructed_block_hash(
        &block,
        &new_payload_request.execution_payload.block_hash,
        crypto.as_ref(),
    )
    .map_err(|e| ExecutionError::Internal(format!("payload conversion: {e}")))?;

    // Validate blob versioned hashes
    validate_versioned_hashes(&block, new_payload_request.versioned_hashes.iter())?;

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

#[cfg(feature = "eip-8025")]
fn validate_eip8025_amsterdam_execution(
    new_payload_request: &ethrex_common::types::eip8025_ssz::NewPayloadRequestAmsterdam,
    execution_witness: ethrex_common::types::block_execution_witness::ExecutionWitness,
    crypto: Arc<dyn Crypto>,
) -> Result<(), ExecutionError> {
    let block = new_payload_request_amsterdam_to_block(new_payload_request, crypto.as_ref())
        .map_err(|e| ExecutionError::Internal(format!("payload conversion: {e}")))?;

    validate_versioned_hashes(&block, new_payload_request.versioned_hashes.iter())?;

    let _result = execute_blocks(
        &[block],
        execution_witness,
        ELASTICITY_MULTIPLIER,
        |db, _| Ok(Evm::new_for_l1(db.clone(), crypto.clone())),
        crypto.clone(),
    )?;

    Ok(())
}

#[cfg(all(test, feature = "eip-8025"))]
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
}
