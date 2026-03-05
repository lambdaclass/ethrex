use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use ethrex_blockchain::Blockchain;
use ethrex_common::H256;
use ethrex_common::types::{MempoolTransaction, Transaction};
use ethrex_storage::error::StoreError;
use rand::{seq::SliceRandom, thread_rng};
use spawned_concurrency::{
    actor,
    error::ActorError,
    protocol,
    tasks::{Actor, ActorRef, ActorStart as _, Context, Handler, send_interval},
};
use tracing::{debug, error, info, trace};

use crate::{
    peer_table::{PeerTable, PeerTableServerProtocol as _},
    rlpx::{
        Message,
        connection::server::PeerConnection,
        eth::transactions::{NewPooledTransactionHashes, Transactions},
        p2p::{Capability, SUPPORTED_ETH_CAPABILITIES},
    },
};

// Soft limit for the number of transaction hashes sent in a single NewPooledTransactionHashes message as per [the spec](https://github.com/ethereum/devp2p/blob/master/caps/eth.md#newpooledtransactionhashes-0x080)
const NEW_POOLED_TRANSACTION_HASHES_SOFT_LIMIT: usize = 4096;

// Amount of seconds after which we prune broadcast records (We should fine tune this)
const PRUNE_WAIT_TIME_SECS: u64 = 600; // 10 minutes

// Amount of seconds between each prune
const PRUNE_INTERVAL_SECS: u64 = 360; // 6 minutes

// Amount of milliseconds between each broadcast
pub const BROADCAST_INTERVAL_MS: u64 = 1000; // 1 second

#[protocol]
pub trait TxBroadcasterProtocol: Send + Sync {
    fn broadcast_txs(&self) -> Result<(), ActorError>;
    fn add_txs(&self, tx_hashes: Vec<H256>, peer_id: H256) -> Result<(), ActorError>;
    fn prune_txs(&self) -> Result<(), ActorError>;
}

#[derive(Debug, Clone, Default)]
struct PeerMask {
    bits: Vec<u64>,
}

impl PeerMask {
    #[inline]
    // Ensure that the internal bit vector can hold the given index
    // If not, resize the vector.
    fn ensure(&mut self, idx: u32) {
        let word = (idx as usize) / 64;
        if self.bits.len() <= word {
            self.bits.resize(word + 1, 0);
        }
    }

    #[inline]
    fn is_set(&self, idx: u32) -> bool {
        let word = (idx as usize) / 64;
        if word >= self.bits.len() {
            return false;
        }
        let bit = (idx as usize) % 64;
        (self.bits[word] >> bit) & 1 == 1
    }

    #[inline]
    fn set(&mut self, idx: u32) {
        self.ensure(idx);
        let word = (idx as usize) / 64;
        let bit = (idx as usize) % 64;
        self.bits[word] |= 1u64 << bit;
    }
}

#[derive(Debug, Clone)]
struct BroadcastRecord {
    peers: PeerMask,
    last_sent: Instant,
}

impl Default for BroadcastRecord {
    fn default() -> Self {
        Self {
            peers: PeerMask::default(),
            last_sent: Instant::now(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TxBroadcaster {
    peer_table: PeerTable,
    blockchain: Arc<Blockchain>,
    // tx_hash -> broadcast record (which peers know it and when it was last sent)
    known_txs: HashMap<H256, BroadcastRecord>,
    // Assign each peer_id (H256) a u32 index used by PeerMask entries
    peer_indexer: HashMap<H256, u32>,
    // Next index to assign to a new peer
    next_peer_idx: u32,
    tx_broadcasting_time_interval: u64,
}

pub async fn send_tx_hashes(
    txs: Vec<MempoolTransaction>,
    capabilities: Vec<Capability>,
    connection: &mut PeerConnection,
    peer_id: H256,
    blockchain: &Arc<Blockchain>,
) -> Result<(), TxBroadcasterError> {
    if SUPPORTED_ETH_CAPABILITIES
        .iter()
        .any(|cap| capabilities.contains(cap))
    {
        for tx_chunk in txs.chunks(NEW_POOLED_TRANSACTION_HASHES_SOFT_LIMIT) {
            let tx_count = tx_chunk.len();
            let mut txs_to_send = Vec::with_capacity(tx_count);
            for tx in tx_chunk {
                txs_to_send.push((**tx).clone());
            }
            let hashes_message = Message::NewPooledTransactionHashes(
                NewPooledTransactionHashes::new(txs_to_send, blockchain)?,
            );
            connection.outgoing_message(hashes_message.clone()).await.unwrap_or_else(|err| {
                error!(peer_id = %format!("{:#x}", peer_id), err = ?err, "Failed to send transactions hashes");
            });
        }
    }
    Ok(())
}

#[actor(protocol = TxBroadcasterProtocol)]
impl TxBroadcaster {
    pub fn spawn(
        kademlia: PeerTable,
        blockchain: Arc<Blockchain>,
        tx_broadcasting_time_interval: u64,
    ) -> Result<ActorRef<TxBroadcaster>, TxBroadcasterError> {
        info!("Starting Transaction Broadcaster");

        let state = TxBroadcaster {
            peer_table: kademlia,
            blockchain,
            known_txs: HashMap::new(),
            peer_indexer: HashMap::new(),
            next_peer_idx: 0,
            tx_broadcasting_time_interval,
        };

        Ok(state.start())
    }

    #[started]
    async fn started(&mut self, ctx: &Context<Self>) {
        send_interval(
            Duration::from_millis(self.tx_broadcasting_time_interval),
            ctx.clone(),
            tx_broadcaster_protocol::BroadcastTxs,
        );

        send_interval(
            Duration::from_secs(PRUNE_INTERVAL_SECS),
            ctx.clone(),
            tx_broadcaster_protocol::PruneTxs,
        );
    }

    #[send_handler]
    async fn handle_broadcast_txs(
        &mut self,
        _msg: tx_broadcaster_protocol::BroadcastTxs,
        _ctx: &Context<Self>,
    ) {
        trace!(received = "BroadcastTxs");

        let _ = self.do_broadcast_txs().await.inspect_err(|_| {
            error!("Failed to broadcast transactions");
        });
    }

    #[send_handler]
    async fn handle_add_txs(&mut self, msg: tx_broadcaster_protocol::AddTxs, _ctx: &Context<Self>) {
        debug!(received = "AddTxs", tx_count = msg.tx_hashes.len());
        self.do_add_txs(msg.tx_hashes, msg.peer_id);
    }

    #[send_handler]
    async fn handle_prune_txs(
        &mut self,
        _msg: tx_broadcaster_protocol::PruneTxs,
        _ctx: &Context<Self>,
    ) {
        debug!(received = "PruneTxs");
        let now = Instant::now();
        let before = self.known_txs.len();
        let prune_window = Duration::from_secs(PRUNE_WAIT_TIME_SECS);

        self.known_txs
            .retain(|_, record| now.duration_since(record.last_sent) < prune_window);
        debug!(
            before = before,
            after = self.known_txs.len(),
            "Pruned old broadcasted transactions"
        );
    }

    // Get or assign a unique index to the peer_id
    #[inline]
    fn peer_index(&mut self, peer_id: H256) -> u32 {
        if let Some(&idx) = self.peer_indexer.get(&peer_id) {
            idx
        } else {
            // We are assigning indexes sequentially, so next_peer_idx is always the next available one.
            // self.peer_indexer.len() could be used instead of next_peer_idx but avoided here if we ever
            // remove entries from peer_indexer in the future.
            let idx = self.next_peer_idx;
            // In practice we won't exceed u32::MAX (~4.29 Billion) peers.
            self.next_peer_idx += 1;
            self.peer_indexer.insert(peer_id, idx);
            idx
        }
    }

    fn do_add_txs(&mut self, txs: Vec<H256>, peer_id: H256) {
        debug!(total = self.known_txs.len(), adding = txs.len(), peer_id = %format!("{:#x}", peer_id), "Adding transactions to known list");

        if txs.is_empty() {
            return;
        }

        let now = Instant::now();
        let peer_idx = self.peer_index(peer_id);
        for tx in txs {
            let record = self.known_txs.entry(tx).or_default();
            record.peers.set(peer_idx);
            record.last_sent = now;
        }
    }

    async fn do_broadcast_txs(&mut self) -> Result<(), TxBroadcasterError> {
        let txs_to_broadcast = self
            .blockchain
            .mempool
            .get_txs_for_broadcast()
            .map_err(|_| TxBroadcasterError::Broadcast)?;
        if txs_to_broadcast.is_empty() {
            return Ok(());
        }
        let peers = self.peer_table.get_peers_with_capabilities().await?;
        let peer_sqrt = (peers.len() as f64).sqrt();

        let full_txs = txs_to_broadcast
            .iter()
            .map(|tx| tx.transaction().clone())
            .filter(|tx| {
                !matches!(tx, Transaction::EIP4844Transaction { .. }) && !tx.is_privileged()
            })
            .collect::<Vec<Transaction>>();

        let blob_txs = txs_to_broadcast
            .iter()
            .filter(|tx| matches!(tx.transaction(), Transaction::EIP4844Transaction { .. }))
            .cloned()
            .collect::<Vec<MempoolTransaction>>();

        let mut shuffled_peers = peers.clone();
        shuffled_peers.shuffle(&mut thread_rng());

        let (peers_to_send_full_txs, peers_to_send_hashes) =
            shuffled_peers.split_at(peer_sqrt.ceil() as usize);

        for (peer_id, mut connection, capabilities) in peers_to_send_full_txs.iter().cloned() {
            let peer_idx = self.peer_index(peer_id);
            let txs_to_send = full_txs
                .iter()
                .filter(|tx| {
                    let hash = tx.hash();
                    !self
                        .known_txs
                        .get(&hash)
                        .is_some_and(|record| record.peers.is_set(peer_idx))
                })
                .cloned()
                .collect::<Vec<Transaction>>();
            self.do_add_txs(txs_to_send.iter().map(|tx| tx.hash()).collect(), peer_id);
            // If a peer is selected to receive the full transactions, we don't send the blob transactions, since they only require to send the hashes
            let txs_message = Message::Transactions(Transactions {
                transactions: txs_to_send,
            });
            connection.outgoing_message(txs_message).await.unwrap_or_else(|err| {
                error!(peer_id = %format!("{:#x}", peer_id), err = ?err, "Failed to send transactions");
            });
            self.send_tx_hashes_internal(blob_txs.clone(), capabilities, &mut connection, peer_id)
                .await?;
        }
        for (peer_id, mut connection, capabilities) in peers_to_send_hashes.iter().cloned() {
            // If a peer is not selected to receive the full transactions, we only send the hashes of all transactions (including blob transactions)
            self.send_tx_hashes_internal(
                txs_to_broadcast.clone(),
                capabilities,
                &mut connection,
                peer_id,
            )
            .await?;
        }
        let broadcasted_hashes: Vec<H256> = txs_to_broadcast.iter().map(|tx| tx.hash()).collect();
        self.blockchain
            .mempool
            .remove_broadcasted_txs(&broadcasted_hashes)?;
        Ok(())
    }

    async fn send_tx_hashes_internal(
        &mut self,
        txs: Vec<MempoolTransaction>,
        capabilities: Vec<Capability>,
        connection: &mut PeerConnection,
        peer_id: H256,
    ) -> Result<(), TxBroadcasterError> {
        let peer_idx = self.peer_index(peer_id);
        let txs_to_send = txs
            .iter()
            .filter(|tx| {
                let hash = tx.hash();
                !self
                    .known_txs
                    .get(&hash)
                    .is_some_and(|record| record.peers.is_set(peer_idx))
                    && !tx.is_privileged()
            })
            .cloned()
            .collect::<Vec<MempoolTransaction>>();
        self.do_add_txs(txs_to_send.iter().map(|tx| tx.hash()).collect(), peer_id);
        send_tx_hashes(
            txs_to_send,
            capabilities,
            connection,
            peer_id,
            &self.blockchain,
        )
        .await
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TxBroadcasterError {
    #[error("Failed to broadcast transactions")]
    Broadcast,
    #[error(transparent)]
    StoreError(#[from] StoreError),
    #[error(transparent)]
    PeerTableError(#[from] ActorError),
}
