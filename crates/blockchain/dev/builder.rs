use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use ethrex_blockchain::{
    Blockchain, BlockchainOptions, SuperBlockchain,
    fork_choice::apply_fork_choice,
    payload::{BuildPayloadArgs, PayloadBuildResult, create_payload},
};
use ethrex_common::{
    Address, H256,
    types::{DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER},
};
use ethrex_storage::{Store, error::StoreError};
use spawned_concurrency::{
    messages::Unused,
    tasks::{CastResponse, GenServer, GenServerHandle},
};

#[derive(Debug, thiserror::Error)]
pub enum L1BuilderError {
    #[error("Block builder error: {0}")]
    StoreError(#[from] StoreError),
    #[error("Block builder internal error: {0}")]
    InternalError(String),
}

pub struct SuperBuilder {
    super_blockchain: Arc<SuperBlockchain>,
}

impl SuperBuilder {
    pub async fn new() -> Self {
        let network = Network::LocalDevnet;

        let genesis = network.get_genesis()?;

        let mut store = {
            let mut store_inner = Store::new("./", EngineType::InMemory)?;
            store_inner.add_initial_state(genesis.clone()).await?;
            store_inner
        };

        let blockchain = Arc::new(Blockchain::new(store.clone(), BlockchainOptions::default()));

        Self {
            store,
            super_blockchain: blockchain,
        }
    }

    pub async fn spawn(
        store: Store,
        blockchain: Arc<Blockchain>,
    ) -> Result<GenServerHandle<SuperBuilder>, L1BuilderError> {
        let builder = Self::new().await.start_blocking();

        builder
            .cast(InMessage::Produce)
            .await
            .map_err(|err| L1BuilderError::InternalError(err.to_string()))?;

        Ok(builder)
    }

    pub async fn produce_block(&self) -> Result<(), L1BuilderError> {
        let head_block_header = {
            let current_block_number = self.store.get_latest_block_number().await?;
            self.store
                .get_block_header(current_block_number)?
                .ok_or(BlockProducerError::StorageDataIsNone)?
        };

        let build_payload_args = BuildPayloadArgs {
            parent: head_block_header.hash(),
            timestamp: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
            fee_recipient: Address::zero(),
            random: H256::zero(),
            withdrawals: Some(Vec::new()),
            beacon_root: Some(H256::zero()),
            version: 3,
            elasticity_multiplier: ELASTICITY_MULTIPLIER,
            gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
        };

        let payload_id = build_payload_args.id()?;

        let payload = create_payload(&build_payload_args, store, Bytes::new())?;

        // for n in 0..block_opts.n_txs.unwrap_or_default() {
        //     let tx_builder = match block_opts
        //         .tx
        //         .as_ref()
        //         .ok_or_eyre("--tx needs to be passed")?
        //     {
        //         TxVariant::ETHTransfer => TxBuilder::ETHTransfer,
        //         TxVariant::ERC20Transfer => unimplemented!(),
        //     };

        //     let tx = tx_builder.build_tx(n, signer).await;

        //     blockchain.add_transaction_to_pool(tx).await?;
        // }

        self.super_blockchain
            .clone()
            .initiate_payload_build(payload, payload_id)
            .await;

        let PayloadBuildResult { payload: block, .. } = self
            .super_blockchain
            .get_payload(payload_id)
            .await
            .map_err(|err| match err {
                ethrex_blockchain::error::ChainError::UnknownPayload => {
                    ethrex_rpc::RpcErr::UnknownPayload(format!(
                        "Payload with id {payload_id:#018x} not found",
                    ))
                }
                err => ethrex_rpc::RpcErr::Internal(err.to_string()),
            })?;

        self.super_blockchain.add_block(block.clone())?;

        // We clone here to avoid initializing the block hash, it is needed
        // uninitialized by the guest program.
        let new_block_hash = block.clone().hash();

        apply_fork_choice(&self.store, new_block_hash, new_block_hash, new_block_hash).await?;

        Ok(())
    }
}

#[derive(Clone)]
pub enum CastMsg {
    Build,
}

#[derive(Clone)]
pub enum OutMsg {
    Done,
}

impl GenServer for SuperBuilder {
    type CallMsg = Unused;

    type CastMsg = CastMsg;

    type OutMsg = OutMsg;

    type Error = L1BuilderError;

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        _handle: &GenServerHandle<Self>,
    ) -> CastResponse {
        match message {
            CastMsg::Build => {}
        }
    }
}
