use super::peer_handler::PeerHandlerError;
use crate::kademlia;
use crate::kademlia::PeerChannels;
use crate::rlpx::p2p::Capability;
use ethrex_common::H256;
use std::collections::HashMap;
use std::i64;

const MAX_SCORE: i64 = 50;
const MIN_SCORE: i64 = -50;

#[derive(Debug, Clone, Default)]
pub struct PeerScores {
    scores: HashMap<H256, PeerScore>,
}

#[derive(Debug, Clone, Default)]
pub struct PeerScore {
    /// This tracks if a peer is being used by a task
    /// So we can't use it yet
    active: bool,
    /// This tracks the score of a peer
    score: i64,
}

impl PeerScores {
    pub fn new() -> Self {
        Self {
            scores: HashMap::default(),
        }
    }

    pub fn get_score(&self, peer_id: &H256) -> i64 {
        self.scores
            .get(peer_id)
            .map(|peer_score| peer_score.score)
            .unwrap_or(0)
    }

    pub fn get_score_opt(&self, peer_id: &H256) -> Option<i64> {
        self.scores.get(peer_id).map(|peer_score| peer_score.score)
    }

    pub fn record_success(&mut self, peer_id: H256) {
        let peer_score = self.scores.entry(peer_id).or_insert(PeerScore::default());
        peer_score.score = peer_score.score.saturating_add(1);
        if peer_score.score > MAX_SCORE {
            peer_score.score = MAX_SCORE;
        }
    }

    pub fn record_failure(&mut self, peer_id: H256) {
        let peer_score = self.scores.entry(peer_id).or_insert(PeerScore::default());
        peer_score.score = peer_score.score.saturating_sub(1);
        if peer_score.score < MIN_SCORE {
            peer_score.score = MIN_SCORE;
        }
    }

    pub fn record_critical_failure(&mut self, peer_id: H256) {
        let peer_score = self.scores.entry(peer_id).or_insert(PeerScore::default());
        peer_score.score = i64::MIN;
    }

    pub fn mark_in_use(&mut self, peer_id: H256) {
        let peer_score = self.scores.entry(peer_id).or_insert(PeerScore::default());
        peer_score.active = true;
    }

    pub fn free_peer(&mut self, peer_id: H256) {
        let peer_score = self.scores.entry(peer_id).or_insert(PeerScore::default());
        peer_score.active = false;
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
                self.scores.entry(*peer_id).or_insert(PeerScore::default());
            }
        }

        self.scores
            .iter()
            .filter(|(_, peer_score)| !peer_score.active)
            .max_by(|(_k1, v1), (_k2, v2)| v1.score.cmp(&v2.score))
            .map(|(k, _v)| k)
    }

    pub async fn get_peer_channel_with_highest_score(
        &self,
        kademlia_table: &kademlia::Kademlia,
        capabilities: &[Capability],
    ) -> Result<Option<(H256, PeerChannels)>, PeerHandlerError> {
        {
            // scope to release the lock of the peer table
            let peer_table = kademlia_table.peers.lock().await;
            let best_peer = self
                .scores
                .iter()
                .filter(|(id, _)| {
                    let supported_caps = match &peer_table.get(id) {
                        Some(peer_data) => &peer_data.supported_capabilities,
                        None => &Vec::new(),
                    };

                    capabilities.iter().all(|cap| supported_caps.contains(cap))
                })
                .max_by(|(_k1, v1), (_k2, v2)| v1.score.cmp(&v2.score))
                .map(|(k, _v)| k)
                .ok_or(PeerHandlerError::NoPeers)?;

            if let Some(channel) = peer_table.get(best_peer) {
                if let Some(channels) = &channel.channels {
                    return Ok(Some((*best_peer, channels.clone())));
                }
            }
        }

        Ok(None)
    }
}
