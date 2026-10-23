//! The one head follower. It alone advances, reanchors and warms the canonical state, so
//! no merge is dropped behind its back.
//!
//! Each step: take the light client's heads, record them for chain proofs, pick the target
//! execution block (the light client's head, or in `require` mode the newest payload with a
//! verified chain proof), reach it with block access lists or by re-proving the pins, then
//! warm unheld keys, follow finality, and track submitted transactions.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use ethrex_common::H256;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::chain_proofs::{ChainProofs, ProofMode};
use crate::fetch::{self, CONCURRENCY};
use crate::light_client::Heads;
use crate::light_client::beacon_api::BeaconApi;
use crate::node::{NodeError, ProofSummary, Shared};
use crate::state::{Key, Plan, State};
use crate::verify::{Proven, TrustedHash, VerifiedHeader};

/// Proof requests in flight while warming, so wallet requests always find room.
pub const WARM_IN_FLIGHT: usize = CONCURRENCY / 2;
/// Facts merged per batch while warming.
pub const WARM_MERGE: usize = 64;
/// Head moves between rebroadcasts of pending transactions.
pub const REBROADCAST_EVERY: u64 = 5;
/// Bodies searched per step for submitted transactions.
pub const SEARCH_BODIES_PER_STEP: usize = 32;
/// Beacon blocks asked for proofs per step.
pub const PROOF_FETCH_PER_STEP: usize = 8;
/// Recent payloads remembered for picking a proven anchor.
const RECENT_PAYLOADS: usize = 512;

/// What a step did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stepped {
    /// The target was already the anchor.
    Current,
    /// This many blocks were applied with their access lists.
    Extended {
        /// Blocks applied.
        blocks: u64,
    },
    /// The pins were re-proven at the target.
    Reanchored,
    /// In `require` mode, nothing newer than the checkpoint is proven and the node is not
    /// anchored at a proven payload yet.
    WaitingForProof,
}

/// Persists learned keys between runs.
pub type Persist = Box<dyn Fn(&State) + Send + Sync>;

/// The follower.
pub struct Follower {
    shared: Arc<Shared>,
    heads: watch::Receiver<Heads>,
    proofs: Option<ChainProofs>,
    beacon_api: Option<BeaconApi>,
    persist: Option<Persist>,
    shutdown: CancellationToken,
    recent_payloads: VecDeque<H256>,
    last_seen_slot: u64,
    searched: HashSet<H256>,
    moves: u64,
}

impl Follower {
    /// A follower over `shared`, fed heads by `heads`. `proofs` enables chain proofs;
    /// `beacon_api` lets it fetch them.
    pub fn new(
        shared: Arc<Shared>,
        heads: watch::Receiver<Heads>,
        proofs: Option<ChainProofs>,
        beacon_api: Option<BeaconApi>,
        shutdown: CancellationToken,
    ) -> Self {
        Self {
            shared,
            heads,
            proofs,
            beacon_api,
            persist: None,
            shutdown,
            recent_payloads: VecDeque::new(),
            last_seen_slot: 0,
            searched: HashSet::new(),
            moves: 0,
        }
    }

    /// Sets what persists learned keys after a step.
    pub fn with_persist(mut self, persist: Persist) -> Self {
        self.persist = Some(persist);
        self
    }

    /// The chain proofs, for tests and status.
    pub const fn proofs(&self) -> Option<&ChainProofs> {
        self.proofs.as_ref()
    }

    /// Runs until shutdown: a step per head change, a warm per wake.
    pub async fn run(mut self) {
        loop {
            if self.shutdown.is_cancelled() {
                return;
            }
            match self.step().await {
                Ok(stepped) => debug!(?stepped, "follower step"),
                Err(err) => warn!(%err, "follower step failed; retrying at the next head"),
            }
            tokio::select! {
                _ = self.shutdown.cancelled() => return,
                changed = self.heads.changed() => {
                    if changed.is_err() {
                        return;
                    }
                }
                _ = self.shared.wake.notified() => {
                    if let Err(err) = self.warm().await {
                        debug!(%err, "warm failed");
                    }
                }
            }
        }
    }

    /// One step.
    pub async fn step(&mut self) -> Result<Stepped, NodeError> {
        let heads = *self.heads.borrow_and_update();
        self.shared.set_heads(heads);
        self.record_heads(heads).await;

        let target_hash = match self.target(heads) {
            Some(hash) => hash,
            None => {
                self.publish_proof_summary();
                self.warm().await?;
                return Ok(Stepped::WaitingForProof);
            }
        };
        let anchor = self.shared.read(|state| state.anchor().clone());
        let stepped = if target_hash == anchor.hash() {
            Stepped::Current
        } else {
            let peers = self.shared.peers();
            let target = fetch::header(peers.as_ref(), TrustedHash::attested(target_hash)).await?;
            let plan = self.shared.read(|state| state.plan(&target));
            let stepped = match plan {
                Plan::Current => Stepped::Current,
                Plan::Extend { blocks } => match self.extend(&anchor, target.clone()).await {
                    Ok(()) => Stepped::Extended { blocks },
                    Err(err) => {
                        debug!(%err, "cannot extend with block access lists; reanchoring");
                        self.reanchor(target).await?;
                        Stepped::Reanchored
                    }
                },
                Plan::Reanchor => {
                    self.reanchor(target).await?;
                    Stepped::Reanchored
                }
            };
            if stepped != Stepped::Current {
                self.moves = self.moves.saturating_add(1);
            }
            stepped
        };
        self.publish_proof_summary();
        self.warm().await?;
        self.follow_finalized(heads).await;
        self.track_transactions().await;
        if let Some(persist) = &self.persist {
            self.shared.read(|state| persist(state));
        }
        let store = self.shared.read(|state| state.snapshot());
        self.shared.peers().announce(&store).await;
        Ok(stepped)
    }

    async fn record_heads(&mut self, heads: Heads) {
        if heads.optimistic.slot < self.last_seen_slot
            && let Some(proofs) = &mut self.proofs
        {
            warn!(
                from = self.last_seen_slot,
                to = heads.optimistic.slot,
                "light client head went backwards; forgetting proofs past it"
            );
            proofs.tracker_mut().forget_from(heads.optimistic.slot);
        }
        self.last_seen_slot = heads.optimistic.slot;
        let payload = heads.optimistic.execution_block_hash;
        if self.recent_payloads.front() != Some(&payload) {
            self.recent_payloads.retain(|p| *p != payload);
            self.recent_payloads.push_front(payload);
            self.recent_payloads.truncate(RECENT_PAYLOADS);
        }
        if let Some(proofs) = &mut self.proofs {
            proofs.tracker_mut().observe(heads.finalized);
            proofs.tracker_mut().observe(heads.optimistic);
            if let Some(api) = &self.beacon_api {
                let accepted = proofs.fetch_from(api, PROOF_FETCH_PER_STEP).await;
                if accepted > 0 {
                    info!(accepted, "execution chain proofs accepted");
                }
            }
        }
    }

    /// The execution block to anchor at, or `None` when `require` mode has no proven one.
    fn target(&self, heads: Heads) -> Option<H256> {
        match self.shared.proof_mode() {
            ProofMode::Off | ProofMode::Observe => Some(heads.optimistic.execution_block_hash),
            ProofMode::Require => {
                let proofs = self.proofs.as_ref()?;
                proofs.tracker().newest_proven(self.recent_payloads.iter())
            }
        }
    }

    fn publish_proof_summary(&self) {
        let Some(proofs) = &self.proofs else {
            return;
        };
        let anchor = self.shared.read(|state| state.anchor().hash());
        self.shared.set_proofs(ProofSummary {
            verified: proofs.tracker().proof_count(),
            proven_payloads: proofs.tracker().proven_count(),
            anchor_proven: proofs.tracker().is_proven(anchor),
        });
    }

    async fn extend(
        &self,
        anchor: &VerifiedHeader,
        target: VerifiedHeader,
    ) -> Result<(), NodeError> {
        let peers = self.shared.peers();
        let chain = fetch::chain(peers.as_ref(), anchor, target).await?;
        let blocks = fetch::bals(peers.as_ref(), chain).await?;
        self.shared.update(|state| {
            for block in blocks {
                state
                    .advance(block)
                    .map_err(|e| NodeError::Fetch(e.to_string()))?;
            }
            Ok(())
        })
    }

    /// Proves the pins at `target` into a fresh store. A pin that cannot be proven never
    /// blocks the move: warming retries it.
    async fn reanchor(&self, target: VerifiedHeader) -> Result<(), NodeError> {
        let pins: Vec<Key> = self.shared.read(|state| state.working().pins().collect());
        let results = fetch::each(self.shared.peers(), &target, pins, CONCURRENCY).await;
        let mut facts = Vec::with_capacity(results.len());
        for (key, result) in results {
            match result {
                Ok(proven) => facts.push(proven),
                Err(err) => debug!(?key, %err, "pin unproven at the new head; warming retries it"),
            }
        }
        self.shared.update(|state| {
            state
                .reanchor(target, &facts)
                .map_err(|e| NodeError::Fetch(e.to_string()))
        })?;
        Ok(())
    }

    /// Proves working-set keys the store lacks, pins first, merging as it goes. Stops when
    /// the head moves.
    pub async fn warm(&mut self) -> Result<(), NodeError> {
        let (anchor, keys) = self.shared.read(|state| {
            let keys: Vec<Key> = state.unheld().collect();
            (state.anchor().clone(), keys)
        });
        if keys.is_empty() {
            return Ok(());
        }
        let keys = warm_order(keys);
        for chunk in keys.chunks(WARM_MERGE) {
            if self.heads.has_changed().unwrap_or(true) || self.shutdown.is_cancelled() {
                return Ok(());
            }
            let results = fetch::each(
                self.shared.peers(),
                &anchor,
                chunk.iter().copied(),
                WARM_IN_FLIGHT,
            )
            .await;
            let facts: Vec<Proven> = results
                .into_iter()
                .filter_map(|(key, result)| match result {
                    Ok(proven) => Some(proven),
                    Err(err) => {
                        debug!(?key, %err, "key left unheld");
                        None
                    }
                })
                .collect();
            self.shared.update(|state| {
                state.merge(anchor.hash(), &facts);
            });
        }
        Ok(())
    }

    async fn follow_finalized(&self, heads: Heads) {
        let known = self
            .shared
            .read(|state| state.finalized().map(VerifiedHeader::hash));
        let wanted = heads.finalized.execution_block_hash;
        if known == Some(wanted) {
            return;
        }
        let peers = self.shared.peers();
        match fetch::header(peers.as_ref(), TrustedHash::attested(wanted)).await {
            Ok(header) => self.shared.update(|state| state.set_finalized(header)),
            Err(err) => debug!(%err, "finalized header not fetched"),
        }
    }

    /// Searches held blocks for pending transactions, fetches receipts of located ones, and
    /// rebroadcasts what is still pending every few head moves.
    async fn track_transactions(&mut self) {
        let (pending_since, anchor_number, headers) = self.shared.read(|state| {
            let store = state.store();
            let since = state.txs().pending().iter().map(|p| p.since.number).min();
            let headers: Vec<VerifiedHeader> = store.headers().cloned().collect();
            (since, store.anchor().number(), headers)
        });
        let peers = self.shared.peers();
        if let Some(since) = pending_since {
            let candidates: Vec<VerifiedHeader> = headers
                .iter()
                .filter(|h| h.number() > since && h.number() <= anchor_number)
                .filter(|h| !self.searched.contains(&h.hash()))
                .take(SEARCH_BODIES_PER_STEP)
                .cloned()
                .collect();
            if !candidates.is_empty() {
                match fetch::bodies(peers.as_ref(), candidates).await {
                    Ok(bodies) => {
                        for body in bodies {
                            let header = body.header().clone();
                            self.searched.insert(header.hash());
                            self.shared.update(|state| {
                                state
                                    .txs_mut()
                                    .locate((&header).into(), &body.body().transactions);
                            });
                        }
                    }
                    Err(err) => debug!(%err, "body search failed"),
                }
            }
            if self.searched.len() > 4096 {
                let keep: HashSet<H256> = headers.iter().map(VerifiedHeader::hash).collect();
                self.searched.retain(|h| keep.contains(h));
            }
        }
        let awaiting = self
            .shared
            .read(|state| state.txs().blocks_awaiting_receipts());
        for block in awaiting {
            let Some(header) = headers.iter().find(|h| h.hash() == block.hash).cloned() else {
                continue;
            };
            let body = match fetch::body(peers.as_ref(), header.clone()).await {
                Ok(body) => body,
                Err(err) => {
                    debug!(%err, "body for receipts not fetched");
                    continue;
                }
            };
            let receipts = match fetch::receipts(peers.as_ref(), header.clone()).await {
                Ok(receipts) => receipts,
                Err(err) => {
                    debug!(%err, "receipts not fetched");
                    continue;
                }
            };
            let located = self.shared.read(|state| state.txs().located_in(block.hash));
            for (hash, index) in located {
                let Ok(index) = usize::try_from(index) else {
                    continue;
                };
                if let Some(receipt) = receipts.receipt(&body, index) {
                    self.shared
                        .update(|state| state.txs_mut().set_receipt(hash, receipt));
                }
            }
        }
        if self.moves > 0 && self.moves.is_multiple_of(REBROADCAST_EVERY) {
            let pending: Vec<_> = self
                .shared
                .read(|state| state.txs().pending().iter().map(|p| p.tx.clone()).collect());
            for tx in pending {
                if let Err(err) = peers.broadcast(tx).await {
                    debug!(%err, "rebroadcast failed");
                }
            }
        }
        let dropped = self.shared.update(|state| {
            let snapshot = state.snapshot();
            state
                .txs_mut()
                .prune_replaced(|address| held_nonce_in(&snapshot, address))
        });
        if !dropped.is_empty() {
            debug!(
                count = dropped.len(),
                "pending transactions replaced by nonce"
            );
        }
    }
}

fn held_nonce_in(store: &crate::state::Store, address: ethrex_common::Address) -> Option<u64> {
    store.account(address).ok().flatten().map(|a| a.nonce)
}

/// Pins first, then learned keys as given; an account whose slot is listed needs no entry
/// of its own, since a slot proof carries the account.
fn warm_order(keys: Vec<Key>) -> Vec<Key> {
    let with_slots: HashSet<ethrex_common::Address> = keys
        .iter()
        .filter_map(|key| match key {
            Key::Slot(address, _) => Some(*address),
            Key::Account(_) => None,
        })
        .collect();
    keys.into_iter()
        .filter(|key| match key {
            Key::Account(address) => !with_slots.contains(address),
            Key::Slot(..) => true,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    use ethrex_common::types::{GenericTransaction, TxKind};
    use ethrex_common::{H256, U256};
    use tokio::sync::{Mutex as AsyncMutex, watch};
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::chain_proofs::engine::MockProofEngine;
    use crate::chain_proofs::tracker::ProofTracker;
    use crate::chain_proofs::types::{
        ExecutionProof, MockProof, PublicInput, SignedExecutionProof,
    };
    use crate::light_client::Heads;
    use crate::node::{BlockSel, Node, Shared};
    use crate::state::{Key, Miss};
    use crate::testing::{Fault, LIAR, World, WorldPeers, beacon_root, eth, lc_head_for};
    use crate::working::{WORKING_SET_CAP, WorkingSet};
    use ethrex_common::types::Genesis;

    const NOBODY: ethrex_common::Address = ethrex_common::Address::repeat_byte(0xde);
    const GAS_CAP: u64 = 50_000_000;

    fn chain_config() -> ethrex_common::types::ChainConfig {
        let genesis: Genesis =
            serde_json::from_str(include_str!("../../../fixtures/genesis/l1-bal.json")).unwrap();
        genesis.config
    }

    struct Harness {
        world: Arc<AsyncMutex<World>>,
        peers: Arc<WorldPeers>,
        shared: Arc<Shared>,
        node: Node,
        heads_tx: watch::Sender<Heads>,
        follower: Follower,
    }

    async fn heads_at(world: &Arc<AsyncMutex<World>>, number: u64) -> Heads {
        let world = world.lock().await;
        Heads {
            optimistic: lc_head_for(&world, number),
            finalized: lc_head_for(&world, 0),
        }
    }

    async fn harness(pins: Vec<Key>, mode: ProofMode) -> Harness {
        let world = Arc::new(AsyncMutex::new(World::new().await));
        let peers = WorldPeers::new(Arc::clone(&world));
        let heads = heads_at(&world, 0).await;
        let anchor = world
            .lock()
            .await
            .verified(heads.optimistic.execution_block_hash);
        let working = WorkingSet::new(WORKING_SET_CAP, pins, Vec::new());
        let peers_dyn: Arc<dyn crate::peers::Peers> = peers.clone();
        let shared = Shared::new(anchor, working, peers_dyn, chain_config(), GAS_CAP, mode);
        let node = Node::new(Arc::clone(&shared), "test".into());
        let (heads_tx, heads_rx) = watch::channel(heads);
        let proofs = (mode != ProofMode::Off).then(|| {
            ChainProofs::new(
                Arc::new(MockProofEngine),
                ProofTracker::new(heads.finalized),
                chain_config().chain_id,
            )
        });
        let follower = Follower::new(
            Arc::clone(&shared),
            heads_rx,
            proofs,
            None,
            CancellationToken::new(),
        );
        Harness {
            world,
            peers,
            shared,
            node,
            heads_tx,
            follower,
        }
    }

    async fn mine(h: &Harness, txs: Vec<ethrex_common::types::Transaction>) -> H256 {
        let mut world = h.world.lock().await;
        let hash = world.mine(txs).await;
        assert!(world.rejected.is_empty(), "{:?}", world.rejected);
        hash
    }

    async fn advance_heads(h: &Harness) {
        let number = {
            let world = h.world.lock().await;
            world.header(world.head()).number
        };
        h.heads_tx.send(heads_at(&h.world, number).await).unwrap();
    }

    #[tokio::test]
    async fn blocks_with_access_lists_extend_without_proofs() {
        let (alice, bob) = {
            let w = World::new().await;
            (w.signer(0).address, w.signer(1).address)
        };
        let mut h = harness(vec![Key::Account(alice), Key::Account(bob)], ProofMode::Off).await;
        assert_eq!(h.follower.step().await.unwrap(), Stepped::Current);
        let before = h
            .node
            .account(bob, BlockSel::Latest)
            .await
            .unwrap()
            .unwrap()
            .balance;
        let proofs_before = h.peers.account_requests.load(Ordering::SeqCst);

        let tx = h.world.lock().await.transfer(0, 0, bob, eth(1));
        mine(&h, vec![tx]).await;
        let tx = h.world.lock().await.transfer(0, 1, bob, eth(1));
        mine(&h, vec![tx]).await;
        advance_heads(&h).await;
        assert_eq!(
            h.follower.step().await.unwrap(),
            Stepped::Extended { blocks: 2 }
        );
        assert_eq!(
            h.peers.account_requests.load(Ordering::SeqCst),
            proofs_before,
            "no proofs needed"
        );
        assert_eq!(h.peers.bal_requests.load(Ordering::SeqCst), 1);
        let after = h
            .node
            .account(bob, BlockSel::Latest)
            .await
            .unwrap()
            .unwrap()
            .balance;
        assert_eq!(after, before + eth(2));
        assert_eq!(h.node.head().number(), 2);
        // The block before the head is retained and serves its own state.
        let mid = h
            .node
            .account(bob, BlockSel::Number(1))
            .await
            .unwrap()
            .unwrap()
            .balance;
        assert_eq!(mid, before + eth(1));
    }

    #[tokio::test]
    async fn a_withheld_access_list_reanchors_and_reproves_the_pins() {
        let alice = World::new().await.signer(0).address;
        let mut h = harness(vec![Key::Account(alice)], ProofMode::Off).await;
        h.follower.step().await.unwrap();
        let tx = h.world.lock().await.transfer(0, 0, NOBODY, eth(1));
        mine(&h, vec![tx]).await;
        h.peers.fault(Fault::WithholdBals);
        advance_heads(&h).await;
        assert_eq!(h.follower.step().await.unwrap(), Stepped::Reanchored);
        assert!(
            h.peers.penalized.lock().unwrap().is_empty(),
            "withholding blames nobody"
        );
        let nonce = h
            .node
            .account(alice, BlockSel::Latest)
            .await
            .unwrap()
            .unwrap()
            .nonce;
        assert_eq!(nonce, 1);
        assert_eq!(h.shared.read(|s| s.store().account_count()), 1);
    }

    #[tokio::test]
    async fn gaps_past_the_bal_window_reanchor_and_learned_keys_warm_back() {
        let (alice, stranger) = {
            let w = World::new().await;
            (w.signer(0).address, w.signer(2).address)
        };
        let mut h = harness(vec![Key::Account(alice)], ProofMode::Off).await;
        h.follower.step().await.unwrap();
        // A read learns the stranger.
        h.node.account(stranger, BlockSel::Latest).await.unwrap();
        assert!(h.shared.read(|s| s.store().holds(Key::Account(stranger))));
        for i in 0..=crate::state::MAX_BAL_GAP {
            let tx = h.world.lock().await.transfer(1, i, NOBODY, eth(1));
            mine(&h, vec![tx]).await;
        }
        advance_heads(&h).await;
        assert_eq!(h.follower.step().await.unwrap(), Stepped::Reanchored);
        // Pins were proven in the step, learned keys warmed right after.
        assert!(h.shared.read(|s| s.store().holds(Key::Account(alice))));
        assert!(h.shared.read(|s| s.store().holds(Key::Account(stranger))));
        assert_eq!(h.node.head().number(), crate::state::MAX_BAL_GAP + 1);
    }

    #[tokio::test]
    async fn a_lying_peer_is_penalized_and_the_value_comes_from_an_honest_retry() {
        let alice = World::new().await.signer(0).address;
        let mut h = harness(vec![], ProofMode::Off).await;
        h.follower.step().await.unwrap();
        h.peers.fault(Fault::LieAboutBalance(alice, eth(999)));
        let result = h.node.account(alice, BlockSel::Latest).await;
        assert!(
            matches!(result, Err(crate::node::NodeError::Unprovable(_))),
            "{result:?}"
        );
        let penalized = h.peers.penalized.lock().unwrap().clone();
        assert!(!penalized.is_empty() && penalized.iter().all(|p| *p == LIAR));
        h.peers.heal();
        let balance = h
            .node
            .account(alice, BlockSel::Latest)
            .await
            .unwrap()
            .unwrap()
            .balance;
        assert_eq!(
            balance,
            h.world.lock().await.account_at_head(alice).unwrap().balance
        );
    }

    #[tokio::test]
    async fn unavailable_keys_are_errors_and_heal_later() {
        let alice = World::new().await.signer(0).address;
        let mut h = harness(vec![], ProofMode::Off).await;
        h.follower.step().await.unwrap();
        h.peers.fault(Fault::Unavailable(alice));
        let result = h.node.storage(alice, U256::one(), BlockSel::Latest).await;
        assert!(
            matches!(result, Err(crate::node::NodeError::Unprovable(_))),
            "{result:?}"
        );
        assert!(h.peers.penalized.lock().unwrap().is_empty());
        h.peers.heal();
        assert_eq!(
            h.node
                .storage(alice, U256::one(), BlockSel::Latest)
                .await
                .unwrap(),
            U256::zero()
        );
    }

    #[tokio::test]
    async fn a_call_through_the_node_learns_its_keys_and_the_follower_keeps_them_current() {
        let (alice, bob) = {
            let w = World::new().await;
            (w.signer(0).address, w.signer(1).address)
        };
        let mut h = harness(vec![], ProofMode::Off).await;
        h.follower.step().await.unwrap();
        let tx = GenericTransaction {
            from: alice,
            to: TxKind::Call(bob),
            value: eth(1),
            ..Default::default()
        };
        let result = h.node.call(tx, BlockSel::Latest).await.unwrap();
        assert!(result.is_success());
        assert!(h.shared.read(|s| s.working().contains(Key::Account(bob))));
        let before = h
            .node
            .account(bob, BlockSel::Latest)
            .await
            .unwrap()
            .unwrap()
            .balance;
        let transfer = h.world.lock().await.transfer(0, 0, bob, eth(3));
        mine(&h, vec![transfer]).await;
        advance_heads(&h).await;
        let proofs_before = h.peers.account_requests.load(Ordering::SeqCst);
        h.follower.step().await.unwrap();
        assert_eq!(
            h.peers.account_requests.load(Ordering::SeqCst),
            proofs_before
        );
        let after = h
            .node
            .account(bob, BlockSel::Latest)
            .await
            .unwrap()
            .unwrap()
            .balance;
        assert_eq!(after, before + eth(3));
    }

    #[tokio::test]
    async fn a_submitted_transaction_is_located_and_gets_a_verified_receipt() {
        let bob = World::new().await.signer(1).address;
        let mut h = harness(vec![], ProofMode::Off).await;
        h.follower.step().await.unwrap();
        let tx = h.world.lock().await.transfer(0, 0, bob, eth(1));
        let raw = tx.encode_canonical_to_vec();
        let hash = h.node.submit(&raw).await.unwrap();
        assert_eq!(h.peers.broadcasts.lock().unwrap().len(), 1);
        assert!(matches!(
            h.node.transaction(hash),
            Some(crate::txbook::SubmittedTx::Pending(_))
        ));
        let sender = h.world.lock().await.signer(0).address;
        assert_eq!(
            h.node
                .transaction_count(sender, BlockSel::Pending)
                .await
                .unwrap(),
            1
        );
        // The world includes it in the next block.
        mine(&h, vec![tx]).await;
        advance_heads(&h).await;
        h.follower.step().await.unwrap();
        let receipt = h.node.receipt(hash).expect("receipt verified");
        assert!(receipt.receipt().succeeded);
        assert_eq!(receipt.block().number, 1);
        assert!(matches!(
            h.node.transaction(hash),
            Some(crate::txbook::SubmittedTx::Located(_))
        ));
        // A transaction for another chain is refused before anything else is checked.
        let wrong_chain = ethrex_common::types::Transaction::EIP1559Transaction(
            ethrex_common::types::EIP1559Transaction {
                chain_id: h.node.chain_id() + 1,
                nonce: 1,
                max_fee_per_gas: 1,
                gas_limit: 21_000,
                to: TxKind::Call(bob),
                ..Default::default()
            },
        );
        assert!(
            h.node
                .submit(&wrong_chain.encode_canonical_to_vec())
                .await
                .is_err()
        );
    }

    fn mock_proof(origin: H256, head: H256, chain_id: u64) -> SignedExecutionProof {
        let input = PublicInput::new(origin, head, chain_id);
        SignedExecutionProof {
            message: ExecutionProof {
                proof_data: MockProof::claiming(&input)
                    .unwrap()
                    .encode()
                    .try_into()
                    .unwrap(),
                proof_type: 3,
                origin_block_root: origin.0,
                head_block_root: head.0,
            },
            validator_index: 0,
            signature: [0u8; 96],
        }
    }

    #[tokio::test]
    async fn require_mode_anchors_only_at_execution_proven_payloads() {
        let bob = World::new().await.signer(1).address;
        let mut h = harness(vec![Key::Account(bob)], ProofMode::Require).await;
        // At start nothing newer than the checkpoint is proven; the anchor is the checkpoint,
        // which counts as proven by trust.
        assert_eq!(h.follower.step().await.unwrap(), Stepped::Current);
        assert_eq!(h.node.head().number(), 0);
        assert!(h.node.status().await.anchor_proven);
        for i in 0..3 {
            let tx = h.world.lock().await.transfer(0, i, bob, eth(1));
            mine(&h, vec![tx]).await;
        }
        // Heads for beacon blocks 1, 2, 3 arrive; each vouches for the payload before it.
        for n in 1..=3 {
            h.heads_tx.send(heads_at(&h.world, n).await).unwrap();
            assert_eq!(h.follower.step().await.unwrap(), Stepped::Current);
        }
        assert_eq!(h.node.head().number(), 0);
        let status = h.node.status().await;
        assert_eq!(status.proof_mode, ProofMode::Require);
        assert_eq!(status.proven_payloads, 0);

        // A proof for beacon block 1 (origin = checkpoint) proves payload 2, the payload
        // beacon block 2 reports as applied.
        let proof = mock_proof(beacon_root(0), beacon_root(1), chain_config().chain_id);
        let accepted = h
            .follower
            .proofs
            .as_mut()
            .unwrap()
            .submit(&proof, None)
            .await
            .unwrap();
        let block2 = h
            .world
            .lock()
            .await
            .store()
            .get_block_header(2)
            .unwrap()
            .unwrap()
            .hash();
        assert_eq!(accepted.execution_block_hash, Some(block2));
        assert_eq!(
            h.follower.step().await.unwrap(),
            Stepped::Extended { blocks: 2 }
        );
        assert_eq!(h.node.head().number(), 2);
        let status = h.node.status().await;
        assert!(status.anchor_proven);
        assert_eq!(status.proven_payloads, 1);

        // A proof whose origin is after the checkpoint does not move the anchor.
        let recent = mock_proof(beacon_root(2), beacon_root(2), chain_config().chain_id);
        assert!(
            h.follower
                .proofs
                .as_mut()
                .unwrap()
                .submit(&recent, None)
                .await
                .is_err()
        );
        assert_eq!(h.follower.step().await.unwrap(), Stepped::Current);
        assert_eq!(h.node.head().number(), 2);
        let expected = {
            let world = h.world.lock().await;
            let root = world.header(block2).state_root;
            world
                .store()
                .get_account_state_by_root(root, bob)
                .unwrap()
                .unwrap()
                .balance
        };
        assert_eq!(
            h.node
                .account(bob, BlockSel::Latest)
                .await
                .unwrap()
                .unwrap()
                .balance,
            expected
        );
    }

    #[tokio::test]
    async fn observe_mode_follows_the_light_client_and_reports_proofs() {
        let bob = World::new().await.signer(1).address;
        let mut h = harness(vec![Key::Account(bob)], ProofMode::Observe).await;
        h.follower.step().await.unwrap();
        let tx = h.world.lock().await.transfer(0, 0, bob, eth(1));
        mine(&h, vec![tx]).await;
        h.heads_tx.send(heads_at(&h.world, 1).await).unwrap();
        assert_eq!(
            h.follower.step().await.unwrap(),
            Stepped::Extended { blocks: 1 }
        );
        let status = h.node.status().await;
        assert_eq!(status.proof_mode, ProofMode::Observe);
        assert_eq!(status.proofs_verified, 0);
        assert!(!status.anchor_proven);
        assert_eq!(h.node.head().number(), 1);
        let _ = Miss::Account(bob);
    }
}
