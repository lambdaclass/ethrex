//! Stateless block validation — shared across EXECUTE precompile, EIP-8025, and zkVM guests.
//!
//! The core function `verify_stateless_new_payload` implements the same logic as
//! `verify_stateless_new_payload` from the execution-specs `projects/zkevm` branch.
//! It is called from three entry points:
//! - The EXECUTE precompile (via the `StatelessValidator` trait)
//! - The EIP-8025 RPC proof generation flow
//! - The zkVM guest program

use std::sync::Arc;

use ethrex_common::types::block_execution_witness::ExecutionWitness;
use ethrex_common::types::stateless_ssz::{
    NewPayloadRequest, SszChainConfig, SszStatelessInput, SszStatelessValidationResult,
};
use ethrex_crypto::Crypto;
use ethrex_guest_program::common::{ExecutionError, execute_blocks};
use ethrex_guest_program::l1::new_payload_request_to_block;
use ssz::SszEncode;
use ssz_merkle::HashTreeRoot;

/// Result of `verify_stateless_new_payload`.
pub struct StatelessValidationResult {
    pub new_payload_request_root: [u8; 32],
    pub successful_validation: bool,
    pub chain_config: SszChainConfig,
}

/// Core stateless validation function matching the execution-specs definition.
///
/// Takes a `NewPayloadRequest`, `ExecutionWitness`, and `ChainConfig`, and:
/// 1. Computes `hash_tree_root` of the `NewPayloadRequest`
/// 2. Converts the payload to a `Block`
/// 3. Executes the block statelessly
/// 4. Returns the validation result
///
/// This is the function that all entry points (EXECUTE precompile, EIP-8025,
/// zkVM guest) should call.
pub fn verify_stateless_new_payload(
    new_payload_request: &NewPayloadRequest,
    execution_witness: ExecutionWitness,
    chain_config: &SszChainConfig,
    crypto: Arc<dyn Crypto>,
) -> StatelessValidationResult {
    let request_root = new_payload_request.hash_tree_root();

    let successful = match verify_inner(new_payload_request, execution_witness, crypto) {
        Ok(()) => {
            tracing::info!("verify_stateless_new_payload: validation succeeded");
            true
        }
        Err(e) => {
            tracing::error!("verify_stateless_new_payload: validation failed: {e}");
            false
        }
    };

    StatelessValidationResult {
        new_payload_request_root: request_root,
        successful_validation: successful,
        chain_config: chain_config.clone(),
    }
}

fn verify_inner(
    new_payload_request: &NewPayloadRequest,
    execution_witness: ExecutionWitness,
    crypto: Arc<dyn Crypto>,
) -> Result<(), ExecutionError> {
    use ethrex_common::types::ELASTICITY_MULTIPLIER;
    use ethrex_vm::Evm;

    let block = new_payload_request_to_block(new_payload_request, crypto.as_ref())
        .map_err(|e| ExecutionError::Internal(format!("payload conversion: {e}")))?;

    // Validate block_hash
    let computed_hash = block.hash();
    let expected_hash =
        ethrex_common::H256::from_slice(&new_payload_request.execution_payload.block_hash);
    if computed_hash != expected_hash {
        tracing::error!(
            "block_hash mismatch details:\n  parent_hash: {:?}\n  coinbase: {:?}\n  state_root: {:?}\n  tx_root: {:?}\n  receipts_root: {:?}\n  number: {}\n  gas_limit: {}\n  gas_used: {}\n  timestamp: {}\n  base_fee: {:?}\n  withdrawals_root: {:?}\n  blob_gas_used: {:?}\n  excess_blob_gas: {:?}\n  parent_beacon_block_root: {:?}\n  requests_hash: {:?}\n  extra_data: {:?}",
            block.header.parent_hash,
            block.header.coinbase,
            block.header.state_root,
            block.header.transactions_root,
            block.header.receipts_root,
            block.header.number,
            block.header.gas_limit,
            block.header.gas_used,
            block.header.timestamp,
            block.header.base_fee_per_gas,
            block.header.withdrawals_root,
            block.header.blob_gas_used,
            block.header.excess_blob_gas,
            block.header.parent_beacon_block_root,
            block.header.requests_hash,
            block.header.extra_data,
        );
        tracing::error!(
            "  ommers_hash: {:?}\n  difficulty: {:?}\n  nonce: {}\n  prev_randao: {:?}\n  logs_bloom len: {}",
            block.header.ommers_hash,
            block.header.difficulty,
            block.header.nonce,
            block.header.prev_randao,
            block.header.logs_bloom.0.len(),
        );
        return Err(ExecutionError::Internal(format!(
            "block_hash mismatch: expected {expected_hash:?}, got {computed_hash:?}"
        )));
    }

    // Validate blob versioned hashes
    validate_versioned_hashes(&block, new_payload_request)?;

    // Execute statelessly
    let _result = execute_blocks(
        &[block],
        execution_witness,
        ELASTICITY_MULTIPLIER,
        |db, _| Ok(Evm::new_for_l1(db.clone(), crypto.clone())),
        crypto.clone(),
    )?;

    Ok(())
}

fn validate_versioned_hashes(
    block: &ethrex_common::types::Block,
    req: &NewPayloadRequest,
) -> Result<(), ExecutionError> {
    use ethrex_common::H256;

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
            "versioned hashes mismatch".to_string(),
        ));
    }

    Ok(())
}

/// Implementation of the `StatelessValidator` trait for the EXECUTE precompile.
///
/// Deserializes SSZ `StatelessInput`, calls `verify_stateless_new_payload`,
/// and serializes the result back to SSZ bytes.
pub struct StatelessExecutor {
    pub crypto: Arc<dyn Crypto>,
}

impl ethrex_vm::StatelessValidator for StatelessExecutor {
    fn verify(&self, input: &[u8]) -> Result<Vec<u8>, ethrex_vm::VMError> {
        use ethrex_vm::{InternalError, VMError};
        use ssz::SszDecode;

        // Deserialize SSZ input
        tracing::debug!(
            "StatelessExecutor: deserializing {} bytes of SSZ input",
            input.len()
        );
        let stateless_input = SszStatelessInput::from_ssz_bytes(input).map_err(|e| {
            tracing::error!("StatelessExecutor: SSZ decode failed: {e}");
            VMError::Internal(InternalError::Custom(format!("SSZ decode: {e}")))
        })?;
        tracing::info!(
            "StatelessExecutor: decoded SSZ input - block_number={}, gas_used={}, chain_id={}, witness_headers={}, witness_state_nodes={}, witness_codes={}",
            stateless_input
                .new_payload_request
                .execution_payload
                .block_number,
            stateless_input
                .new_payload_request
                .execution_payload
                .gas_used,
            stateless_input.chain_config.chain_id,
            stateless_input.witness.headers.len(),
            stateless_input.witness.state.len(),
            stateless_input.witness.codes.len(),
        );

        // Derive first_block_number and initial_state_root from witness headers
        let (first_block_number, initial_state_root) = {
            use ethrex_common::types::BlockHeader;
            use ethrex_rlp::decode::RLPDecode;
            let headers = stateless_input.witness.headers_as_vecs();
            if headers.is_empty() {
                return Err(VMError::Internal(InternalError::Custom(
                    "witness contains no headers".to_string(),
                )));
            }
            let last_header = BlockHeader::decode(headers.last().expect("checked non-empty"))
                .map_err(|e| {
                    VMError::Internal(InternalError::Custom(format!("header decode: {e}")))
                })?;
            (last_header.number + 1, last_header.state_root)
        };

        // Convert SszExecutionWitness → internal ExecutionWitness
        let execution_witness = ssz_witness_to_internal(
            &stateless_input.witness,
            &stateless_input.chain_config,
            first_block_number,
            initial_state_root,
        )
        .map_err(|e| {
            VMError::Internal(InternalError::Custom(format!("witness conversion: {e}")))
        })?;

        let result = verify_stateless_new_payload(
            &stateless_input.new_payload_request,
            execution_witness,
            &stateless_input.chain_config,
            self.crypto.clone(),
        );

        // Serialize result to SSZ
        let ssz_result = SszStatelessValidationResult {
            new_payload_request_root: result.new_payload_request_root,
            successful_validation: result.successful_validation,
            chain_config: result.chain_config,
        };
        let mut buf = Vec::new();
        ssz_result.ssz_append(&mut buf);
        Ok(buf)
    }
}

/// Convert SSZ execution witness to internal format.
///
/// The SSZ `state` field contains raw trie-node preimages (same format as
/// `RpcExecutionWitness.state`). We reconstruct embedded trie structures
/// from these flat node bytes.
fn ssz_witness_to_internal(
    ssz_witness: &ethrex_common::types::stateless_ssz::SszExecutionWitness,
    chain_config: &SszChainConfig,
    first_block_number: u64,
    initial_state_root: ethrex_common::H256,
) -> Result<ExecutionWitness, String> {
    use ethrex_common::H256;
    use ethrex_common::types::ChainConfig;
    use ethrex_common::utils::keccak;
    use ethrex_rlp::decode::RLPDecode;
    use ethrex_trie::{EMPTY_TRIE_HASH, Node, NodeRef, Trie};
    use std::collections::BTreeMap;

    let codes = ssz_witness.codes_as_vecs();
    let block_headers_bytes = ssz_witness.headers_as_vecs();
    let state_bytes = ssz_witness.state_as_vecs();

    // Build node map: hash → decoded Node
    let nodes: BTreeMap<H256, Node> = state_bytes
        .into_iter()
        .filter_map(|b| {
            if b == [0x80] {
                return None; // skip null nodes
            }
            let hash = keccak(&b);
            Some(Node::decode(&b).map(|node| (hash, node)))
        })
        .collect::<Result<_, _>>()
        .map_err(|e| format!("node decode: {e}"))?;

    // Get state trie root and embed subtrie
    let state_trie_root = if let NodeRef::Node(root, _) =
        Trie::get_embedded_root(&nodes, initial_state_root)
            .map_err(|e| format!("state trie root: {e}"))?
    {
        Some((*root).clone())
    } else {
        None
    };

    // Walk state trie to find account storage roots
    let mut storage_trie_roots = BTreeMap::new();
    if let Some(ref root_node) = state_trie_root {
        // Collect all account leaf nodes from the state trie
        let accounts = collect_account_storage_roots(root_node, &nodes);
        for (hashed_address, storage_root_hash) in accounts {
            if storage_root_hash == *EMPTY_TRIE_HASH || !nodes.contains_key(&storage_root_hash) {
                continue;
            }
            if let Ok(NodeRef::Node(node, _)) = Trie::get_embedded_root(&nodes, storage_root_hash) {
                storage_trie_roots.insert(hashed_address, (*node).clone());
            }
        }
    }

    Ok(ExecutionWitness {
        codes,
        block_headers_bytes,
        first_block_number,
        // The spec's ChainConfig only carries chain_id; fork rules are
        // implicit.  Stateless validation always runs at the latest fork
        // (Amsterdam), so activate all prior forks at timestamp/block 0.
        chain_config: ChainConfig {
            chain_id: chain_config.chain_id,
            homestead_block: Some(0),
            eip150_block: Some(0),
            eip155_block: Some(0),
            eip158_block: Some(0),
            byzantium_block: Some(0),
            constantinople_block: Some(0),
            petersburg_block: Some(0),
            istanbul_block: Some(0),
            berlin_block: Some(0),
            london_block: Some(0),
            terminal_total_difficulty: Some(0),
            terminal_total_difficulty_passed: true,
            shanghai_time: Some(0),
            cancun_time: Some(0),
            prague_time: Some(0),
            ..Default::default()
        },
        state_trie_root,
        storage_trie_roots,
    })
}

/// Walk the state trie and collect (hashed_address, storage_root) pairs from leaf nodes.
fn collect_account_storage_roots(
    root: &ethrex_trie::Node,
    nodes: &std::collections::BTreeMap<ethrex_common::H256, ethrex_trie::Node>,
) -> Vec<(ethrex_common::H256, ethrex_common::H256)> {
    use ethrex_common::types::block_execution_witness::collect_accounts_from_trie;
    use ethrex_crypto::NativeCrypto;
    collect_accounts_from_trie(root, nodes, &NativeCrypto)
}
