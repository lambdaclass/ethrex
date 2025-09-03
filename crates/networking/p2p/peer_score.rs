use super::peer_handler::PeerHandlerError;
use crate::kademlia;
use crate::kademlia::PeerChannels;
use crate::rlpx::p2p::Capability;
use ethrex_common::H256;
use std::collections::HashMap;

const MAX_SCORE: i64 = 50;
const MIN_SCORE: i64 = -50;

#[derive(Debug, Clone, Default)]
pub struct PeerScores {
    scores: HashMap<H256, i64>,
}

impl PeerScores {
    pub fn new() -> Self {
        Self {
            scores: HashMap::default(),
        }
    }

    pub fn get_score(&self, peer_id: &H256) -> i64 {
        *self.scores.get(peer_id).unwrap_or(&0)
    }

    pub fn get_score_opt(&self, peer_id: &H256) -> Option<i64> {
        self.scores.get(peer_id).copied()
    }

    pub fn check_or_insert(&mut self, peer_id: H256, value: i64) {
        self.scores.entry(peer_id).or_insert(value);
    }

    pub fn record_success(&mut self, peer_id: H256) {
        let score = self.scores.entry(peer_id).or_insert(0);
        *score = score.saturating_add(1);
        if *score > MAX_SCORE {
            *score = MAX_SCORE;
        }
    }

    pub fn record_failure(&mut self, peer_id: H256) {
        let score = self.scores.entry(peer_id).or_insert(0);
        *score = score.saturating_sub(1);
        if *score < MIN_SCORE {
            *score = MIN_SCORE;
        }
    }

    pub fn record_critical_failure(&mut self, peer_id: H256) {
        self.scores.insert(peer_id, i64::MIN);
    }
}

impl PeerScores {
    /// Returns the peer with the highest score.
    /// If `update_peers_from_kademlia` is true, it updates the peer list
    /// from the Kademlia table before selecting the best peer.
    pub async fn get_best_peer<'a>(
        &'a mut self,
        kademlia_table: &kademlia::Kademlia,
        update_peers_from_kademlia: bool,
    ) -> Option<&'a H256> {
        if update_peers_from_kademlia {
            // update peers from Kademlia
            for peer_id in kademlia_table.peers.lock().await.keys() {
                self.scores.entry(*peer_id).or_insert(0);
            }
        }

        self.scores
            .iter()
            .max_by(|(_k1, v1), (_k2, v2)| v1.cmp(v2))
            .map(|(k, _v)| k)
    }

    pub async fn get_peer_channel_with_highest_score(
        &self,
        kademlia_table: &kademlia::Kademlia,
        _capabilities: &[Capability],
    ) -> Result<Option<(H256, Option<PeerChannels>)>, PeerHandlerError> {
        let best_peer = self
            .scores
            .iter()
            .max_by(|(_k1, v1), (_k2, v2)| v1.cmp(v2))
            .map(|(k, _v)| k)
            .ok_or(PeerHandlerError::NoPeers)?;

        if let Some(channel) = kademlia_table.peers.lock().await.get(best_peer) {
            return Ok(Some((*best_peer, channel.channels.clone())));
        }

        Ok(None)
    }
}
