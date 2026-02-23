//! NativeL1Committer GenServer — picks produced L2 blocks from the shared
//! queue and submits them to the NativeRollup.sol contract via advance().
//!
//! The advance() call sends:
//!   advance(uint256 l1MessagesCount, BlockParams blockParams, bytes transactionsRlp, bytes witnessJson)
//!
//! where BlockParams is:
//!   (bytes32 postStateRoot, bytes32 postReceiptsRoot, address coinbase, bytes32 prevRandao, uint256 timestamp)

use std::time::Duration;

use bytes::Bytes;
use ethrex_common::types::TxType;
use ethrex_common::{Address, H256, U256};
use ethrex_l2_common::calldata::Value;
use ethrex_l2_rpc::signer::Signer;
use ethrex_l2_sdk::{
    build_generic_tx, calldata::encode_calldata, send_tx_bump_gas_exponential_backoff,
};
use ethrex_rpc::clients::Overrides;
use ethrex_rpc::clients::eth::EthClient;
use spawned_concurrency::tasks::{
    CastResponse, GenServer, GenServerHandle, InitResult, Success, send_after,
};
use tracing::{debug, error, info};

use super::types::{ProducedBlockInfo, ProducedBlocks};

const ADVANCE_FUNCTION_SIGNATURE: &str =
    "advance(uint256,(bytes32,bytes32,address,bytes32,uint256),bytes,bytes)";

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
    #[error("Lock poisoned: {0}")]
    Lock(String),
    #[error("Signer error: {0}")]
    Signer(String),
}

pub struct NativeL1Committer {
    eth_client: EthClient,
    contract_address: Address,
    signer: Signer,
    produced_blocks: ProducedBlocks,
    commit_interval_ms: u64,
}

impl NativeL1Committer {
    pub fn new(
        eth_client: EthClient,
        contract_address: Address,
        signer: Signer,
        produced_blocks: ProducedBlocks,
        commit_interval_ms: u64,
    ) -> Self {
        Self {
            eth_client,
            contract_address,
            signer,
            produced_blocks,
            commit_interval_ms,
        }
    }

    /// Pop the next produced block from the queue and submit it to L1.
    async fn commit_next_block(&mut self) -> Result<(), NativeL1CommitterError> {
        let block_info = {
            let mut queue = self
                .produced_blocks
                .lock()
                .map_err(|e| NativeL1CommitterError::Lock(e.to_string()))?;
            match queue.pop_front() {
                Some(info) => info,
                None => {
                    debug!("NativeL1Committer: no blocks to commit");
                    return Ok(());
                }
            }
        };

        info!(
            "NativeL1Committer: committing block {} to L1 (state_root={:?}, l1_msgs={})",
            block_info.block_number, block_info.post_state_root, block_info.l1_messages_count
        );

        let tx_hash = self.send_advance(&block_info).await?;

        info!(
            "NativeL1Committer: advance() tx sent for block {}: {:?}",
            block_info.block_number, tx_hash
        );

        Ok(())
    }

    /// Build and send the advance() transaction to NativeRollup.sol.
    async fn send_advance(
        &self,
        block: &ProducedBlockInfo,
    ) -> Result<H256, NativeL1CommitterError> {
        // BlockParams struct: (postStateRoot, postReceiptsRoot, coinbase, prevRandao, timestamp)
        let block_params = Value::Tuple(vec![
            Value::FixedBytes(Bytes::from(block.post_state_root.as_bytes().to_vec())),
            Value::FixedBytes(Bytes::from(block.receipts_root.as_bytes().to_vec())),
            Value::Address(block.coinbase),
            Value::FixedBytes(Bytes::from(block.prev_randao.as_bytes().to_vec())),
            Value::Uint(U256::from(block.timestamp)),
        ]);

        let calldata = encode_calldata(
            ADVANCE_FUNCTION_SIGNATURE,
            &[
                Value::Uint(U256::from(block.l1_messages_count)),
                block_params,
                Value::Bytes(Bytes::from(block.transactions_rlp.clone())),
                Value::Bytes(Bytes::from(block.witness_json.clone())),
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
