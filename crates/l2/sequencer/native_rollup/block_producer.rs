//! NativeBlockProducer GenServer — produces L2 blocks compatible with the
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
use ethrex_common::merkle_tree::{compute_merkle_proof, compute_merkle_root};
use ethrex_common::types::{EIP1559Transaction, MempoolTransaction, Transaction, TxKind};
use ethrex_common::{Address, H256, U256};
use ethrex_l2_common::calldata::Value;
use ethrex_l2_rpc::signer::{Signable, Signer};
use ethrex_l2_sdk::calldata::encode_calldata;
use ethrex_levm::execute_precompile::{L1_ANCHOR, L2_BRIDGE};
use ethrex_storage::Store;
use ethrex_vm::BlockExecutionResult;
use spawned_concurrency::tasks::{
    CastResponse, GenServer, GenServerHandle, InitResult, Success, send_after,
};
use tracing::{debug, error, info, warn};

use super::types::{L1Message, PendingL1Messages};

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

#[derive(Clone)]
pub enum CastMsg {
    Produce,
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
    Internal(#[from] spawned_concurrency::error::GenServerError),
    #[error("System time error: {0}")]
    SystemTime(#[from] std::time::SystemTimeError),
    #[error("Signer error: {0}")]
    Signer(String),
    #[error("Mempool error: {0}")]
    Mempool(String),
}

pub struct NativeBlockProducer {
    store: Store,
    config: NativeBlockProducerConfig,
    blockchain: Arc<Blockchain>,
    pending_l1_messages: PendingL1Messages,
}

impl NativeBlockProducer {
    pub fn new(
        store: Store,
        config: NativeBlockProducerConfig,
        blockchain: Arc<Blockchain>,
        pending_l1_messages: PendingL1Messages,
    ) -> Self {
        Self {
            store,
            config,
            blockchain,
            pending_l1_messages,
        }
    }

    /// Take L1 messages from the shared queue that fit within the block gas limit.
    ///
    /// Pops messages one by one, summing each message's gas limit. Once adding
    /// another message would exceed `block_gas_limit`, the remaining messages
    /// stay in the queue for the next block.
    fn take_l1_messages_for_block(&self) -> Vec<L1Message> {
        match self.pending_l1_messages.lock() {
            Ok(mut queue) => {
                let mut selected = Vec::new();
                let mut cumulative_gas: u64 = 0;

                while let Some(msg) = queue.front() {
                    let next_gas = cumulative_gas.saturating_add(msg.gas_limit);
                    if next_gas > self.config.block_gas_limit {
                        warn!(
                            "NativeBlockProducer: L1 messages gas ({next_gas}) would exceed \
                             block gas limit ({}), deferring {} remaining messages",
                            self.config.block_gas_limit,
                            queue.len()
                        );
                        break;
                    }
                    cumulative_gas = next_gas;
                    if let Some(msg) = queue.pop_front() {
                        selected.push(msg);
                    }
                }

                selected
            }
            Err(e) => {
                error!("NativeBlockProducer: failed to lock pending_l1_messages: {e}");
                Vec::new()
            }
        }
    }

    /// Write the L1 messages Merkle root to the L1Anchor predeploy's storage
    /// slot 0 in the VM cache, mirroring what the EXECUTE precompile does as a
    /// system transaction before regular execution. This allows L2 contracts
    /// (L2Bridge) to verify individual messages via Merkle proofs.
    fn anchor_l1_messages(
        messages: &[L1Message],
        context: &mut PayloadBuildContext,
    ) -> Result<(), NativeBlockProducerError> {
        let message_hashes: Vec<H256> = messages.iter().map(L1Message::compute_hash).collect();
        let merkle_root = compute_merkle_root(&message_hashes);

        let anchor_account =
            context.vm.db.get_account_mut(L1_ANCHOR).map_err(|e| {
                NativeBlockProducerError::Vm(format!("failed to load L1Anchor: {e}"))
            })?;
        anchor_account
            .storage
            .insert(H256::zero(), U256::from_big_endian(merkle_root.as_bytes()));

        // Also record the old value in initial_accounts_state so that
        // get_state_transitions() can compute the storage diff. Since we
        // write directly into the cache (this is a system action, not an EVM
        // execution), we must manually ensure the key exists in the initial
        // state — otherwise the diff will fail with "old value not found".
        if let Some(initial_account) = context.vm.db.initial_accounts_state.get_mut(&L1_ANCHOR) {
            initial_account
                .storage
                .entry(H256::zero())
                .or_insert(U256::zero());
        }

        debug!(
            "NativeBlockProducer: anchored L1 messages root {:?} ({} messages)",
            merkle_root,
            messages.len()
        );

        Ok(())
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
                    Value::Bytes(Bytes::new()), // empty data for PoC
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
                to: TxKind::Call(L2_BRIDGE),
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
                tip: 0,
            });
        }

        Ok(relayer_txs)
    }

    /// Produce a single L2 block following the payload_builder.rs pattern.
    async fn produce_block(&mut self) -> Result<(), NativeBlockProducerError> {
        // 1. Take L1 messages that fit within the block gas limit and build relayer txs
        let l1_messages = self.take_l1_messages_for_block();
        let relayer_txs = if !l1_messages.is_empty() {
            info!(
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

        let args = BuildPayloadArgs {
            parent: head_hash,
            timestamp: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
            fee_recipient: self.config.coinbase,
            random: H256::zero(),
            withdrawals: Some(vec![]),
            beacon_root: None,
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

        // 4. Anchor L1 messages Merkle root in L1Anchor predeploy before execution.
        //    Always performed, even with no messages (anchors zero hash).
        Self::anchor_l1_messages(&l1_messages, &mut context)?;

        // 5. Fill transactions: relayer txs first, then mempool
        self.fill_transactions(&mut context, relayer_txs).await?;

        // 6. Finalize payload (compute state root, receipts root, etc.)
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

        let chain_config = self.store.get_chain_config();
        let latest_block_number = self.store.get_latest_block_number().await?;

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

            // Check replay protection
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

            // Check nonce
            let maybe_sender_acc_info = self
                .store
                .get_account_info(latest_block_number, head_tx.tx.sender())
                .await?;

            if maybe_sender_acc_info.is_some_and(|acc_info| head_tx.nonce() < acc_info.nonce) {
                if is_relayer_tx {
                    return Err(NativeBlockProducerError::Vm(format!(
                        "relayer tx {tx_hash} failed: nonce too low"
                    )));
                }
                debug!("NativeBlockProducer: removing tx with nonce too low: {tx_hash:#x}");
                txs.pop();
                self.blockchain.remove_transaction_from_pool(&tx_hash)?;
                continue;
            }

            // Execute tx
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

impl GenServer for NativeBlockProducer {
    type CallMsg = ();
    type CastMsg = CastMsg;
    type OutMsg = ();
    type Error = NativeBlockProducerError;

    async fn init(self, handle: &GenServerHandle<Self>) -> Result<InitResult<Self>, Self::Error> {
        handle
            .clone()
            .cast(CastMsg::Produce)
            .await
            .map_err(NativeBlockProducerError::Internal)?;
        Ok(Success(self))
    }

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        handle: &GenServerHandle<Self>,
    ) -> CastResponse {
        match message {
            CastMsg::Produce => {
                let _ = self
                    .produce_block()
                    .await
                    .inspect_err(|e| error!("NativeBlockProducer error: {e}"));

                send_after(
                    Duration::from_millis(self.config.block_time_ms),
                    handle.clone(),
                    CastMsg::Produce,
                );
                CastResponse::NoReply
            }
        }
    }

    async fn handle_call(
        &mut self,
        _message: Self::CallMsg,
        _handle: &GenServerHandle<Self>,
    ) -> spawned_concurrency::tasks::CallResponse<Self> {
        spawned_concurrency::tasks::CallResponse::Reply(())
    }
}
