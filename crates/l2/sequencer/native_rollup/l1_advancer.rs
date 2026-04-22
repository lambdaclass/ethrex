//! NativeL1Advancer GenServer — reads produced L2 blocks from the Store,
//! generates an execution witness, and submits them to the NativeRollup.sol
//! contract via advance().
//!
//! The new advance() call sends:
//!   advance(uint256 l1MessagesCount, bytes sszStatelessInput, bytes32 newBlockHash, bytes32 newStateRoot)
//!
//! where sszStatelessInput is the SSZ-encoded StatelessInput containing:
//!   NewPayloadRequest, ExecutionWitness, ChainConfig, and public_keys.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use ethrex_blockchain::Blockchain;
use ethrex_common::types::{Block, BlockHeader, TxType};
use ethrex_common::{Address, H256, U256};
use ethrex_crypto::NativeCrypto;
use ethrex_l2_common::calldata::Value;
use ethrex_l2_rpc::signer::Signer;
use ethrex_l2_sdk::{
    build_generic_tx, calldata::encode_calldata, get_native_rollup_block_number,
    send_tx_bump_gas_exponential_backoff,
};
use ethrex_rlp::encode::RLPEncode;
use ethrex_rpc::clients::Overrides;
use ethrex_rpc::clients::eth::EthClient;
use ethrex_storage::Store;
use spawned_concurrency::{
    actor,
    error::ActorError,
    protocol,
    tasks::{Actor, ActorRef, ActorStart as _, Context, Handler, send_after},
};
use tracing::{debug, error, info};

const ADVANCE_FUNCTION_SIGNATURE: &str = "advance(uint256,bytes,bytes32,bytes32)";

#[protocol]
pub trait NativeL1AdvancerProtocol: Send + Sync {
    fn advance(&self) -> Result<(), ActorError>;
}

#[derive(Debug, thiserror::Error)]
pub enum NativeL1AdvancerError {
    #[error("EthClient error: {0}")]
    EthClient(#[from] ethrex_rpc::clients::eth::errors::EthClientError),
    #[error("Encoding error: {0}")]
    Encoding(String),
    #[error("Internal error: {0}")]
    Internal(#[from] spawned_concurrency::error::ActorError),
    #[error("Signer error: {0}")]
    Signer(String),
    #[error("Store error: {0}")]
    Store(#[from] ethrex_storage::error::StoreError),
    #[error("Chain error: {0}")]
    Chain(#[from] ethrex_blockchain::error::ChainError),
}

pub struct NativeL1Advancer {
    eth_client: EthClient,
    contract_address: Address,
    signer: Signer,
    store: Store,
    blockchain: Arc<Blockchain>,
    relayer_address: Address,
    advance_interval_ms: u64,
}

impl NativeL1Advancer {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        eth_client: EthClient,
        contract_address: Address,
        signer: Signer,
        store: Store,
        blockchain: Arc<Blockchain>,
        relayer_address: Address,
        advance_interval_ms: u64,
    ) -> Self {
        Self {
            eth_client,
            contract_address,
            signer,
            store,
            blockchain,
            relayer_address,
            advance_interval_ms,
        }
    }

    /// Determine the next block to advance and submit it to L1.
    async fn advance_next_block(&mut self) -> Result<(), NativeL1AdvancerError> {
        // 1. Query the on-chain block number from NativeRollup.sol
        let on_chain_block_number =
            get_native_rollup_block_number(&self.eth_client, self.contract_address).await?;

        let next_block = on_chain_block_number + 1;

        // 2. Fetch block from Store
        let block_header = match self.store.get_block_header(next_block)? {
            Some(h) => h,
            None => {
                debug!(
                    "NativeL1Advancer: block {} not produced yet, skipping",
                    next_block
                );
                return Ok(());
            }
        };

        let block_body = match self.store.get_block_body(next_block).await? {
            Some(b) => b,
            None => {
                debug!(
                    "NativeL1Advancer: block {} body not found, skipping",
                    next_block
                );
                return Ok(());
            }
        };

        let block = Block {
            header: block_header.clone(),
            body: block_body.clone(),
        };

        // 3. Generate execution witness.
        //    The parent_beacon_block_root carries the L1 messages Merkle root.
        //    The EIP-4788 system contract writes it during block processing,
        //    so the witness must include the beacon roots contract state.
        let witness = self
            .blockchain
            .generate_witness_for_blocks(&[block])
            .await?;

        // 4. Count L1 messages by counting relayer txs in the block
        let l1_messages_count: u64 = block_body
            .transactions
            .iter()
            .filter(|tx| tx.sender(&NativeCrypto).ok() == Some(self.relayer_address))
            .count()
            .try_into()
            .map_err(|_| NativeL1AdvancerError::Encoding("l1 messages count overflow".into()))?;

        // 5. Build SSZ StatelessInput
        let ssz_input = build_ssz_stateless_input(&block_header, &block_body, &witness)
            .map_err(|e| NativeL1AdvancerError::Encoding(format!("SSZ encoding: {e}")))?;

        // Debug: verify SSZ round-trip locally before sending
        {
            use ethrex_common::types::stateless_ssz::SszStatelessInput;
            use libssz::SszDecode;
            let decoded = SszStatelessInput::from_ssz_bytes(&ssz_input)
                .map_err(|e| NativeL1AdvancerError::Encoding(format!("SSZ decode check: {e}")))?;
            let reconstructed = ethrex_guest_program::l1::new_payload_request_to_block(
                &decoded.new_payload_request,
                &NativeCrypto,
            )
            .map_err(|e| NativeL1AdvancerError::Encoding(format!("block reconstruction: {e}")))?;
            let original_hash = block_header.compute_block_hash(&NativeCrypto);
            let reconstructed_hash = reconstructed.hash();
            if original_hash != reconstructed_hash {
                error!(
                    "SSZ round-trip hash mismatch! original={:?} reconstructed={:?}",
                    original_hash, reconstructed_hash
                );
                // Log all differing fields
                if block_header.parent_hash != reconstructed.header.parent_hash {
                    error!(
                        "  parent_hash: {:?} vs {:?}",
                        block_header.parent_hash, reconstructed.header.parent_hash
                    );
                }
                if block_header.ommers_hash != reconstructed.header.ommers_hash {
                    error!(
                        "  ommers_hash: {:?} vs {:?}",
                        block_header.ommers_hash, reconstructed.header.ommers_hash
                    );
                }
                if block_header.state_root != reconstructed.header.state_root {
                    error!(
                        "  state_root: {:?} vs {:?}",
                        block_header.state_root, reconstructed.header.state_root
                    );
                }
                if block_header.transactions_root != reconstructed.header.transactions_root {
                    error!(
                        "  transactions_root: {:?} vs {:?}",
                        block_header.transactions_root, reconstructed.header.transactions_root
                    );
                }
                if block_header.receipts_root != reconstructed.header.receipts_root {
                    error!(
                        "  receipts_root: {:?} vs {:?}",
                        block_header.receipts_root, reconstructed.header.receipts_root
                    );
                }
                if block_header.logs_bloom != reconstructed.header.logs_bloom {
                    error!("  logs_bloom differs");
                }
                if block_header.base_fee_per_gas != reconstructed.header.base_fee_per_gas {
                    error!(
                        "  base_fee: {:?} vs {:?}",
                        block_header.base_fee_per_gas, reconstructed.header.base_fee_per_gas
                    );
                }
                if block_header.withdrawals_root != reconstructed.header.withdrawals_root {
                    error!(
                        "  withdrawals_root: {:?} vs {:?}",
                        block_header.withdrawals_root, reconstructed.header.withdrawals_root
                    );
                }
                if block_header.blob_gas_used != reconstructed.header.blob_gas_used {
                    error!(
                        "  blob_gas_used: {:?} vs {:?}",
                        block_header.blob_gas_used, reconstructed.header.blob_gas_used
                    );
                }
                if block_header.excess_blob_gas != reconstructed.header.excess_blob_gas {
                    error!(
                        "  excess_blob_gas: {:?} vs {:?}",
                        block_header.excess_blob_gas, reconstructed.header.excess_blob_gas
                    );
                }
                if block_header.parent_beacon_block_root
                    != reconstructed.header.parent_beacon_block_root
                {
                    error!(
                        "  parent_beacon_block_root: {:?} vs {:?}",
                        block_header.parent_beacon_block_root,
                        reconstructed.header.parent_beacon_block_root
                    );
                }
                if block_header.requests_hash != reconstructed.header.requests_hash {
                    error!(
                        "  requests_hash: {:?} vs {:?}",
                        block_header.requests_hash, reconstructed.header.requests_hash
                    );
                }
                if block_header.extra_data != reconstructed.header.extra_data {
                    error!(
                        "  extra_data: {:?} vs {:?}",
                        block_header.extra_data, reconstructed.header.extra_data
                    );
                }
                return Err(NativeL1AdvancerError::Encoding(format!(
                    "SSZ round-trip hash mismatch: original={original_hash:?} reconstructed={reconstructed_hash:?}"
                )));
            }
            info!("SSZ round-trip OK: block_hash={:?}", original_hash);
        }

        // 6. Send advance() tx
        let tx_hash = self
            .send_advance(&block_header, &ssz_input, l1_messages_count)
            .await?;

        info!(
            "NativeL1Advancer: advanced block {} on L1 (state_root={:?}, l1_msgs={}, tx={:?})",
            next_block, block_header.state_root, l1_messages_count, tx_hash
        );

        Ok(())
    }

    /// Build and send the advance() transaction to NativeRollup.sol.
    async fn send_advance(
        &self,
        header: &BlockHeader,
        ssz_input: &[u8],
        l1_messages_count: u64,
    ) -> Result<H256, NativeL1AdvancerError> {
        let calldata = encode_calldata(
            ADVANCE_FUNCTION_SIGNATURE,
            &[
                Value::Uint(U256::from(l1_messages_count)),
                Value::Bytes(Bytes::from(ssz_input.to_vec())),
                Value::FixedBytes(Bytes::from(
                    header.compute_block_hash(&NativeCrypto).as_bytes().to_vec(),
                )),
                Value::FixedBytes(Bytes::from(header.state_root.as_bytes().to_vec())),
            ],
        )
        .map_err(|e| NativeL1AdvancerError::Encoding(e.to_string()))?;

        let tx = build_generic_tx(
            &self.eth_client,
            TxType::EIP1559,
            self.contract_address,
            self.signer.address(),
            Bytes::from(calldata),
            Overrides {
                from: Some(self.signer.address()),
                ..Default::default()
            },
        )
        .await
        .map_err(NativeL1AdvancerError::EthClient)?;

        let tx_hash = send_tx_bump_gas_exponential_backoff(&self.eth_client, tx, &self.signer)
            .await
            .map_err(NativeL1AdvancerError::EthClient)?;

        Ok(tx_hash)
    }
}

/// Build SSZ-encoded StatelessInput from block data and execution witness.
///
/// Converts internal types to SSZ containers and serializes the full
/// `StatelessInput` structure that the EXECUTE precompile expects.
pub fn build_ssz_stateless_input(
    header: &BlockHeader,
    body: &ethrex_common::types::BlockBody,
    witness: &ethrex_common::types::block_execution_witness::ExecutionWitness,
) -> Result<Vec<u8>, String> {
    use ethrex_common::types::stateless_ssz::*;
    use ethrex_rlp::encode::RLPEncode;
    use libssz::SszEncode;
    use libssz_types::SszList;

    // 1. Convert Block → SSZ ExecutionPayload
    let transactions: Vec<SszList<u8, 1_073_741_824>> = body
        .transactions
        .iter()
        .map(|tx| {
            let encoded = tx.encode_canonical_to_vec();
            let mut list = SszList::new();
            for byte in encoded {
                list.push(byte);
            }
            list
        })
        .collect();
    let mut ssz_transactions = SszList::new();
    for tx in transactions {
        ssz_transactions.push(tx);
    }

    let ssz_withdrawals = SszList::new(); // Empty for L2

    // base_fee_per_gas as LE uint256
    let mut base_fee_bytes = [0u8; 32];
    if let Some(base_fee) = header.base_fee_per_gas {
        base_fee_bytes[..8].copy_from_slice(&base_fee.to_le_bytes());
    }

    // logs_bloom as SszVector<u8, 256>
    let bloom_vec: Vec<u8> = header.logs_bloom.0.to_vec();
    let logs_bloom: LogsBloom = bloom_vec
        .try_into()
        .map_err(|_| "logs_bloom conversion failed")?;

    // extra_data
    let mut extra_data = SszList::new();
    for byte in header.extra_data.iter() {
        extra_data.push(*byte);
    }

    // block_hash
    let block_hash = header.compute_block_hash(&NativeCrypto);

    let execution_payload = ExecutionPayload {
        parent_hash: header.parent_hash.0,
        fee_recipient: Bytes20(header.coinbase.0),
        state_root: header.state_root.0,
        receipts_root: header.receipts_root.0,
        logs_bloom,
        prev_randao: header.prev_randao.0,
        block_number: header.number,
        gas_limit: header.gas_limit,
        gas_used: header.gas_used,
        timestamp: header.timestamp,
        extra_data,
        base_fee_per_gas: base_fee_bytes,
        block_hash: block_hash.0,
        transactions: ssz_transactions,
        withdrawals: ssz_withdrawals,
        blob_gas_used: header.blob_gas_used.unwrap_or(0),
        excess_blob_gas: header.excess_blob_gas.unwrap_or(0),
        deposit_requests: SszList::new(),
        withdrawal_requests: SszList::new(),
        consolidation_requests: SszList::new(),
    };

    // 2. Build SSZ NewPayloadRequest
    let parent_beacon_block_root = header
        .parent_beacon_block_root
        .map(|h| h.0)
        .unwrap_or([0u8; 32]);

    let new_payload_request = NewPayloadRequest {
        execution_payload,
        versioned_hashes: SszList::new(), // Empty for L2
        parent_beacon_block_root,
        execution_requests: SszList::new(), // Empty for L2
    };

    // 3. Convert internal ExecutionWitness → SSZ ExecutionWitness
    let ssz_witness = internal_witness_to_ssz(witness)?;

    // 4. Assemble StatelessInput
    let stateless_input = SszStatelessInput {
        new_payload_request,
        witness: ssz_witness,
        chain_config: SszChainConfig {
            chain_id: witness.chain_config.chain_id,
        },
        public_keys: SszList::new(), // Empty for now
    };

    // 5. Serialize to SSZ bytes
    let mut buf = Vec::new();
    stateless_input.ssz_append(&mut buf);
    Ok(buf)
}

/// Convert internal ExecutionWitness to SSZ format.
///
/// The internal witness has embedded trie structures. The SSZ format
/// needs flat preimage bytes. We extract the raw node bytes from the
/// trie nodes, codes, and headers.
fn internal_witness_to_ssz(
    witness: &ethrex_common::types::block_execution_witness::ExecutionWitness,
) -> Result<ethrex_common::types::stateless_ssz::SszExecutionWitness, String> {
    use ethrex_common::types::stateless_ssz::SszExecutionWitness;
    use ethrex_rlp::encode::RLPEncode;
    use libssz_types::SszList;

    // State: encode trie nodes back to their RLP preimage bytes.
    // The internal witness stores them as embedded Node structures.
    // We need to flatten them back to raw bytes for SSZ.
    let mut state_nodes = SszList::new();
    if let Some(ref root_node) = witness.state_trie_root {
        let mut preimages = Vec::new();
        collect_node_preimages(root_node, &mut preimages);
        for preimage in preimages {
            let mut node_list = SszList::new();
            for byte in preimage {
                node_list.push(byte);
            }
            state_nodes.push(node_list);
        }
    }

    // Also add storage trie node preimages
    for (_addr, storage_root) in &witness.storage_trie_roots {
        let mut preimages = Vec::new();
        collect_node_preimages(storage_root, &mut preimages);
        for preimage in preimages {
            let mut node_list = SszList::new();
            for byte in preimage {
                node_list.push(byte);
            }
            state_nodes.push(node_list);
        }
    }

    // Codes
    let mut codes = SszList::new();
    for code in &witness.codes {
        let mut code_list = SszList::new();
        for byte in code {
            code_list.push(*byte);
        }
        codes.push(code_list);
    }

    // Headers
    let mut headers = SszList::new();
    for header_bytes in &witness.block_headers_bytes {
        let mut header_list = SszList::new();
        for byte in header_bytes {
            header_list.push(*byte);
        }
        headers.push(header_list);
    }

    Ok(SszExecutionWitness {
        state: state_nodes,
        codes,
        headers,
    })
}

/// Recursively collect RLP-encoded preimages from a trie Node.
fn collect_node_preimages(node: &ethrex_trie::Node, preimages: &mut Vec<Vec<u8>>) {
    use ethrex_rlp::encode::RLPEncode;
    // Encode the node to its RLP representation
    let encoded = node.encode_to_vec();
    preimages.push(encoded);

    // Recurse into children
    match node {
        ethrex_trie::Node::Branch(branch) => {
            for choice in &branch.choices {
                if let ethrex_trie::NodeRef::Node(child, _) = choice {
                    collect_node_preimages(child, preimages);
                }
            }
        }
        ethrex_trie::Node::Extension(ext) => {
            if let ethrex_trie::NodeRef::Node(child, _) = &ext.child {
                collect_node_preimages(child, preimages);
            }
        }
        ethrex_trie::Node::Leaf(_) => {} // No children
    }
}

#[actor(protocol = NativeL1AdvancerProtocol)]
impl NativeL1Advancer {
    pub fn spawn(
        eth_client: EthClient,
        contract_address: Address,
        signer: Signer,
        store: Store,
        blockchain: Arc<Blockchain>,
        relayer_address: Address,
        advance_interval_ms: u64,
    ) -> ActorRef<NativeL1Advancer> {
        let advancer = Self::new(
            eth_client,
            contract_address,
            signer,
            store,
            blockchain,
            relayer_address,
            advance_interval_ms,
        );
        advancer.start()
    }

    #[started]
    async fn started(&mut self, ctx: &Context<Self>) {
        let _ = ctx
            .send(native_l1_advancer_protocol::Advance)
            .inspect_err(|e| error!("NativeL1Advancer: failed to send initial Advance: {e}"));
    }

    #[send_handler]
    async fn handle_advance(
        &mut self,
        _msg: native_l1_advancer_protocol::Advance,
        ctx: &Context<Self>,
    ) {
        let _ = self
            .advance_next_block()
            .await
            .inspect_err(|e| error!("NativeL1Advancer error: {e}"));

        send_after(
            Duration::from_millis(self.advance_interval_ms),
            ctx.clone(),
            native_l1_advancer_protocol::Advance,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::H256;
    use ethrex_common::types::block_execution_witness::ExecutionWitness;
    use ethrex_common::types::{Block, BlockBody, BlockHeader};
    use ethrex_crypto::NativeCrypto;
    use std::sync::Arc;

    /// Test that Block → SSZ → Block round-trip preserves the block hash.
    ///
    /// This catches mismatches in field encoding (wrong defaults, missing
    /// fields, different formats) that cause `verify_stateless_new_payload`
    /// to reject the block with "block_hash mismatch".
    #[test]
    fn test_block_ssz_roundtrip_preserves_hash() {
        let crypto = Arc::new(NativeCrypto);

        // Build a minimal L2-like block (Shanghai, no txs, empty body)
        let parent_hash = H256::zero();
        let header = BlockHeader {
            parent_hash,
            ommers_hash: *ethrex_common::constants::DEFAULT_OMMERS_HASH,
            coinbase: ethrex_common::Address::zero(),
            state_root: H256::from_low_u64_be(0x1234),
            transactions_root: ethrex_common::types::compute_transactions_root(&[], &NativeCrypto),
            receipts_root: ethrex_common::types::compute_receipts_root(&[], &NativeCrypto),
            logs_bloom: Default::default(),
            number: 1,
            gas_limit: 30_000_000,
            gas_used: 0,
            timestamp: 1000,
            base_fee_per_gas: Some(7),
            prev_randao: H256::zero(),
            extra_data: bytes::Bytes::new(),
            // Shanghai fields
            withdrawals_root: Some(ethrex_common::types::compute_withdrawals_root(
                &[],
                &NativeCrypto,
            )),
            // Cancun fields (present in L2 blocks even though chain is Shanghai)
            blob_gas_used: Some(0),
            excess_blob_gas: Some(0),
            parent_beacon_block_root: Some(H256::zero()),
            // Prague fields
            requests_hash: Some(ethrex_common::types::requests::compute_requests_hash(&[])),
            ..Default::default()
        };
        let body = BlockBody {
            transactions: vec![],
            ommers: vec![],
            withdrawals: Some(vec![]),
        };
        let block = Block::new(header.clone(), body.clone());
        let original_hash = block.hash();

        // Build a minimal witness (empty, just enough for the SSZ encoding)
        let witness = ExecutionWitness {
            codes: vec![],
            block_headers_bytes: vec![],
            first_block_number: 1,
            chain_config: ethrex_common::types::ChainConfig {
                chain_id: 1,
                ..Default::default()
            },
            state_trie_root: None,
            storage_trie_roots: Default::default(),
        };

        // Block → SSZ
        let ssz_bytes = build_ssz_stateless_input(&header, &body, &witness)
            .expect("SSZ encoding should succeed");

        // SSZ → deserialize → reconstruct Block
        use ethrex_common::types::stateless_ssz::SszStatelessInput;
        use libssz::SszDecode;
        let input = SszStatelessInput::from_ssz_bytes(&ssz_bytes)
            .expect("SSZ deserialization should succeed");

        let reconstructed_block = ethrex_guest_program::l1::new_payload_request_to_block(
            &input.new_payload_request,
            crypto.as_ref(),
        )
        .expect("Block reconstruction should succeed");

        let reconstructed_hash = reconstructed_block.hash();

        // Compare field by field to make debugging easier
        assert_eq!(
            header.parent_hash, reconstructed_block.header.parent_hash,
            "parent_hash mismatch"
        );
        assert_eq!(
            header.coinbase, reconstructed_block.header.coinbase,
            "coinbase mismatch"
        );
        assert_eq!(
            header.state_root, reconstructed_block.header.state_root,
            "state_root mismatch"
        );
        assert_eq!(
            header.transactions_root, reconstructed_block.header.transactions_root,
            "transactions_root mismatch"
        );
        assert_eq!(
            header.receipts_root, reconstructed_block.header.receipts_root,
            "receipts_root mismatch"
        );
        assert_eq!(
            header.number, reconstructed_block.header.number,
            "number mismatch"
        );
        assert_eq!(
            header.gas_limit, reconstructed_block.header.gas_limit,
            "gas_limit mismatch"
        );
        assert_eq!(
            header.gas_used, reconstructed_block.header.gas_used,
            "gas_used mismatch"
        );
        assert_eq!(
            header.timestamp, reconstructed_block.header.timestamp,
            "timestamp mismatch"
        );
        assert_eq!(
            header.base_fee_per_gas, reconstructed_block.header.base_fee_per_gas,
            "base_fee_per_gas mismatch"
        );
        assert_eq!(
            header.prev_randao, reconstructed_block.header.prev_randao,
            "prev_randao mismatch"
        );
        assert_eq!(
            header.extra_data, reconstructed_block.header.extra_data,
            "extra_data mismatch"
        );
        assert_eq!(
            header.withdrawals_root, reconstructed_block.header.withdrawals_root,
            "withdrawals_root mismatch"
        );
        assert_eq!(
            header.blob_gas_used, reconstructed_block.header.blob_gas_used,
            "blob_gas_used mismatch"
        );
        assert_eq!(
            header.excess_blob_gas, reconstructed_block.header.excess_blob_gas,
            "excess_blob_gas mismatch"
        );
        assert_eq!(
            header.parent_beacon_block_root, reconstructed_block.header.parent_beacon_block_root,
            "parent_beacon_block_root mismatch"
        );
        assert_eq!(
            header.requests_hash, reconstructed_block.header.requests_hash,
            "requests_hash mismatch"
        );
        assert_eq!(
            header.logs_bloom, reconstructed_block.header.logs_bloom,
            "logs_bloom mismatch"
        );
        assert_eq!(
            header.difficulty, reconstructed_block.header.difficulty,
            "difficulty mismatch"
        );
        assert_eq!(
            header.nonce, reconstructed_block.header.nonce,
            "nonce mismatch"
        );
        assert_eq!(
            header.ommers_hash, reconstructed_block.header.ommers_hash,
            "ommers_hash mismatch"
        );

        // The final check: block hash must match
        assert_eq!(
            original_hash, reconstructed_hash,
            "Block hash mismatch after SSZ round-trip!\n  original:      {original_hash:?}\n  reconstructed: {reconstructed_hash:?}"
        );
    }
}
