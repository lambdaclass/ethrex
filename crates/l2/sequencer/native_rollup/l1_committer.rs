//! NativeL1Committer GenServer — reads produced L2 blocks from the Store,
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
    build_generic_tx, calldata::encode_calldata, send_tx_bump_gas_exponential_backoff,
};
use ethrex_levm::execute_precompile::L1_ANCHOR;
use ethrex_rlp::encode::RLPEncode;
use ethrex_rpc::clients::Overrides;
use ethrex_rpc::clients::eth::EthClient;
use ethrex_rpc::types::block_identifier::BlockIdentifier;
use ethrex_storage::Store;
use spawned_concurrency::tasks::{
    CastResponse, GenServer, GenServerHandle, InitResult, Success, send_after,
};
use tracing::{debug, error, info};

const ADVANCE_FUNCTION_SIGNATURE: &str =
    "advance(uint256,(bytes32,bytes32,address,bytes32,uint256),bytes,bytes)";

/// NativeRollup.sol storage slot 1 stores the current block number.
const BLOCK_NUMBER_SLOT: u64 = 1;

#[derive(Clone)]
pub enum CastMsg {
    Commit,
}

#[derive(Debug, thiserror::Error)]
pub enum NativeL1CommitterError {
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

pub struct NativeL1Committer {
    eth_client: EthClient,
    contract_address: Address,
    signer: Signer,
    store: Store,
    blockchain: Arc<Blockchain>,
    relayer_address: Address,
    commit_interval_ms: u64,
}

impl NativeL1Committer {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        eth_client: EthClient,
        contract_address: Address,
        signer: Signer,
        store: Store,
        blockchain: Arc<Blockchain>,
        relayer_address: Address,
        commit_interval_ms: u64,
    ) -> Self {
        Self {
            eth_client,
            contract_address,
            signer,
            store,
            blockchain,
            relayer_address,
            commit_interval_ms,
        }
    }

    /// Determine the next block to commit and submit it to L1.
    async fn commit_next_block(&mut self) -> Result<(), NativeL1CommitterError> {
        // 1. Query the on-chain block number from NativeRollup contract storage slot 1
        let on_chain_block_number = self
            .eth_client
            .get_storage_at(
                self.contract_address,
                U256::from(BLOCK_NUMBER_SLOT),
                BlockIdentifier::Tag(ethrex_rpc::types::block_identifier::BlockTag::Latest),
            )
            .await?;

        let next_block: u64 = (on_chain_block_number + 1)
            .try_into()
            .map_err(|_| NativeL1CommitterError::Encoding("block number overflow".into()))?;

        // 2. Fetch block from Store
        let block_header = match self.store.get_block_header(next_block)? {
            Some(h) => h,
            None => {
                debug!(
                    "NativeL1Committer: block {} not produced yet, skipping",
                    next_block
                );
                return Ok(());
            }
        };

        let block_body = match self.store.get_block_body(next_block).await? {
            Some(b) => b,
            None => {
                debug!(
                    "NativeL1Committer: block {} body not found, skipping",
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
            .map_err(|e| NativeL1CommitterError::Encoding(e.to_string()))?;

        // 4. Count L1 messages by counting relayer txs in the block
        let l1_messages_count: u64 = block_body
            .transactions
            .iter()
            .filter(|tx| tx.sender().ok() == Some(self.relayer_address))
            .count()
            .try_into()
            .map_err(|_| NativeL1CommitterError::Encoding("l1 messages count overflow".into()))?;

        info!(
            "NativeL1Committer: committing block {} to L1 (state_root={:?}, l1_msgs={})",
            next_block, block_header.state_root, l1_messages_count
        );

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
            "NativeL1Committer: advance() tx sent for block {}: {:?}",
            next_block, tx_hash
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
    ) -> Result<H256, NativeL1CommitterError> {
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
        .map_err(|e| NativeL1CommitterError::Encoding(e.to_string()))?;

        let gas_price = self
            .eth_client
            .get_gas_price_with_extra(20)
            .await?
            .try_into()
            .unwrap_or(20_000_000_000u64);

        let tx = build_generic_tx(
            &self.eth_client,
            TxType::EIP1559,
            self.contract_address,
            self.signer.address(),
            Bytes::from(calldata),
            Overrides {
                from: Some(self.signer.address()),
                max_fee_per_gas: Some(gas_price),
                max_priority_fee_per_gas: Some(gas_price),
                ..Default::default()
            },
        )
        .await
        .map_err(NativeL1CommitterError::EthClient)?;

        let tx_hash = send_tx_bump_gas_exponential_backoff(&self.eth_client, tx, &self.signer)
            .await
            .map_err(NativeL1CommitterError::EthClient)?;

        Ok(tx_hash)
    }
}

impl GenServer for NativeL1Committer {
    type CallMsg = ();
    type CastMsg = CastMsg;
    type OutMsg = ();
    type Error = NativeL1CommitterError;

    async fn init(self, handle: &GenServerHandle<Self>) -> Result<InitResult<Self>, Self::Error> {
        handle
            .clone()
            .cast(CastMsg::Commit)
            .await
            .map_err(NativeL1CommitterError::Internal)?;
        Ok(Success(self))
    }

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        handle: &GenServerHandle<Self>,
    ) -> CastResponse {
        match message {
            CastMsg::Commit => {
                let _ = self
                    .commit_next_block()
                    .await
                    .inspect_err(|e| error!("NativeL1Committer error: {e}"));

                send_after(
                    Duration::from_millis(self.commit_interval_ms),
                    handle.clone(),
                    CastMsg::Commit,
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
