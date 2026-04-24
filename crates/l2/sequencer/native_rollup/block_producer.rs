//! NativeBlockProducer actor — produces L2 blocks compatible with the
//! EXECUTE precompile on L1.
//!
//! Follows the same pattern as `payload_builder.rs` in the L2 stack:
//! 1. `create_payload` builds an initial empty block
//! 2. Relayer txs for L1 messages are built (not added to mempool)
//! 3. Relayer txs are executed first to guarantee L1 message inclusion
//! 4. Remaining gas is filled from mempool txs
//! 5. `finalize_payload` computes state root and receipts
//! 6. Block is stored and fork choice is applied
//!
//! The key difference from the L2 payload builder: this uses `BlockchainType::L1`
//! (and thus `VMType::L1`), has no blob-size constraints, and handles L1 message
//! relay via Merkle proofs. Relayer txs bypass the mempool to ensure they always
//! get priority over regular transactions.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use ethrex_blockchain::payload::{
    BuildPayloadArgs, HeadTransaction, PayloadBuildContext, apply_plain_transaction,
};
use ethrex_blockchain::{Blockchain, fork_choice::apply_fork_choice, payload::create_payload};
use ethrex_common::types::{EIP1559Transaction, MempoolTransaction, Transaction, TxKind};
use ethrex_common::{Address, H256, U256};
use ethrex_l2_common::calldata::Value;
use ethrex_l2_common::merkle_tree::{compute_merkle_proof, compute_merkle_root};
use ethrex_l2_common::messages::NATIVE_ROLLUP_L2_BRIDGE;
use ethrex_l2_rpc::signer::{Signable, Signer};
use ethrex_l2_sdk::calldata::encode_calldata;
// L1Anchor predeploy removed — L1 messages Merkle root is now
// carried via parent_beacon_block_root in the block header.
use ethrex_storage::Store;
use ethrex_vm::BlockExecutionResult;
use spawned_concurrency::{
    actor,
    error::ActorError,
    protocol,
    tasks::{Actor, ActorRef, ActorStart as _, Context, Handler, send_after},
};
use tracing::{debug, error, info, warn};

use super::types::L1Message;

/// Configuration for the native block producer.
#[derive(Clone, Debug)]
pub struct NativeBlockProducerConfig {
    pub block_time_ms: u64,
    pub coinbase: Address,
    pub block_gas_limit: u64,
    pub chain_id: u64,
    /// Signer for the relayer that calls L2Bridge.processL1Message().
    pub relayer_signer: Signer,
}

#[derive(Debug, thiserror::Error)]
pub enum NativeBlockProducerError {
    #[error("Store error: {0}")]
    Store(#[from] ethrex_storage::error::StoreError),
    #[error("Chain error: {0}")]
    Chain(#[from] ethrex_blockchain::error::ChainError),
    #[error("Fork choice error: {0}")]
    ForkChoice(#[from] ethrex_blockchain::error::InvalidForkChoice),
    #[error("Evm error: {0}")]
    Evm(#[from] ethrex_vm::EvmError),
    #[error("VM error: {0}")]
    Vm(String),
    #[error("Encoding error: {0}")]
    Encoding(String),
    #[error("Internal error: {0}")]
    Internal(#[from] spawned_concurrency::error::ActorError),
    #[error("System time error: {0}")]
    SystemTime(#[from] std::time::SystemTimeError),
    #[error("Signer error: {0}")]
    Signer(String),
    #[error("Mempool error: {0}")]
    Mempool(String),
}

#[protocol]
pub trait NativeBlockProducerProtocol: Send + Sync {
    fn produce(&self) -> Result<(), ActorError>;
    fn enqueue_l1_messages(&self, messages: Vec<L1Message>) -> Result<(), ActorError>;
}

pub struct NativeBlockProducer {
    store: Store,
    config: NativeBlockProducerConfig,
    blockchain: Arc<Blockchain>,
    /// Queue of L1 messages waiting to be included in the next L2 block.
    /// Populated by the L1 watcher via `EnqueueL1Messages` messages.
    pending_l1_messages: VecDeque<L1Message>,
}

impl NativeBlockProducer {
    pub fn new(
        store: Store,
        config: NativeBlockProducerConfig,
        blockchain: Arc<Blockchain>,
    ) -> Self {
        Self {
            store,
            config,
            blockchain,
            pending_l1_messages: VecDeque::new(),
        }
    }

    /// Produce a single L2 block following the payload_builder.rs pattern.
    async fn produce_block(&mut self) -> Result<(), NativeBlockProducerError> {
        // 1. Take L1 messages that fit within the block gas limit and build relayer txs
        let l1_messages = self.take_l1_messages_for_block();
        let relayer_txs = if !l1_messages.is_empty() {
            debug!(
                "NativeBlockProducer: building relayer txs for {} L1 messages",
                l1_messages.len()
            );
            self.build_relayer_transactions(&l1_messages).await?
        } else {
            VecDeque::new()
        };

        // 2. Create initial payload (same as L2 block producer)
        let head_header = {
            let current_block_number = self.store.get_latest_block_number().await?;
            self.store.get_block_header(current_block_number)?.ok_or(
                NativeBlockProducerError::Vm("parent header not found".into()),
            )?
        };
        let head_hash = head_header.hash();

        // Compute L1 messages Merkle root for parent_beacon_block_root.
        // This value will be stored by the EIP-4788 system contract during
        // block processing, making it accessible to L2 contracts via
        // BEACON_ROOTS_ADDRESS.
        let message_hashes: Vec<H256> = l1_messages.iter().map(L1Message::compute_hash).collect();
        let l1_merkle_root = compute_merkle_root(&message_hashes);

        let args = BuildPayloadArgs {
            parent: head_hash,
            timestamp: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
            fee_recipient: self.config.coinbase,
            random: H256::zero(),
            withdrawals: Some(vec![]),
            beacon_root: Some(l1_merkle_root),
            slot_number: None,
            version: 3,
            elasticity_multiplier: 2, // EIP-1559 default
            gas_ceil: self.config.block_gas_limit,
        };
        let payload = create_payload(&args, &self.store, Bytes::new())?;
        let block_number = payload.header.number;

        // 3. Build PayloadBuildContext with BlockchainType::L1 (VMType::L1)
        let mut context =
            PayloadBuildContext::new(payload, &self.store, &self.blockchain.options.r#type)?;

        // 4. Apply system operations (beacon root, block hash history)
        self.blockchain.apply_system_operations(&mut context)?;

        // 5. Fill transactions: relayer txs first, then mempool
        self.fill_transactions(&mut context, relayer_txs).await?;

        // 6. Extract requests and apply withdrawals (match L1 execute_block order)
        self.blockchain.extract_requests(&mut context)?;
        self.blockchain.apply_withdrawals(&mut context)?;

        // 7. Finalize payload (compute state root, receipts root, etc.)
        self.blockchain.finalize_payload(&mut context)?;

        // 7. Store block
        let block = context.payload;
        let account_updates = context.account_updates;

        let account_updates_list = self
            .store
            .apply_account_updates_batch(block.header.parent_hash, &account_updates)?
            .ok_or(NativeBlockProducerError::Chain(
                ethrex_blockchain::error::ChainError::ParentStateNotFound,
            ))?;

        let execution_result = BlockExecutionResult {
            receipts: context.receipts,
            requests: Vec::new(),
            block_gas_used: block.header.gas_used,
        };

        let transactions_count = block.body.transactions.len();
        let block_hash = block.hash();

        self.blockchain
            .store_block(block, account_updates_list, execution_result)?;

        // 8. Apply fork choice
        apply_fork_choice(&self.store, block_hash, block_hash, block_hash).await?;

        info!(
            "NativeBlockProducer: produced block {} ({:?}) with {} txs, gas_used={}",
            block_number, block_hash, transactions_count, context.cumulative_gas_spent
        );

        Ok(())
    }

    // -- Helpers --

    /// Take L1 messages from the internal queue that fit within the block gas limit.
    ///
    /// Pops messages one by one, summing each message's gas limit. Once adding
    /// another message would exceed `block_gas_limit`, the remaining messages
    /// stay in the queue for the next block.
    fn take_l1_messages_for_block(&mut self) -> Vec<L1Message> {
        let mut selected = Vec::new();
        let mut cumulative_gas: u64 = 0;

        while let Some(msg) = self.pending_l1_messages.front() {
            let next_gas = cumulative_gas.saturating_add(msg.gas_limit);
            if next_gas > self.config.block_gas_limit {
                warn!(
                    "NativeBlockProducer: L1 messages gas ({next_gas}) would exceed \
                     block gas limit ({}), deferring {} remaining messages",
                    self.config.block_gas_limit,
                    self.pending_l1_messages.len()
                );
                break;
            }
            cumulative_gas = next_gas;
            if let Some(msg) = self.pending_l1_messages.pop_front() {
                selected.push(msg);
            }
        }

        selected
    }

    /// Build signed EIP-1559 relayer transactions for each L1 message.
    ///
    /// Returns `HeadTransaction`s to be executed before mempool txs,
    /// guaranteeing L1 messages get priority in the block.
    async fn build_relayer_transactions(
        &self,
        messages: &[L1Message],
    ) -> Result<VecDeque<HeadTransaction>, NativeBlockProducerError> {
        let relayer_address = self.config.relayer_signer.address();

        // Get relayer nonce from store
        let latest_block_number = self.store.get_latest_block_number().await?;
        let start_nonce = self
            .store
            .get_account_state(latest_block_number, relayer_address)
            .await?
            .map(|a| a.nonce)
            .unwrap_or(0);

        // Get base fee from latest block header
        let base_fee = self
            .store
            .get_block_header(latest_block_number)?
            .and_then(|h| h.base_fee_per_gas)
            .unwrap_or(1_000_000_000);

        // Compute Merkle root and individual proofs
        let message_hashes: Vec<H256> = messages.iter().map(L1Message::compute_hash).collect();

        let mut relayer_txs = VecDeque::with_capacity(messages.len());

        for (i, msg) in messages.iter().enumerate() {
            let merkle_proof = compute_merkle_proof(&message_hashes, i);

            let calldata = encode_calldata(
                "processL1Message(address,address,uint256,uint256,bytes,uint256,bytes32[])",
                &[
                    Value::Address(msg.sender),
                    Value::Address(msg.to),
                    Value::Uint(msg.value),
                    Value::Uint(U256::from(msg.gas_limit)),
                    Value::Bytes(msg.data.clone()),
                    Value::Uint(msg.nonce),
                    Value::Array(
                        merkle_proof
                            .iter()
                            .map(|h| Value::FixedBytes(Bytes::from(h.as_bytes().to_vec())))
                            .collect(),
                    ),
                ],
            )
            .map_err(|e| NativeBlockProducerError::Encoding(e.to_string()))?;

            let i_u64: u64 = i
                .try_into()
                .map_err(|_| NativeBlockProducerError::Encoding("message index overflow".into()))?;
            let nonce = start_nonce + i_u64;
            let max_priority_fee = 1_000_000_000u64; // 1 gwei
            let max_fee = base_fee + max_priority_fee;

            let inner = EIP1559Transaction {
                chain_id: self.config.chain_id,
                nonce,
                max_priority_fee_per_gas: max_priority_fee,
                max_fee_per_gas: max_fee,
                gas_limit: msg.gas_limit,
                to: TxKind::Call(NATIVE_ROLLUP_L2_BRIDGE),
                value: U256::zero(),
                data: Bytes::from(calldata),
                access_list: vec![],
                ..Default::default()
            };

            let mut tx = Transaction::EIP1559Transaction(inner);
            tx.sign_inplace(&self.config.relayer_signer)
                .await
                .map_err(|e| NativeBlockProducerError::Signer(e.to_string()))?;

            relayer_txs.push_back(HeadTransaction {
                tx: MempoolTransaction::new(tx, relayer_address),
                tip: U256::zero(),
            });
        }

        Ok(relayer_txs)
    }

    // anchor_l1_messages removed — L1 messages Merkle root is now set via
    // parent_beacon_block_root in the block header (see produce_block).
    // The EIP-4788 system contract writes it to BEACON_ROOTS_ADDRESS during
    // block processing, making it accessible to L2 contracts.

    /// Fill transactions into the payload context.
    ///
    /// Relayer txs are popped first (before touching the mempool) to guarantee
    /// L1 messages always get included. Once drained, remaining gas is filled
    /// from regular mempool txs.
    async fn fill_transactions(
        &self,
        context: &mut PayloadBuildContext,
        mut relayer_txs: VecDeque<HeadTransaction>,
    ) -> Result<(), NativeBlockProducerError> {
        let (mut txs, mut blob_txs) = self.blockchain.fetch_mempool_transactions(context)?;

        // Discard blob txs — not supported in native rollup
        while blob_txs.peek().is_some() {
            let blob_hash = blob_txs.peek().map(|tx| tx.tx.hash()).unwrap_or_default();
            self.blockchain.remove_transaction_from_pool(&blob_hash)?;
            blob_txs.pop();
        }

        loop {
            if context.remaining_gas < ethrex_blockchain::constants::TX_GAS_COST {
                debug!("NativeBlockProducer: no more gas to run transactions");
                break;
            }

            // Relayer txs first, then mempool
            let mut is_relayer_tx = false;
            let head_tx = if let Some(relayer_tx) = relayer_txs.pop_front() {
                is_relayer_tx = true;
                relayer_tx
            } else if let Some(peeked) = txs.peek() {
                peeked
            } else {
                break;
            };

            // Check if we have enough gas for this specific tx
            if context.remaining_gas < head_tx.tx.gas_limit() {
                if is_relayer_tx {
                    return Err(NativeBlockProducerError::Vm(format!(
                        "relayer tx {} failed: not enough gas",
                        head_tx.tx.hash()
                    )));
                }
                debug!(
                    "NativeBlockProducer: skipping tx {}, not enough gas",
                    head_tx.tx.hash()
                );
                txs.pop();
                continue;
            }

            let tx_hash = head_tx.tx.hash();

            // Check whether the tx is replay-protected
            let chain_config = self.store.get_chain_config();
            if head_tx.tx.protected() && !chain_config.is_eip155_activated(context.block_number()) {
                if is_relayer_tx {
                    return Err(NativeBlockProducerError::Vm(format!(
                        "relayer tx {tx_hash} failed: replay protection check"
                    )));
                }
                debug!(
                    "NativeBlockProducer: ignoring replay-protected tx: {}",
                    tx_hash
                );
                txs.pop();
                self.blockchain.remove_transaction_from_pool(&tx_hash)?;
                continue;
            }

            // Execute tx (the VM validates nonce, balance, etc.)
            let receipt = match apply_plain_transaction(&head_tx, context) {
                Ok(receipt) => receipt,
                Err(e) => {
                    if is_relayer_tx {
                        return Err(NativeBlockProducerError::Vm(format!(
                            "relayer tx {tx_hash} failed: {e}"
                        )));
                    }
                    debug!("NativeBlockProducer: failed to execute tx {}: {e}", tx_hash);
                    txs.pop();
                    continue;
                }
            };

            if !is_relayer_tx {
                txs.shift()?;
                self.blockchain.remove_transaction_from_pool(&tx_hash)?;
            }

            let tx: Transaction = (*head_tx).clone();
            context.payload.body.transactions.push(tx);
            context.receipts.push(receipt);
        }

        Ok(())
    }
}

#[actor(protocol = NativeBlockProducerProtocol)]
impl NativeBlockProducer {
    pub fn spawn(
        store: Store,
        config: NativeBlockProducerConfig,
        blockchain: Arc<Blockchain>,
    ) -> ActorRef<NativeBlockProducer> {
        let producer = Self::new(store, config, blockchain);
        producer.start()
    }

    #[started]
    async fn started(&mut self, ctx: &Context<Self>) {
        let _ = ctx
            .send(native_block_producer_protocol::Produce)
            .inspect_err(|e| error!("NativeBlockProducer: failed to send initial Produce: {e}"));
    }

    #[send_handler]
    async fn handle_produce(
        &mut self,
        _msg: native_block_producer_protocol::Produce,
        ctx: &Context<Self>,
    ) {
        let _ = self
            .produce_block()
            .await
            .inspect_err(|e| error!("NativeBlockProducer error: {e}"));

        send_after(
            Duration::from_millis(self.config.block_time_ms),
            ctx.clone(),
            native_block_producer_protocol::Produce,
        );
    }

    #[send_handler]
    async fn handle_enqueue_l1_messages(
        &mut self,
        msg: native_block_producer_protocol::EnqueueL1Messages,
        _ctx: &Context<Self>,
    ) {
        for m in &msg.messages {
            debug!(
                "NativeBlockProducer: queued L1 message nonce={} sender={:?} to={:?} value={}",
                m.nonce, m.sender, m.to, m.value
            );
        }
        self.pending_l1_messages.extend(msg.messages);
    }
}
