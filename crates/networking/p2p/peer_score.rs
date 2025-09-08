use crate::kademlia;
use crate::kademlia::PeerChannels;
use crate::rlpx::p2p::Capability;
use ethrex_common::H256;
use std::collections::BTreeMap;

const MAX_SCORE: i64 = 50;
const MIN_SCORE: i64 = -50;

#[derive(Debug, Clone, Default)]
pub struct PeerScores {
    scores: BTreeMap<H256, PeerScore>,
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
            scores: BTreeMap::default(),
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
        let peer_score = self.scores.entry(peer_id).or_default();
        peer_score.score = peer_score.score.saturating_add(1);
        if peer_score.score > MAX_SCORE {
            peer_score.score = MAX_SCORE;
        }
    }

    pub fn record_failure(&mut self, peer_id: H256) {
        let peer_score = self.scores.entry(peer_id).or_default();
        peer_score.score = peer_score.score.saturating_sub(1);
        if peer_score.score < MIN_SCORE {
            peer_score.score = MIN_SCORE;
        }
    }

    pub fn record_critical_failure(&mut self, peer_id: H256) {
        let peer_score = self.scores.entry(peer_id).or_default();
        peer_score.score = i64::MIN;
    }

    pub fn mark_in_use(&mut self, peer_id: H256) {
        let peer_score = self.scores.entry(peer_id).or_default();
        peer_score.active = true;
    }

    pub fn free_peer(&mut self, peer_id: H256) {
        let peer_score = self.scores.entry(peer_id).or_default();
        peer_score.active = false;
    }

    pub async fn update_peers(&mut self, kademlia_table: &kademlia::Kademlia) {
        let peer_table = kademlia_table.peers.lock().await;
        for (peer_id, _) in peer_table.iter() {
            self.scores.entry(*peer_id).or_default();
        }
        self.scores
            .retain(|peer_id, _| peer_table.contains_key(peer_id));
    }

    /// Returns the peer and it's peer channel with the highest score.
    pub async fn get_peer_channel_with_highest_score(
        &self,
        kademlia_table: &kademlia::Kademlia,
        capabilities: &[Capability],
    ) -> Option<(H256, PeerChannels)> {
        let peer_table = kademlia_table.peers.lock().await;
        self.scores
            .iter()
            .filter_map(|(id, peer_score)| {
                if peer_score.active {
                    return None;
                }
                let Some(peer_data) = &peer_table.get(id) else {
                    return None;
                };
                if !capabilities
                    .iter()
                    .all(|cap| peer_data.supported_capabilities.contains(cap))
                {
                    return None;
                }
                let peer_channel = peer_data.channels.clone()?;

                Some((*id, peer_score.score, peer_channel))
            })
            .max_by(|v1, v2| v1.1.cmp(&v2.1))
            .map(|(k, _, v)| (k, v))
    }

    pub fn len(&self) -> usize {
        self.scores.len()
    }

    pub fn is_empty(&self) -> bool {
        self.scores.is_empty()
    }
}
