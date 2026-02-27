//! NativeL1Advancer GenServer — reads produced L2 blocks from the Store,
//! generates an execution witness, and submits them to the NativeRollup.sol
//! contract via advance().
//!
//! The advance() call sends:
//!   advance(uint256 l1MessagesCount, BlockParams blockParams, bytes transactionsRlp, bytes witnessJson)
//!
//! where BlockParams is:
//!   (bytes32 postStateRoot, bytes32 postReceiptsRoot, address coinbase, bytes32 prevRandao, uint256 timestamp)

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use ethrex_blockchain::Blockchain;
use ethrex_common::types::{Block, BlockHeader, TxType};
use ethrex_common::{Address, H256, U256};
use ethrex_l2_common::calldata::Value;
use ethrex_l2_rpc::signer::Signer;
use ethrex_l2_sdk::{
    build_generic_tx, calldata::encode_calldata, get_native_rollup_block_number,
    send_tx_bump_gas_exponential_backoff,
};
use ethrex_levm::execute_precompile::L1_ANCHOR;
use ethrex_rlp::encode::RLPEncode;
use ethrex_rpc::clients::Overrides;
use ethrex_rpc::clients::eth::EthClient;
use ethrex_storage::Store;
use spawned_concurrency::tasks::{
    CastResponse, GenServer, GenServerHandle, InitResult, Success, send_after,
};
use tracing::{debug, error, info};

const ADVANCE_FUNCTION_SIGNATURE: &str =
    "advance(uint256,(bytes32,bytes32,address,bytes32,uint256),bytes,bytes)";

#[derive(Clone)]
pub enum CastMsg {
    Advance,
}

#[derive(Debug, thiserror::Error)]
pub enum NativeL1AdvancerError {
    #[error("EthClient error: {0}")]
    EthClient(#[from] ethrex_rpc::clients::eth::errors::EthClientError),
    #[error("Encoding error: {0}")]
    Encoding(String),
    #[error("Internal error: {0}")]
    Internal(#[from] spawned_concurrency::error::GenServerError),
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
        //    Read the L1 messages Merkle root from L1Anchor's storage (the
        //    block producer already computed and stored it). The witness
        //    generator must apply this write before re-executing the block so
        //    that (a) the witness includes L1Anchor's trie nodes, and (b) the
        //    re-execution produces the same state root as the original block.
        let l1_anchor_value = self
            .store
            .get_storage_at(next_block, L1_ANCHOR, H256::zero())?
            .unwrap_or(U256::zero());

        let pre_execution_writes = vec![(L1_ANCHOR, H256::zero(), l1_anchor_value)];
        let witness = self
            .blockchain
            .generate_witness_for_blocks_with_pre_execution_writes(&[block], &pre_execution_writes)
            .await?;

        let witness_json = serde_json::to_vec(&witness)
            .map_err(|e| NativeL1AdvancerError::Encoding(e.to_string()))?;

        // 4. Count L1 messages by counting relayer txs in the block
        let l1_messages_count: u64 = block_body
            .transactions
            .iter()
            .filter(|tx| tx.sender().ok() == Some(self.relayer_address))
            .count()
            .try_into()
            .map_err(|_| NativeL1AdvancerError::Encoding("l1 messages count overflow".into()))?;

        // 5. Build and send advance() tx
        let transactions_rlp = block_body.transactions.encode_to_vec();
        let tx_hash = self
            .send_advance(
                &block_header,
                &transactions_rlp,
                &witness_json,
                l1_messages_count,
            )
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
        transactions_rlp: &[u8],
        witness_json: &[u8],
        l1_messages_count: u64,
    ) -> Result<H256, NativeL1AdvancerError> {
        // BlockParams struct: (postStateRoot, postReceiptsRoot, coinbase, prevRandao, timestamp)
        let block_params = Value::Tuple(vec![
            Value::FixedBytes(Bytes::from(header.state_root.as_bytes().to_vec())),
            Value::FixedBytes(Bytes::from(header.receipts_root.as_bytes().to_vec())),
            Value::Address(header.coinbase),
            Value::FixedBytes(Bytes::from(header.prev_randao.as_bytes().to_vec())),
            Value::Uint(U256::from(header.timestamp)),
        ]);

        let calldata = encode_calldata(
            ADVANCE_FUNCTION_SIGNATURE,
            &[
                Value::Uint(U256::from(l1_messages_count)),
                block_params,
                Value::Bytes(Bytes::from(transactions_rlp.to_vec())),
                Value::Bytes(Bytes::from(witness_json.to_vec())),
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

impl GenServer for NativeL1Advancer {
    type CallMsg = ();
    type CastMsg = CastMsg;
    type OutMsg = ();
    type Error = NativeL1AdvancerError;

    async fn init(self, handle: &GenServerHandle<Self>) -> Result<InitResult<Self>, Self::Error> {
        handle
            .clone()
            .cast(CastMsg::Advance)
            .await
            .map_err(NativeL1AdvancerError::Internal)?;
        Ok(Success(self))
    }

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        handle: &GenServerHandle<Self>,
    ) -> CastResponse {
        match message {
            CastMsg::Advance => {
                let _ = self
                    .advance_next_block()
                    .await
                    .inspect_err(|e| error!("NativeL1Advancer error: {e}"));

                send_after(
                    Duration::from_millis(self.advance_interval_ms),
                    handle.clone(),
                    CastMsg::Advance,
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
