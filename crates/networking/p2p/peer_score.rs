use ethrex_common::H256;
use std::{
    collections::HashMap,
};

use crate::kademlia;

#[derive(Debug, Clone, Default)]
pub struct PeerScores {
    scores: HashMap<H256, i64>,
}

#[async_trait::async_trait]
impl PeerScores {
    pub fn new() -> Self {
        Self {
            scores: HashMap::default(),
        }
    }

    pub fn get_score(&self, peer_id: &H256) -> i64 {
        *self.scores.get(peer_id).unwrap_or(&0)
    }

    pub fn record_success(&mut self, peer_id: H256) {
        let score = self.scores.entry(peer_id).or_insert(0);
        *score = score.saturating_add(1);
        if *score > 50 {
            *score = 50;
        }
    }

    pub fn record_failure(&mut self, peer_id: H256) {
        let score = self.scores.entry(peer_id).or_insert(0);
        *score = score.saturating_sub(1);
        if *score < -50 {
            *score = -50;
        }
    }

    pub fn record_critical_failure(&mut self, peer_id: H256) {
        self.scores.insert(peer_id, i64::MIN);
    }

    /// Returns the peer with the highest score.
    /// If `update_peers_from_kademlia` is true, it updates the peer list 
    /// from the Kademlia table before selecting the best peer.
    pub async fn get_best_peer(&self, kademlia_table: &kademlia::Kademlia, update_peers_from_kademlia: bool) -> Option<H256> {
        if update_peers_from_kademlia {
            // update peers from Kademlia
            for peer_id in kademlia_table.peers.lock().await.keys() {
                self.scores.entry(*peer_id).or_insert(0);
            }
        }

        self.scores.iter()
            .max_by(|(_k1, v1), (_k2, v2)| v1.cmp(v2))
            .map(|(k, _v)| k)
    }
}