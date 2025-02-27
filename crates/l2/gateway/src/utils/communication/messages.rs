use crate::utils::{
    time::{Duration, IngestionTime, Instant, Nanos},
    transaction::{SimulatedTx, Transaction},
};
use ethrex_common::{
    types::{Block, Fork},
    Address, H256,
};
use ethrex_rpc::{
    engine::{
        fork_choice::ForkChoiceUpdatedV3,
        payload::{GetPayloadV3Request, NewPayloadV3Request},
    },
    types::{
        fork_choice::{ForkChoiceState, PayloadAttributesV3},
        payload::ExecutionPayload,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
};
use strum_macros::AsRefStr;
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize, Deserialize, Default)]
pub struct InternalMessage<T> {
    ingestion_t: IngestionTime,
    data: T,
}

impl<T> InternalMessage<T> {
    #[inline]
    pub fn new(ingestion_t: IngestionTime, data: T) -> Self {
        Self { ingestion_t, data }
    }

    #[inline]
    pub fn with_data<D>(&self, data: D) -> InternalMessage<D> {
        InternalMessage::new(self.ingestion_t, data)
    }

    #[inline]
    pub fn data(&self) -> &T {
        &self.data
    }

    #[inline]
    pub fn into_data(self) -> T {
        self.data
    }

    #[inline]
    pub fn map<R>(self, f: impl FnOnce(T) -> R) -> InternalMessage<R> {
        InternalMessage {
            ingestion_t: self.ingestion_t,
            data: f(self.data),
        }
    }

    #[inline]
    pub fn map_ref<R>(&self, f: impl FnOnce(&T) -> R) -> InternalMessage<R> {
        InternalMessage {
            ingestion_t: self.ingestion_t,
            data: f(&self.data),
        }
    }

    #[inline]
    pub fn unpack(self) -> (IngestionTime, T) {
        (self.ingestion_t, self.data)
    }

    /// This is only useful within the same socket as the original tsamp
    #[inline]
    pub fn elapsed(&self) -> Duration {
        self.ingestion_t.internal().elapsed()
    }

    /// These are real nanos since unix epoc
    #[inline]
    pub fn elapsed_nanos(&self) -> Nanos {
        self.ingestion_t.real().elapsed()
    }

    #[inline]
    pub fn ingestion_time(&self) -> IngestionTime {
        self.ingestion_t
    }
}

impl<T> From<InternalMessage<T>> for (IngestionTime, T) {
    #[inline]
    fn from(value: InternalMessage<T>) -> Self {
        value.unpack()
    }
}

impl<T> From<T> for InternalMessage<T> {
    #[inline]
    fn from(value: T) -> Self {
        Self::new(IngestionTime::now(), value)
    }
}

impl<T> Deref for InternalMessage<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl<T> DerefMut for InternalMessage<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data
    }
}

impl<T> From<&InternalMessage<T>> for IngestionTime {
    #[inline]
    fn from(value: &InternalMessage<T>) -> Self {
        value.ingestion_t
    }
}

impl<T> AsRef<IngestionTime> for InternalMessage<T> {
    #[inline]
    fn as_ref(&self) -> &IngestionTime {
        &self.ingestion_t
    }
}

impl<T> From<&InternalMessage<T>> for Instant {
    #[inline]
    fn from(value: &InternalMessage<T>) -> Self {
        value.ingestion_t.into()
    }
}

impl<T> From<&InternalMessage<T>> for Nanos {
    #[inline]
    fn from(value: &InternalMessage<T>) -> Self {
        value.ingestion_t.into()
    }
}

impl<T> From<InternalMessage<T>> for Instant {
    #[inline]
    fn from(value: InternalMessage<T>) -> Self {
        value.ingestion_t.into()
    }
}

impl<T> From<InternalMessage<T>> for Nanos {
    #[inline]
    fn from(value: InternalMessage<T>) -> Self {
        value.ingestion_t.into()
    }
}

/// Supported Engine API RPC methods
#[derive(Debug, AsRefStr)]
pub enum EngineApi {
    ForkChoiceUpdatedV3(ForkChoiceUpdatedV3),
    NewPayloadV3(NewPayloadV3Request),
    GetPayloadV3(GetPayloadV3Request),
}

impl EngineApi {
    pub fn messages_from_block(block: &Block) -> (EngineApi, EngineApi, EngineApi) {
        let block_hash = block.hash();

        let new_payload = EngineApi::NewPayloadV3(NewPayloadV3Request {
            payload: ExecutionPayload::from_block(block.clone()),
            expected_blob_versioned_hashes: Default::default(),
            parent_beacon_block_root: block
                .header
                .parent_beacon_block_root
                .expect("parent beacon root should always be set"),
        });

        let first_forkchoice_updated = EngineApi::ForkChoiceUpdatedV3(ForkChoiceUpdatedV3 {
            fork_choice_state: ForkChoiceState {
                head_block_hash: block_hash,
                safe_block_hash: Default::default(),
                finalized_block_hash: Default::default(),
            },
            payload_attributes: None,
        });

        let second_forkchoice_updated = EngineApi::ForkChoiceUpdatedV3(ForkChoiceUpdatedV3 {
            fork_choice_state: ForkChoiceState {
                head_block_hash: block_hash,
                safe_block_hash: Default::default(),
                finalized_block_hash: Default::default(),
            },
            payload_attributes: Some(PayloadAttributesV3 {
                timestamp: block.header.timestamp,
                prev_randao: block.header.prev_randao,
                suggested_fee_recipient: block.header.coinbase,
                withdrawals: Default::default(),
                parent_beacon_block_root: block.header.parent_beacon_block_root,
            }),
        });

        (
            new_payload,
            first_forkchoice_updated,
            second_forkchoice_updated,
        )
    }
}

#[derive(Clone, Debug, AsRefStr)]
#[repr(u8)]
pub enum SequencerToSimulator<Db> {
    /// Simulate Tx
    SimulateTx(Arc<Transaction>, DBSorting<Db>),
    /// Simulate Tx Top of frag
    //TODO: Db could be set on frag commit once we broadcast msgs to sims
    SimulateTxTof(Arc<Transaction>, DBFrag<Db>),
}

impl<Db> SequencerToSimulator<Db> {
    pub fn sim_info(&self) -> (Address, u64, u64) {
        match self {
            SequencerToSimulator::SimulateTx(t, db) => (t.sender(), t.nonce(), db.state_id()),
            SequencerToSimulator::SimulateTxTof(t, db) => (t.sender(), t.nonce(), db.state_id()),
        }
    }
}

#[derive(Debug)]
pub struct SimulatorToSequencer {
    /// Sender address and nonce
    pub sender_info: (Address, u64),
    pub state_id: u64,
    pub simtime: Duration,
    pub msg: SimulatorToSequencerMsg,
}

impl SimulatorToSequencer {
    pub fn new(
        sender_info: (Address, u64),
        state_id: u64,
        simtime: Duration,
        msg: SimulatorToSequencerMsg,
    ) -> Self {
        Self {
            sender_info,
            state_id,
            simtime,
            msg,
        }
    }

    pub fn sender(&self) -> &Address {
        &self.sender_info.0
    }

    pub fn nonce(&self) -> u64 {
        self.sender_info.1
    }
}

pub type SimulationResult<T> = Result<T, SimulationError>;

#[derive(Debug, AsRefStr)]
#[repr(u8)]
pub enum SimulatorToSequencerMsg {
    /// Simulation on top of any state.
    Tx(SimulationResult<SimulatedTx>),
    /// Simulation on top of a fragment. Used by the transaction pool.
    TxPoolTopOfFrag(SimulationResult<SimulatedTx>),
}

#[derive(Clone, Debug, Error, AsRefStr)]
#[repr(u8)]
pub enum SimulationError {
    #[error("Evm error: {0}")]
    EvmError(String),
    #[error("Order pays nothing")]
    ZeroPayment,
    #[error("Order reverts and is not allowed to revert")]
    RevertWithDisallowedRevert,
}

#[derive(Clone, Copy, Debug, PartialEq, AsRefStr)]
pub enum SequencerToExternal {}

#[derive(Debug, thiserror::Error)]
pub enum BlockSyncError {
    #[error("Block fetch failed: {0}")]
    Fetch(#[from] reqwest::Error),
    // #[error("Block execution failed: {0}")]
    // Execution(#[from] BlockExecutionError),
    #[error("DB error: {0}")]
    Database(#[from] crate::db::Error),
    // #[error("Payload error: {0}")]
    // Payload(#[from] PayloadError),
    #[error("Failed to recover transaction signer")]
    SignerRecovery,
}

pub type BlockSyncMessage = Block;

#[derive(Clone, Debug, AsRefStr)]
pub enum BlockFetch {
    FromTo(u64, u64),
}

impl BlockFetch {
    pub fn fetch_to(&self) -> u64 {
        match self {
            BlockFetch::FromTo(_, to) => *to,
        }
    }
}

/// Represents the parameters required to configure the next block.
#[derive(Clone, Debug)]
pub struct EvmBlockParams {
    pub spec_id: Fork,
    pub env: Box<Env>,
}

#[derive(Clone)]
pub struct NextBlockAttributes {
    pub env_attributes: NextBlockEnvAttributes,
    /// Txs to add top of block.
    pub forced_inclusion_txs: Vec<Arc<Transaction>>,
    /// Parent block beacon root.
    pub parent_beacon_block_root: Option<H256>,
}

impl std::fmt::Debug for NextBlockAttributes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NextBlockAttributes")
            .field("env_attributes", &self.env_attributes)
            .field("forced_inclusion_txs", &self.forced_inclusion_txs.len())
            .field("parent_beacon_block_root", &self.parent_beacon_block_root)
            .finish()
    }
}
