//! NativeBlockProducer GenServer — produces L2 blocks compatible with the
//! EXECUTE precompile on L1.
//!
//! This is a PoC implementation that builds blocks manually through LEVM,
//! mirroring exactly what the EXECUTE precompile does on L1:
//! 1. Write L1Anchor (Merkle root of consumed L1 messages) to L1Anchor predeploy
//! 2. Execute transactions using VMType::L1
//! 3. Compute post-state root and receipts root
//!
//! The produced blocks are pushed to the shared `ProducedBlocks` queue for
//! the L1 committer to pick up and submit via advance().

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use ethrex_common::merkle_tree::{compute_merkle_proof, compute_merkle_root};
use ethrex_common::types::{
    AccountState, AccountUpdate, Block, BlockBody, BlockHeader, EIP1559Transaction,
    ELASTICITY_MULTIPLIER, Receipt, Transaction, TxKind,
    block_execution_witness::{ExecutionWitness, GuestProgramState},
    calculate_base_fee_per_gas,
};
use ethrex_common::{Address, H256, U256};
use ethrex_crypto::keccak::keccak_hash;
use ethrex_l2_common::calldata::Value;
use ethrex_l2_sdk::calldata::encode_calldata;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::db::guest_program_state_db::GuestProgramStateDb;
use ethrex_levm::environment::{EVMConfig, Environment};
use ethrex_levm::errors::TxResult;
use ethrex_levm::execute_precompile::{L1_ANCHOR, L2_BRIDGE};
use ethrex_levm::tracing::LevmCallTracer;
use ethrex_levm::vm::{VM, VMType};
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::Store;
use ethrex_trie::Trie;
use spawned_concurrency::tasks::{
    CastResponse, GenServer, GenServerHandle, InitResult, Success, send_after,
};
use tracing::{debug, error, info, warn};

use super::types::{L1Message, PendingL1Messages, ProducedBlockInfo, ProducedBlocks};

/// Configuration for the native block producer.
#[derive(Clone, Debug)]
pub struct NativeBlockProducerConfig {
    pub block_time_ms: u64,
    pub coinbase: Address,
    pub block_gas_limit: u64,
    pub chain_id: u64,
    /// Private key bytes for the relayer that calls L2Bridge.processL1Message().
    /// In a real system this would come from a secure key store.
    pub relayer_key: [u8; 32],
}

#[derive(Clone)]
pub enum CastMsg {
    Produce,
}

#[derive(Debug, thiserror::Error)]
pub enum NativeBlockProducerError {
    #[error("Store error: {0}")]
    Store(#[from] ethrex_storage::error::StoreError),
    #[error("VM error: {0}")]
    Vm(String),
    #[error("Encoding error: {0}")]
    Encoding(String),
    #[error("Internal error: {0}")]
    Internal(#[from] spawned_concurrency::error::GenServerError),
    #[error("System time error: {0}")]
    SystemTime(#[from] std::time::SystemTimeError),
    #[error("Base fee calculation error: {0}")]
    BaseFee(String),
    #[error("Lock poisoned: {0}")]
    Lock(String),
    #[error("Missing data: {0}")]
    Missing(String),
}

pub struct NativeBlockProducer {
    store: Store,
    config: NativeBlockProducerConfig,
    pending_l1_messages: PendingL1Messages,
    produced_blocks: ProducedBlocks,
    /// Pre-compiled runtime bytecodes for L2Bridge and L1Anchor.
    /// Read once at startup from the solc output directory.
    bridge_runtime: Vec<u8>,
    anchor_runtime: Vec<u8>,
}

impl NativeBlockProducer {
    pub fn new(
        store: Store,
        config: NativeBlockProducerConfig,
        pending_l1_messages: PendingL1Messages,
        produced_blocks: ProducedBlocks,
        bridge_runtime: Vec<u8>,
        anchor_runtime: Vec<u8>,
    ) -> Self {
        Self {
            store,
            config,
            pending_l1_messages,
            produced_blocks,
            bridge_runtime,
            anchor_runtime,
        }
    }

    /// Drain all pending L1 messages from the shared queue.
    fn drain_l1_messages(&self) -> Vec<L1Message> {
        match self.pending_messages_lock() {
            Ok(mut queue) => queue.drain(..).collect(),
            Err(e) => {
                error!("NativeBlockProducer: {e}");
                Vec::new()
            }
        }
    }

    fn pending_messages_lock(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, VecDeque<L1Message>>, NativeBlockProducerError> {
        self.pending_l1_messages
            .lock()
            .map_err(|e| NativeBlockProducerError::Lock(e.to_string()))
    }

    /// Compute the keccak256(abi.encodePacked(sender, to, value, gasLimit, dataHash, nonce))
    /// message hash matching NativeRollup.sol _recordL1Message.
    fn compute_message_hash(msg: &L1Message) -> H256 {
        let mut preimage = Vec::with_capacity(168);
        preimage.extend_from_slice(msg.sender.as_bytes()); // 20 bytes
        preimage.extend_from_slice(msg.to.as_bytes()); // 20 bytes
        preimage.extend_from_slice(&msg.value.to_big_endian()); // 32 bytes
        preimage.extend_from_slice(&U256::from(msg.gas_limit).to_big_endian()); // 32 bytes
        preimage.extend_from_slice(msg.data_hash.as_bytes()); // 32 bytes
        preimage.extend_from_slice(&msg.nonce.to_big_endian()); // 32 bytes
        H256::from(keccak_hash(&preimage))
    }

    /// Derive the relayer address from the config's private key bytes.
    fn relayer_address(&self) -> Result<Address, NativeBlockProducerError> {
        use k256::ecdsa::{SigningKey, VerifyingKey};
        let key = SigningKey::from_bytes((&self.config.relayer_key).into())
            .map_err(|e| NativeBlockProducerError::Vm(format!("invalid relayer key: {e}")))?;
        let vk = VerifyingKey::from(&key);
        let pubkey_bytes = vk.to_encoded_point(false);
        let all_bytes = pubkey_bytes.as_bytes();
        let uncompressed = all_bytes
            .get(1..)
            .ok_or(NativeBlockProducerError::Vm("empty pubkey".into()))?;
        let hash = keccak_hash(uncompressed);
        let addr_bytes = hash
            .get(12..)
            .ok_or(NativeBlockProducerError::Vm("hash too short".into()))?;
        Ok(Address::from_slice(addr_bytes))
    }

    /// Build signed EIP-1559 relayer transactions that call
    /// L2Bridge.processL1Message(from, to, value, gasLimit, data, nonce, merkleProof)
    /// for each L1 message.
    fn build_relayer_transactions(
        &self,
        messages: &[L1Message],
        message_hashes: &[H256],
        relayer_nonce_start: u64,
        base_fee: u64,
    ) -> Result<Vec<Transaction>, NativeBlockProducerError> {
        use k256::ecdsa::SigningKey;

        let relayer_key = SigningKey::from_bytes((&self.config.relayer_key).into())
            .map_err(|e| NativeBlockProducerError::Encoding(e.to_string()))?;

        let mut txs = Vec::with_capacity(messages.len());

        for (i, msg) in messages.iter().enumerate() {
            let merkle_proof = compute_merkle_proof(message_hashes, i);

            // Encode processL1Message calldata
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
            let nonce = relayer_nonce_start + i_u64;
            let max_priority_fee = 1_000_000_000u64; // 1 gwei
            let max_fee = base_fee + max_priority_fee;

            let mut tx = EIP1559Transaction {
                chain_id: self.config.chain_id,
                nonce,
                max_priority_fee_per_gas: max_priority_fee,
                max_fee_per_gas: max_fee,
                gas_limit: 200_000, // generous for processL1Message
                to: TxKind::Call(L2_BRIDGE),
                value: U256::zero(),
                data: Bytes::from(calldata),
                access_list: vec![],
                ..Default::default()
            };

            // Sign the transaction
            sign_eip1559_tx(&mut tx, &relayer_key)?;
            txs.push(Transaction::EIP1559Transaction(tx));
        }

        Ok(txs)
    }

    /// Produce a single L2 block. This is the core function that mirrors
    /// the EXECUTE precompile's logic.
    async fn produce_block(&mut self) -> Result<(), NativeBlockProducerError> {
        // 1. Get parent header
        let current_block_number = self.store.get_latest_block_number().await?;
        let parent_header = self.store.get_block_header(current_block_number)?.ok_or(
            NativeBlockProducerError::Missing("parent header not found".into()),
        )?;
        let parent_hash = parent_header.compute_block_hash();
        let block_number = parent_header.number + 1;

        // 2. Compute base fee (EIP-1559)
        let base_fee = calculate_base_fee_per_gas(
            self.config.block_gas_limit,
            parent_header.gas_limit,
            parent_header.gas_used,
            parent_header.base_fee_per_gas.unwrap_or(1_000_000_000),
            ELASTICITY_MULTIPLIER,
        )
        .ok_or(NativeBlockProducerError::BaseFee(
            "base fee calculation returned None (gas limit check failed)".into(),
        ))?;

        // 3. Drain L1 messages and compute Merkle root
        let l1_messages = self.drain_l1_messages();
        let message_hashes: Vec<H256> =
            l1_messages.iter().map(Self::compute_message_hash).collect();
        let l1_anchor = compute_merkle_root(&message_hashes);
        let l1_messages_count: u64 = l1_messages
            .len()
            .try_into()
            .map_err(|_| NativeBlockProducerError::Encoding("too many L1 messages".into()))?;

        // 4. Build relayer transactions for L1 messages
        // Get relayer nonce from store
        let relayer = self.relayer_address()?;
        let current_block_number_for_nonce = self.store.get_latest_block_number().await?;
        let relayer_nonce = self
            .store
            .get_account_state(current_block_number_for_nonce, relayer)
            .await?
            .map(|a| a.nonce)
            .unwrap_or(0);

        let relayer_txs = if !l1_messages.is_empty() {
            self.build_relayer_transactions(&l1_messages, &message_hashes, relayer_nonce, base_fee)?
        } else {
            vec![]
        };

        // 5. For PoC: only include relayer txs (no mempool txs for simplicity)
        let transactions = relayer_txs;

        if transactions.is_empty() && l1_messages.is_empty() {
            debug!("NativeBlockProducer: no transactions, producing empty block");
        }

        // 6. Build block timestamp
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let coinbase = self.config.coinbase;
        let prev_randao = H256::zero();
        let chain_id = self.config.chain_id;

        // 7. Reconstruct the pre-state for this block using tries
        // For the PoC, we build the state from the store and execute via LEVM
        let _bridge_code_hash = H256(keccak_hash(&self.bridge_runtime));
        let _anchor_code_hash = H256(keccak_hash(&self.anchor_runtime));

        // Build state trie from store
        // We need to read account states for all addresses that will be touched
        let mut touched_addresses = vec![coinbase, relayer, L2_BRIDGE, L1_ANCHOR];
        for msg in &l1_messages {
            if !touched_addresses.contains(&msg.to) {
                touched_addresses.push(msg.to);
            }
        }

        let mut state_trie = Trie::new_temp();
        let mut storage_trie_roots = BTreeMap::new();

        for &addr in &touched_addresses {
            let account_state = self
                .store
                .get_account_state(current_block_number, addr)
                .await?
                .unwrap_or_default();

            insert_account(&mut state_trie, addr, &account_state);

            // If the account has storage (bridge or anchor), we need the storage trie
            if (addr == L2_BRIDGE || addr == L1_ANCHOR)
                && let Some(storage_trie) = self
                    .build_storage_trie_for_account(addr, &parent_header)
                    .await
                && let Some(root_node) = get_trie_root_node(&storage_trie)
            {
                storage_trie_roots.insert(addr, root_node);
            }
        }

        let pre_state_root = state_trie.hash_no_commit();

        // Verify pre-state root matches parent header (match EXECUTE precompile)
        if pre_state_root != parent_header.state_root {
            return Err(NativeBlockProducerError::Vm(format!(
                "Pre-state root mismatch: expected {:?} (parent), got {:?}",
                parent_header.state_root, pre_state_root
            )));
        }

        // 8. Build a temporary header for the witness
        let temp_header = BlockHeader {
            parent_hash,
            number: block_number,
            gas_limit: self.config.block_gas_limit,
            base_fee_per_gas: Some(base_fee),
            timestamp,
            coinbase,
            ..Default::default()
        };

        let chain_config = self.store.get_chain_config();

        // Build execution witness
        let witness = ExecutionWitness {
            codes: vec![self.bridge_runtime.clone(), self.anchor_runtime.clone()],
            block_headers_bytes: vec![parent_header.encode_to_vec(), temp_header.encode_to_vec()],
            first_block_number: block_number,
            chain_config,
            state_trie_root: get_trie_root_node(&state_trie),
            storage_trie_roots: storage_trie_roots.clone(),
            keys: vec![],
        };

        // 9. Create GuestProgramState and execute transactions
        // Build GuestProgramState from witness (consumes the witness).
        // We need to rebuild the final witness later with the correct header anyway.
        let guest_state: GuestProgramState = witness
            .try_into()
            .map_err(|e| NativeBlockProducerError::Vm(format!("{e:?}")))?;

        // Initialize block header hashes (match EXECUTE precompile)
        guest_state
            .initialize_block_header_hashes(&[])
            .map_err(|e| {
                NativeBlockProducerError::Vm(format!(
                    "Failed to initialize block header hashes: {e}"
                ))
            })?;

        let db_inner = Arc::new(GuestProgramStateDb::new(guest_state));

        // Write L1Anchor before executing transactions (system write)
        {
            let mut storage = rustc_hash::FxHashMap::default();
            storage.insert(H256::zero(), U256::from_big_endian(l1_anchor.as_bytes()));
            let anchor_update = AccountUpdate {
                address: L1_ANCHOR,
                added_storage: storage,
                ..Default::default()
            };
            db_inner
                .state
                .lock()
                .map_err(|e| NativeBlockProducerError::Lock(e.to_string()))?
                .apply_account_updates(&[anchor_update])
                .map_err(|e| NativeBlockProducerError::Vm(format!("L1Anchor write failed: {e}")))?;
        }

        let db_dyn: Arc<dyn ethrex_levm::db::Database> = db_inner.clone();
        let mut gen_db = GeneralizedDatabase::new(db_dyn);

        let config = EVMConfig::new_from_chain_config(&chain_config, &temp_header);

        // Execute each transaction and collect receipts
        let mut receipts = Vec::new();
        let mut total_gas_used = 0u64;

        for tx in &transactions {
            let (origin, gas_limit, tx_nonce, max_priority_fee, max_fee) = match tx {
                Transaction::EIP1559Transaction(inner) => {
                    let origin = tx
                        .sender()
                        .map_err(|e| NativeBlockProducerError::Vm(format!("sender: {e}")))?;
                    (
                        origin,
                        inner.gas_limit,
                        inner.nonce,
                        inner.max_priority_fee_per_gas,
                        inner.max_fee_per_gas,
                    )
                }
                _ => {
                    return Err(NativeBlockProducerError::Vm(
                        "Only EIP1559 transactions supported in native rollup".into(),
                    ));
                }
            };

            // Match EXECUTE precompile: effective_gas_price = min(max_priority + base_fee, max_fee)
            let effective_gas_price =
                U256::from(std::cmp::min(max_priority_fee + base_fee, max_fee));

            let env = Environment {
                origin,
                gas_limit,
                config,
                block_number: U256::from(block_number),
                coinbase,
                timestamp: U256::from(timestamp),
                prev_randao: Some(prev_randao),
                slot_number: U256::zero(),
                chain_id: U256::from(chain_id),
                base_fee_per_gas: U256::from(base_fee),
                base_blob_fee_per_gas: U256::zero(),
                gas_price: effective_gas_price,
                block_excess_blob_gas: None,
                block_blob_gas_used: None,
                tx_blob_hashes: vec![],
                tx_max_priority_fee_per_gas: Some(U256::from(max_priority_fee)),
                tx_max_fee_per_gas: Some(U256::from(max_fee)),
                tx_max_fee_per_blob_gas: None,
                tx_nonce,
                block_gas_limit: self.config.block_gas_limit,
                difficulty: U256::zero(),
                is_privileged: false,
                fee_token: None,
            };

            let mut vm = VM::new(env, &mut gen_db, tx, LevmCallTracer::disabled(), VMType::L1)
                .map_err(|e| NativeBlockProducerError::Vm(format!("VM creation: {e}")))?;

            let report = vm
                .execute()
                .map_err(|e| NativeBlockProducerError::Vm(format!("TX execution: {e}")))?;

            let succeeded = matches!(report.result, TxResult::Success);
            if !succeeded {
                warn!("NativeBlockProducer: tx failed: {:?}", report.result);
            }

            total_gas_used += report.gas_used;

            // Match EXECUTE precompile: failed txs get empty logs
            let receipt_logs = if succeeded { report.logs } else { vec![] };

            let receipt = Receipt::new(
                tx.tx_type(),
                succeeded,
                total_gas_used, // cumulative
                receipt_logs,
            );
            receipts.push(receipt);
        }

        // 10. Compute post-state root
        let account_updates = gen_db
            .get_state_transitions()
            .map_err(|e| NativeBlockProducerError::Vm(format!("state transitions: {e}")))?;

        db_inner
            .state
            .lock()
            .map_err(|e| NativeBlockProducerError::Lock(e.to_string()))?
            .apply_account_updates(&account_updates)
            .map_err(|e| NativeBlockProducerError::Vm(format!("apply updates: {e}")))?;

        let post_state_root = db_inner
            .state
            .lock()
            .map_err(|e| NativeBlockProducerError::Lock(e.to_string()))?
            .state_trie_root()
            .map_err(|e| NativeBlockProducerError::Vm(format!("state root: {e}")))?;

        // 11. Compute roots
        let transactions_root = ethrex_common::types::compute_transactions_root(&transactions);
        let receipts_root = ethrex_common::types::compute_receipts_root(&receipts);

        // 12. Build final block header
        let final_header = BlockHeader {
            parent_hash,
            number: block_number,
            gas_used: total_gas_used,
            gas_limit: self.config.block_gas_limit,
            base_fee_per_gas: Some(base_fee),
            timestamp,
            coinbase,
            transactions_root,
            receipts_root,
            state_root: post_state_root,
            withdrawals_root: Some(ethrex_common::types::compute_withdrawals_root(&[])),
            prev_randao,
            ..Default::default()
        };

        // Build final witness with correct header (for the L1 committer)
        let final_witness = ExecutionWitness {
            codes: vec![self.bridge_runtime.clone(), self.anchor_runtime.clone()],
            block_headers_bytes: vec![parent_header.encode_to_vec(), final_header.encode_to_vec()],
            first_block_number: block_number,
            chain_config,
            state_trie_root: get_trie_root_node(&state_trie),
            storage_trie_roots,
            keys: vec![],
        };

        let witness_json = serde_json::to_vec(&final_witness)
            .map_err(|e| NativeBlockProducerError::Encoding(e.to_string()))?;

        let block = Block {
            header: final_header.clone(),
            body: BlockBody {
                transactions: transactions.clone(),
                ommers: vec![],
                withdrawals: Some(vec![]),
            },
        };

        let transactions_rlp = transactions.encode_to_vec();

        // 13. Store the block in the store and update canonical chain
        let block_hash = final_header.compute_block_hash();
        self.store.add_block(block).await?;
        self.store
            .forkchoice_update(
                vec![(block_number, block_hash)],
                block_number,
                block_hash,
                None,
                None,
            )
            .await?;

        info!(
            "NativeBlockProducer: produced block {} with {} txs, gas_used={}, state_root={:?}",
            block_number,
            transactions.len(),
            total_gas_used,
            post_state_root
        );

        // 14. Push to produced blocks queue for the committer
        let produced_info = ProducedBlockInfo {
            block_number,
            pre_state_root,
            post_state_root,
            receipts_root,
            coinbase,
            prev_randao,
            timestamp,
            transactions_rlp,
            witness_json,
            gas_used: total_gas_used,
            l1_messages_count,
            l1_anchor,
            parent_base_fee: parent_header.base_fee_per_gas.unwrap_or(1_000_000_000),
            parent_gas_limit: parent_header.gas_limit,
            parent_gas_used: parent_header.gas_used,
        };

        self.produced_blocks
            .lock()
            .map_err(|e| NativeBlockProducerError::Lock(e.to_string()))?
            .push_back(produced_info);

        info!(
            "NativeBlockProducer: block {} queued for L1 commitment",
            block_number
        );

        Ok(())
    }

    /// Build a storage trie for an account by reading the storage trie from the store.
    ///
    /// For the PoC this reads known storage slots for L2Bridge and L1Anchor.
    /// A production implementation would copy the full storage trie.
    async fn build_storage_trie_for_account(
        &self,
        address: Address,
        parent_header: &BlockHeader,
    ) -> Option<Trie> {
        // Get the account's storage root from the state
        let account_state = self
            .store
            .get_account_state(parent_header.number, address)
            .await
            .ok()
            .flatten()?;

        if account_state.storage_root == *ethrex_trie::EMPTY_TRIE_HASH {
            return None;
        }

        // Open the storage trie from the store
        let hashed_address = H256(keccak_hash(address.to_fixed_bytes()));
        let storage_trie = self
            .store
            .open_storage_trie(
                hashed_address,
                parent_header.state_root,
                account_state.storage_root,
            )
            .ok()?;

        Some(storage_trie)
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

// ===== Helper functions =====

fn insert_account(trie: &mut Trie, address: Address, state: &AccountState) {
    let hashed_addr = keccak_hash(address.to_fixed_bytes()).to_vec();
    if let Err(e) = trie.insert(hashed_addr, state.encode_to_vec()) {
        error!("Failed to insert account {:?} into trie: {e}", address);
    }
}

fn get_trie_root_node(trie: &Trie) -> Option<ethrex_trie::Node> {
    trie.hash_no_commit();
    trie.root_node().ok()?.map(|arc_node| (*arc_node).clone())
}

/// Sign an EIP-1559 transaction with a k256 SigningKey.
/// This mirrors the test helper in test/tests/l2/native_rollups.rs.
fn sign_eip1559_tx(
    tx: &mut EIP1559Transaction,
    key: &k256::ecdsa::SigningKey,
) -> Result<(), NativeBlockProducerError> {
    use ethrex_rlp::structs::Encoder;
    use k256::ecdsa::signature::hazmat::PrehashSigner;

    let mut buf = vec![0x02u8];
    Encoder::new(&mut buf)
        .encode_field(&tx.chain_id)
        .encode_field(&tx.nonce)
        .encode_field(&tx.max_priority_fee_per_gas)
        .encode_field(&tx.max_fee_per_gas)
        .encode_field(&tx.gas_limit)
        .encode_field(&tx.to)
        .encode_field(&tx.value)
        .encode_field(&tx.data)
        .encode_field(&tx.access_list)
        .finish();

    let msg_hash = keccak_hash(&buf);
    let (sig, recid) = key
        .sign_prehash(&msg_hash)
        .map_err(|e| NativeBlockProducerError::Vm(format!("signing failed: {e}")))?;
    let sig_bytes = sig.to_bytes();
    let r_bytes = sig_bytes
        .get(..32)
        .ok_or(NativeBlockProducerError::Vm("sig_r too short".into()))?;
    let s_bytes = sig_bytes
        .get(32..64)
        .ok_or(NativeBlockProducerError::Vm("sig_s too short".into()))?;
    tx.signature_r = U256::from_big_endian(r_bytes);
    tx.signature_s = U256::from_big_endian(s_bytes);
    tx.signature_y_parity = recid.to_byte() != 0;
    Ok(())
}
