use std::sync::Arc;

#[cfg(feature = "eip-8025")]
use ethrex_common::Address;
#[cfg(feature = "eip-8025")]
use ethrex_common::utils::keccak;
use ethrex_crypto::Crypto;

use crate::common::ExecutionError;
use crate::common::execute_blocks;
use crate::l1::input::ProgramInput;
#[cfg(feature = "eip-8025")]
use crate::l1::input::{
    CanonicalExecutionWitness, CanonicalStatelessInput, DecodedEip8025, PublicKeysList,
};
use crate::l1::output::ProgramOutput;

use ethrex_common::types::ELASTICITY_MULTIPLIER;
use ethrex_common::validate_block_access_list_hash;
use ethrex_vm::Evm;

#[cfg(feature = "eip-8025")]
use libssz_merkle::Sha256Hasher;

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
        burned_fees: _,
        bals: _,
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

/// Wrapper to bridge `ethrex_crypto::Crypto` to `libssz_merkle::Sha256Hasher`,
/// so `hash_tree_root` is computed via crypto precompiles in the zkVM.
/// Required because the orphan rule prevents a direct impl on `Arc<dyn Crypto>`.
#[cfg(feature = "eip-8025")]
struct CryptoWrapper(Arc<dyn Crypto>);

#[cfg(feature = "eip-8025")]
impl Sha256Hasher for CryptoWrapper {
    fn hash(&self, data: &[u8]) -> [u8; 32] {
        self.0.sha256(data)
    }
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
    let decoded = super::decode_eip8025(bytes).map_err(|err| {
        ExecutionError::Internal(format!("failed to decode EIP-8025 input: {err}"))
    })?;

    execute_decoded(ProgramInput::Wire(decoded), crypto)
}

/// Execute an already-built [`ProgramInput`].
///
/// The `Direct` arm has no `NewPayloadRequest`, so it returns a sentinel
/// `ProgramOutput` with zero request_root and `valid = true`. `ExecBackend`
/// promotes `valid = false` to `Err` for result-only callers.
#[cfg(feature = "eip-8025")]
pub fn execute_decoded(
    input: ProgramInput,
    crypto: Arc<dyn Crypto>,
) -> Result<ProgramOutput, ExecutionError> {
    use libssz_merkle::HashTreeRoot;

    match input {
        ProgramInput::Direct {
            blocks,
            execution_witness,
        } => {
            let chain_id = execution_witness.chain_config.chain_id;
            execute_blocks(
                &blocks,
                execution_witness,
                ELASTICITY_MULTIPLIER,
                |db, _| Ok(Evm::new_for_l1(db.clone(), crypto.clone())),
                crypto.clone(),
            )?;
            Ok(ProgramOutput {
                new_payload_request_root: [0u8; 32],
                valid: true,
                chain_id,
            })
        }
        ProgramInput::Wire(DecodedEip8025::Legacy {
            new_payload_request,
            execution_witness,
        }) => {
            let request_root = new_payload_request.hash_tree_root(&CryptoWrapper(crypto.clone()));
            let chain_id = execution_witness.chain_config.chain_id;
            let valid =
                validate_eip8025_execution(&new_payload_request, execution_witness, crypto).is_ok();

            Ok(ProgramOutput {
                new_payload_request_root: request_root,
                valid,
                chain_id,
            })
        }
        ProgramInput::Wire(DecodedEip8025::Canonical {
            stateless_input,
            chain_config,
        }) => Ok(execute_canonical_stateless_input_decoded(
            stateless_input,
            chain_config,
            crypto,
        )),
    }
}

#[cfg(feature = "eip-8025")]
fn execute_canonical_stateless_input_decoded(
    stateless_input: CanonicalStatelessInput,
    chain_config: ethrex_common::types::ChainConfig,
    crypto: Arc<dyn Crypto>,
) -> ProgramOutput {
    use libssz_merkle::HashTreeRoot;

    let request_root = stateless_input
        .new_payload_request
        .hash_tree_root(&CryptoWrapper(crypto.clone()));
    let chain_id = stateless_input.chain_config.chain_id;
    let valid = validate_eip8025_canonical_execution(stateless_input, chain_config, crypto).is_ok();

    ProgramOutput {
        new_payload_request_root: request_root,
        valid,
        chain_id,
    }
}

#[cfg(feature = "eip-8025")]
fn decode_payload_transactions<const MAX_TXS: usize, const MAX_BYTES_PER_TX: usize>(
    transactions: &libssz_types::SszList<libssz_types::SszList<u8, MAX_BYTES_PER_TX>, MAX_TXS>,
) -> Result<Vec<ethrex_common::types::Transaction>, String> {
    transactions
        .iter()
        .map(|tx_bytes| {
            ethrex_common::types::Transaction::decode_canonical(tx_bytes)
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

/// Convert a 32-byte little-endian SSZ `uint256` base-fee field to `u64`.
///
/// The upper 24 bytes (`[8..32]`) MUST be zero. They don't affect block validation
/// (base fee fits in `u64` for any real chain), but they ARE covered by
/// `NewPayloadRequest::hash_tree_root()`. Silently truncating to the low 8 bytes
/// would let ~2^192 distinct SSZ inputs reconstruct the *same* block while producing
/// *different* roots, breaking the "one block ⇒ one root" commitment invariant for
/// any root-keyed consumer (e.g. a ZK variant or a root-anchored settlement path).
/// Rejecting non-zero upper bytes closes that malleability.
fn base_fee_per_gas_from_le_bytes(bytes: &[u8; 32]) -> Result<u64, String> {
    if bytes[8..].iter().any(|&b| b != 0) {
        return Err("base_fee_per_gas exceeds u64 (non-zero upper bytes)".to_string());
    }
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
fn eip8025_new_payload_request_to_block(
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
    let logs_bloom = Bloom::from_slice(&payload.logs_bloom);

    let transactions_root = compute_transactions_root(&transactions, crypto);
    let withdrawals_root = compute_withdrawals_root(&withdrawals, crypto);

    let body = BlockBody {
        transactions,
        ommers: vec![],
        withdrawals: Some(withdrawals),
    };

    let header = BlockHeader {
        parent_hash: H256::from_slice(&payload.parent_hash),
        ommers_hash: *DEFAULT_OMMERS_HASH,
        coinbase: Address::from_slice(&payload.fee_recipient.0),
        state_root: H256::from_slice(&payload.state_root),
        transactions_root,
        receipts_root: H256::from_slice(&payload.receipts_root),
        logs_bloom,
        difficulty: 0.into(),
        number: payload.block_number,
        gas_limit: payload.gas_limit,
        gas_used: payload.gas_used,
        timestamp: payload.timestamp,
        extra_data: Bytes::copy_from_slice(&payload.extra_data),
        prev_randao: H256::from_slice(&payload.prev_randao),
        nonce: 0,
        base_fee_per_gas: Some(base_fee_per_gas),
        withdrawals_root: Some(withdrawals_root),
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

    let block_access_list = BlockAccessList::decode(&payload.block_access_list)
        .map_err(|e| format!("block access list decode: {e}"))?;
    block_access_list
        .validate_ordering()
        .map_err(|e| format!("block access list ordering: {e}"))?;
    if block_access_list.encode_to_vec().as_slice() != &payload.block_access_list[..] {
        return Err("block access list is not canonically encoded".to_string());
    }

    let execution_requests = req.execution_requests.to_encoded_requests();
    let requests_hash = compute_requests_hash(&execution_requests);
    let base_fee_per_gas = base_fee_per_gas_from_le_bytes(&payload.base_fee_per_gas)?;
    let logs_bloom = Bloom::from_slice(&payload.logs_bloom);

    let transactions_root = compute_transactions_root(&transactions, crypto);
    let withdrawals_root = compute_withdrawals_root(&withdrawals, crypto);

    let body = BlockBody {
        transactions,
        ommers: vec![],
        withdrawals: Some(withdrawals),
    };

    let header = BlockHeader {
        parent_hash: H256::from_slice(&payload.parent_hash),
        ommers_hash: *DEFAULT_OMMERS_HASH,
        coinbase: Address::from_slice(&payload.fee_recipient.0),
        state_root: H256::from_slice(&payload.state_root),
        transactions_root,
        receipts_root: H256::from_slice(&payload.receipts_root),
        logs_bloom,
        difficulty: 0.into(),
        number: payload.block_number,
        gas_limit: payload.gas_limit,
        gas_used: payload.gas_used,
        timestamp: payload.timestamp,
        extra_data: Bytes::copy_from_slice(&payload.extra_data),
        prev_randao: H256::from_slice(&payload.prev_randao),
        nonce: 0,
        base_fee_per_gas: Some(base_fee_per_gas),
        withdrawals_root: Some(withdrawals_root),
        blob_gas_used: Some(payload.blob_gas_used),
        excess_blob_gas: Some(payload.excess_blob_gas),
        parent_beacon_block_root: Some(H256::from_slice(&req.parent_beacon_block_root)),
        requests_hash: Some(requests_hash),
        block_access_list_hash: Some(block_access_list.compute_hash(crypto)),
        slot_number: Some(payload.slot_number),
        ..Default::default()
    };

    let block = Block::new(header, body);
    validate_reconstructed_block_hash(&block, &payload.block_hash, crypto)?;
    Ok(block)
}

/// Validate that the blob versioned hashes in the `NewPayloadRequest` match
/// the blob commitments in the block's transactions.
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

/// Transform a native-rollup SSZ `NewPayloadRequest` (`stateless_ssz`) into a `Block`.
///
/// Always compiled — used by the EXECUTE precompile path (`ethrex-blockchain`),
/// the L2 advancer, and [`verify_stateless_block`]. Distinct from
/// [`eip8025_new_payload_request_to_block`], which reconstructs from the
/// EIP-8025 guest's `eip8025_ssz` payload (no in-SSZ block access list).
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

    // Convert base_fee_per_gas from [u8; 32] LE uint256 to u64. The helper rejects
    // non-zero upper bytes so a single block maps to a single hash_tree_root (see
    // `base_fee_per_gas_from_le_bytes`).
    let base_fee_per_gas = base_fee_per_gas_from_le_bytes(&payload.base_fee_per_gas)?;

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
        // EIP-7843: reconstruct the slot number carried in the SSZ payload so the
        // computed block hash matches the producer's. Native-rollup blocks are
        // Amsterdam+, where slot_number is always present.
        slot_number: Some(payload.slot_number),
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
        header.block_access_list_hash = Some(bal.compute_hash(crypto));
    }

    Ok(Block::new(header, body))
}

/// Core stateless block validation for the native-rollup EXECUTE path.
///
/// Sole caller: `ethrex-blockchain`'s `verify_stateless_new_payload`
/// (`StatelessExecutor`, the `StatelessValidator` trait impl invoked by the
/// EXECUTE precompile). NOTE: the zkVM guest binaries do **not** call this —
/// they validate via the separate `validate_eip8025_*` path
/// (`eip8025_new_payload_request_to_block`), so changes here do not affect
/// zk-proof output.
///
/// Implements the `verify_stateless_new_payload` logic from execution-specs:
/// reconstruct block → validate versioned hashes → execute statelessly →
/// inject recomputed `burned_fees` → validate the recomputed block access list
/// hash (Amsterdam+) → verify `block_hash`.
///
/// **Always compiled** — no `#[cfg(feature = "eip-8025")]` gate, so the
/// always-compiled `verify_inner` in `ethrex-blockchain` can call it without
/// pulling in the SSZ feature.
pub fn verify_stateless_block(
    new_payload_request: &ethrex_common::types::stateless_ssz::NewPayloadRequest,
    execution_witness: ethrex_common::types::block_execution_witness::ExecutionWitness,
    crypto: Arc<dyn Crypto>,
) -> Result<(), ExecutionError> {
    // ChainConfig is Copy — capture it before execute_blocks consumes execution_witness.
    let chain_config = execution_witness.chain_config;

    // Transform SSZ NewPayloadRequest → Block.
    // Do NOT call block.hash() here — burned_fees is not yet known so any
    // cached value would be stale.
    let block = new_payload_request_to_block(new_payload_request, crypto.as_ref())
        .map_err(|e| ExecutionError::Internal(format!("payload conversion: {e}")))?;

    // Keep block in a fixed-size array so we can reclaim it after execute_blocks
    // (which borrows it as &[Block] without consuming it).
    let blocks = [block];

    // Validate blob versioned hashes (does not touch block.hash())
    validate_versioned_hashes(&blocks[0], new_payload_request.versioned_hashes.iter())?;

    // Execute statelessly — burned_fees and BAL are recomputed from actual execution.
    let result = execute_blocks(
        &blocks,
        execution_witness,
        ELASTICITY_MULTIPLIER,
        |db, _| Ok(Evm::new_for_l1(db.clone(), crypto.clone())),
        crypto.clone(),
    )?;

    // Inject recomputed burned_fees into the header, then check block_hash.
    //
    // Safety: execute_blocks calls initialize_block_header_hashes which
    // populates block.header.hash via OnceCell — but with burned_fees=None
    // (pre-execution value).  into_with_burned_fees() takes ownership, sets
    // burned_fees, and calls OnceCell::take() to clear the stale cache, so
    // the next hash() call reflects the injected value.
    //
    // At Amsterdam (pre-LStar), burned_fees is None both here and in the
    // original header, so the hash is unchanged — no regression on the
    // current path.
    let recomputed_burned_fees = result.burned_fees.first().copied().flatten();
    let recomputed_bal = result.bals.into_iter().next().flatten();
    let [block] = blocks;
    let tx_count = block.body.transactions.len();
    let verified_header = block.header.into_with_burned_fees(recomputed_burned_fees);

    // EIP-7928 (Amsterdam+): validate the recomputed BAL — structural checks
    // (index bounds, size cap) and hash match against header.block_access_list_hash.
    // Pre-Amsterdam blocks produce recomputed_bal = None, so this is a no-op there.
    if let Some(ref bal) = recomputed_bal {
        validate_block_access_list_hash(
            &verified_header,
            &chain_config,
            bal,
            tx_count,
            crypto.as_ref(),
        )
        .map_err(ExecutionError::BlockValidation)?;
    }

    let computed_hash = verified_header.hash();
    let expected_hash =
        ethrex_common::H256::from_slice(&new_payload_request.execution_payload.block_hash);
    if computed_hash != expected_hash {
        return Err(ExecutionError::Internal(format!(
            "block_hash mismatch: expected {expected_hash:?}, got {computed_hash:?}"
        )));
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
        Bytes::copy_from_slice(bytes)
    }

    ethrex_common::types::block_execution_witness::RpcExecutionWitness {
        state: witness.state.iter().map(copy_ssz_bytes).collect(),
        // The specs do not have a `keys` field in the witness. This field
        // is inherited from a legacy debug_executionWitness design.
        // A `keys` field is not currently planned to be included in
        // the specs. It might if there is rough consensus it is valuable
        // for execution witness validation performance.
        keys: Vec::new(),
        codes: witness.codes.iter().map(copy_ssz_bytes).collect(),
        headers: witness.headers.iter().map(copy_ssz_bytes).collect(),
    }
}

/// Validate the canonical input's `ChainConfig` and witness, then reconstruct
/// the `Block` from the Amsterdam `NewPayloadRequest` it carries and execute it
/// statelessly.
#[cfg(feature = "eip-8025")]
pub fn validate_eip8025_canonical_execution(
    stateless_input: CanonicalStatelessInput,
    chain_config: ethrex_common::types::ChainConfig,
    crypto: Arc<dyn Crypto>,
) -> Result<(), ExecutionError> {
    let block_timestamp = stateless_input
        .new_payload_request
        .execution_payload
        .timestamp;
    let block_number = stateless_input
        .new_payload_request
        .execution_payload
        .block_number;
    validate_canonical_chain_config(
        &stateless_input.chain_config,
        &chain_config,
        block_number,
        block_timestamp,
    )?;

    let rpc_witness = canonical_execution_witness_to_rpc(stateless_input.witness);
    // Decode headers once; reused by the chain-linkage check and `into_execution_witness`.
    let decoded_headers = ethrex_common::types::block_execution_witness::decode_witness_headers(
        &rpc_witness.headers,
    )?;
    // EELS `test_validation_headers_non_contiguous_chain`: check chain linkage
    // in input order, before any sort/dedup.
    ethrex_common::types::block_execution_witness::validate_witness_headers_chain(
        &decoded_headers,
        crypto.as_ref(),
    )?;

    let execution_witness = rpc_witness.into_execution_witness(
        chain_config,
        block_number,
        &decoded_headers,
        crypto.as_ref(),
    )?;

    validate_eip8025_amsterdam_execution(
        &stateless_input.new_payload_request,
        execution_witness,
        crypto,
        stateless_input.public_keys,
    )
}

/// Validate `chain_id`, `active_fork.activation`, and `active_fork.blob_schedule`
/// from the prover's `CanonicalChainConfig` against the verifier's `ChainConfig`.
#[cfg(feature = "eip-8025")]
fn validate_canonical_chain_config(
    canonical: &crate::l1::input::CanonicalChainConfig,
    expected: &ethrex_common::types::ChainConfig,
    block_number: u64,
    block_timestamp: u64,
) -> Result<(), ExecutionError> {
    if canonical.chain_id != expected.chain_id {
        return Err(ExecutionError::Internal(format!(
            "chain_id mismatch between canonical input ({}) and chain config ({})",
            canonical.chain_id, expected.chain_id
        )));
    }

    // EELS `validate_chain_config` / `_is_activation_active`: the declared active
    // fork must actually be active for this payload. The activation point must set
    // a block_number or a timestamp, and the payload must be at or past it.
    // `block_number`/`timestamp` are `SszList<u64, MAX_OPTIONAL_FORK_ACTIVATION_VALUES=1>`,
    // i.e. an `Option<u64>` carrying 0 or 1 value.
    let activation = &canonical.active_fork.activation;
    let activation_block_number = activation.block_number.iter().next().copied();
    let activation_timestamp = activation.timestamp.iter().next().copied();
    if activation_block_number.is_none() && activation_timestamp.is_none() {
        return Err(ExecutionError::Internal(
            "fork activation must set block_number or timestamp".to_string(),
        ));
    }
    if let Some(activation_block_number) = activation_block_number
        && block_number < activation_block_number
    {
        return Err(ExecutionError::Internal(format!(
            "ChainConfig active_fork is not active for the target payload: \
             block_number {block_number} precedes activation {activation_block_number}"
        )));
    }
    if let Some(activation_timestamp) = activation_timestamp
        && block_timestamp < activation_timestamp
    {
        return Err(ExecutionError::Internal(format!(
            "ChainConfig active_fork is not active for the target payload: \
             timestamp {block_timestamp} precedes activation {activation_timestamp}"
        )));
    }

    // TODO: `fork` is not compared. EELS and ethrex number forks differently, and
    // the spec stores the fork id for canonical-root determinism rather than
    // verifier cross-checking. The blob-schedule check below is a partial proxy
    // and misses forks with identical blob parameters.

    // Single-entry check is sound because `MAX_BLOB_SCHEDULES_PER_FORK = 1`.
    let canonical_schedule = canonical.active_fork.blob_schedule.iter().next();
    let expected_schedule = expected.get_fork_blob_schedule(block_timestamp);
    match (canonical_schedule, expected_schedule) {
        (Some(c), Some(e)) => {
            if c.target != e.target as u64
                || c.max != e.max as u64
                || c.base_fee_update_fraction != e.base_fee_update_fraction
            {
                return Err(ExecutionError::Internal(format!(
                    "blob_schedule mismatch: canonical \
                     (target={}, max={}, base_fee_update_fraction={}) \
                     vs chain config (target={}, max={}, base_fee_update_fraction={})",
                    c.target,
                    c.max,
                    c.base_fee_update_fraction,
                    e.target,
                    e.max,
                    e.base_fee_update_fraction
                )));
            }
        }
        (Some(_), None) => {
            return Err(ExecutionError::Internal(
                "blob_schedule mismatch: canonical input includes a schedule but \
                 chain config has none at the block's timestamp"
                    .to_string(),
            ));
        }
        (None, Some(_)) => {
            return Err(ExecutionError::Internal(
                "blob_schedule mismatch: canonical input omits the schedule but \
                 chain config has one at the block's timestamp"
                    .to_string(),
            ));
        }
        (None, None) => {}
    }

    Ok(())
}

/// Reconstruct the `Block` from a legacy `NewPayloadRequest` and execute it
/// statelessly against the supplied `ExecutionWitness`.
#[cfg(feature = "eip-8025")]
pub fn validate_eip8025_execution(
    new_payload_request: &ethrex_common::types::eip8025_ssz::NewPayloadRequest,
    execution_witness: ethrex_common::types::block_execution_witness::ExecutionWitness,
    crypto: Arc<dyn Crypto>,
) -> Result<(), ExecutionError> {
    // Transform SSZ NewPayloadRequest → Block
    let block = eip8025_new_payload_request_to_block(new_payload_request, crypto.as_ref())
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
    public_keys: PublicKeysList,
) -> Result<(), ExecutionError> {
    let block = new_payload_request_amsterdam_to_block(new_payload_request, crypto.as_ref())
        .map_err(|e| ExecutionError::Internal(format!("payload conversion: {e}")))?;

    validate_versioned_hashes(&block, new_payload_request.versioned_hashes.iter())?;

    if public_keys.len() != block.body.transactions.len() {
        return Err(ExecutionError::Internal(format!(
            "Found {} public keys in the stateless input, but there are {} transactions",
            public_keys.len(),
            block.body.transactions.len()
        )));
    }
    for (public_key, tx) in public_keys.iter().zip(block.body.transactions.iter()) {
        // SSZ decode fixes the length at 65; uncompressed secp256k1 is 0x04 || X || Y.
        let pk_bytes: &[u8] = public_key;
        if pk_bytes[0] != 0x04 {
            return Err(ExecutionError::Internal(
                "Stateless input public key is not a 65-byte uncompressed secp256k1 key"
                    .to_string(),
            ));
        }
        let derived = Address::from_slice(&keccak(&pk_bytes[1..])[12..]);
        let recovered = tx.sender(crypto.as_ref()).map_err(|e| {
            ExecutionError::Internal(format!("failed to recover transaction sender: {e}"))
        })?;
        if recovered != derived {
            return Err(ExecutionError::Internal(
                "Stateless input public key does not match recovered transaction sender"
                    .to_string(),
            ));
        }
    }

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
