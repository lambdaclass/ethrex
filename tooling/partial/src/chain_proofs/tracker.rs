//! Where EIP-8440 proofs meet the chain the light client follows.
//!
//! The light client yields beacon headers with the execution block applied before each. For
//! a header `N'` whose parent `N` was also seen, `N'.execution_block_hash != N.execution_block_hash`
//! means `N` was *full* and committed the payload `N'.execution_block_hash`. A proof whose
//! `head_block_root` is `root(N)` therefore proves the execution of that payload, and of
//! every payload back to its origin. The tracker keeps those facts and answers which
//! execution blocks are execution-proven since the checkpoint.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use ethrex_common::H256;

use crate::light_client::LcHead;

/// Beacon blocks remembered for mapping proofs to payloads.
pub const BEACON_HISTORY: usize = 4096;

/// What is known about one beacon block the light client vouched for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SeenBlock {
    slot: u64,
    parent_root: H256,
    /// The payload applied before this block.
    execution_before: H256,
    /// The payload this block committed, once its child shows it was applied.
    committed_payload: Option<H256>,
}

/// A verified proof.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AcceptedProof {
    /// Beacon root the proof is for.
    pub head_block_root: H256,
    /// Beacon root where its recursion started.
    pub origin_block_root: H256,
    /// Slot of the origin, if known.
    pub origin_slot: Option<u64>,
    /// Proof type.
    pub proof_type: u8,
    /// Execution block the proof's head payload is, once mapped.
    pub execution_block_hash: Option<H256>,
}

/// Why a proof was not accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProofRejected {
    /// The proof's head is not a beacon block the light client vouched for.
    #[error("proof head {0:#x} is not a known beacon block")]
    UnknownHead(H256),
    /// The proof's origin is after the checkpoint, so it says nothing about the gap.
    #[error("proof origin at slot {origin_slot} is after the checkpoint at slot {checkpoint_slot}")]
    OriginTooRecent {
        /// Slot of the origin.
        origin_slot: u64,
        /// Slot of the checkpoint.
        checkpoint_slot: u64,
    },
    /// The same proof type already proved this head.
    #[error("proof type {proof_type} already proved head {head:#x}")]
    Duplicate {
        /// The head.
        head: H256,
        /// The type.
        proof_type: u8,
    },
}

/// Tracks beacon blocks and verified proofs; answers which execution blocks are proven.
#[derive(Clone, Debug)]
pub struct ProofTracker {
    checkpoint: LcHead,
    seen: HashMap<H256, SeenBlock>,
    order: VecDeque<H256>,
    /// Payload hash to the beacon root that committed it.
    committed_by: HashMap<H256, H256>,
    /// Verified proofs by head root, then proof type.
    proofs: HashMap<H256, BTreeMap<u8, AcceptedProof>>,
    /// Execution blocks whose payload has a verified proof with an origin at or before
    /// the checkpoint.
    proven_payloads: HashSet<H256>,
}

impl ProofTracker {
    /// A tracker for a node that started at `checkpoint`. The checkpoint's execution block
    /// counts as proven: it is the trust root every proof is measured against.
    pub fn new(checkpoint: LcHead) -> Self {
        let mut tracker = Self {
            checkpoint,
            seen: HashMap::new(),
            order: VecDeque::new(),
            committed_by: HashMap::new(),
            proofs: HashMap::new(),
            proven_payloads: HashSet::from([checkpoint.execution_block_hash]),
        };
        tracker.observe(checkpoint);
        tracker
    }

    /// The checkpoint.
    pub const fn checkpoint(&self) -> LcHead {
        self.checkpoint
    }

    /// Records a beacon block the light client vouched for. Returns the payload the block's
    /// parent is now known to have committed, if the parent was seen and was full.
    pub fn observe(&mut self, head: LcHead) -> Option<(H256, H256)> {
        if let std::collections::hash_map::Entry::Vacant(slot) = self.seen.entry(head.beacon_root) {
            slot.insert(SeenBlock {
                slot: head.slot,
                parent_root: head.parent_root,
                execution_before: head.execution_block_hash,
                committed_payload: None,
            });
            self.order.push_back(head.beacon_root);
            while self.order.len() > BEACON_HISTORY {
                if let Some(old) = self.order.pop_front()
                    && let Some(block) = self.seen.remove(&old)
                {
                    if let Some(payload) = block.committed_payload {
                        self.committed_by.remove(&payload);
                    }
                    self.proofs.remove(&old);
                }
            }
        }
        let parent = *self.seen.get(&head.parent_root)?;
        if parent.execution_before == head.execution_block_hash {
            return None;
        }
        let payload = head.execution_block_hash;
        if let Some(parent_mut) = self.seen.get_mut(&head.parent_root) {
            parent_mut.committed_payload = Some(payload);
        }
        self.committed_by.insert(payload, head.parent_root);
        if let Some(by_type) = self.proofs.get_mut(&head.parent_root) {
            for proof in by_type.values_mut() {
                proof.execution_block_hash = Some(payload);
            }
            if by_type.values().any(|p| p.execution_block_hash.is_some()) {
                self.proven_payloads.insert(payload);
            }
        }
        Some((head.parent_root, payload))
    }

    /// The slot of a seen beacon block.
    pub fn slot_of(&self, root: H256) -> Option<u64> {
        self.seen.get(&root).map(|b| b.slot)
    }

    /// The beacon block that committed `payload`, if its child was seen.
    pub fn committer_of(&self, payload: H256) -> Option<H256> {
        self.committed_by.get(&payload).copied()
    }

    /// Whether a proof for `head_block_root` would be new.
    pub fn wants(&self, head_block_root: H256, proof_type: u8) -> bool {
        self.seen.contains_key(&head_block_root)
            && !self
                .proofs
                .get(&head_block_root)
                .is_some_and(|by_type| by_type.contains_key(&proof_type))
    }

    /// Records a proof the engine verified. `origin_slot` is the slot of the origin block,
    /// as the caller established it (a seen block, or a header fetched by root). The origin
    /// must be at or before the checkpoint for the proof to cover the gap since it.
    pub fn accept(
        &mut self,
        head_block_root: H256,
        origin_block_root: H256,
        origin_slot: Option<u64>,
        proof_type: u8,
    ) -> Result<AcceptedProof, ProofRejected> {
        let block = *self
            .seen
            .get(&head_block_root)
            .ok_or(ProofRejected::UnknownHead(head_block_root))?;
        let origin_slot = origin_slot.or_else(|| self.slot_of(origin_block_root));
        let covers_checkpoint = origin_block_root == self.checkpoint.beacon_root
            || origin_slot.is_some_and(|slot| slot <= self.checkpoint.slot);
        if !covers_checkpoint {
            return Err(ProofRejected::OriginTooRecent {
                origin_slot: origin_slot.unwrap_or(u64::MAX),
                checkpoint_slot: self.checkpoint.slot,
            });
        }
        let by_type = self.proofs.entry(head_block_root).or_default();
        if by_type.contains_key(&proof_type) {
            return Err(ProofRejected::Duplicate {
                head: head_block_root,
                proof_type,
            });
        }
        let accepted = AcceptedProof {
            head_block_root,
            origin_block_root,
            origin_slot,
            proof_type,
            execution_block_hash: block.committed_payload,
        };
        by_type.insert(proof_type, accepted);
        if let Some(payload) = block.committed_payload {
            self.proven_payloads.insert(payload);
        }
        Ok(accepted)
    }

    /// Whether the execution block `payload` has a verified chain proof reaching the
    /// checkpoint.
    pub fn is_proven(&self, payload: H256) -> bool {
        self.proven_payloads.contains(&payload)
    }

    /// Among `candidates` (execution block hashes, newest first), the first proven one.
    pub fn newest_proven<'a>(
        &self,
        candidates: impl IntoIterator<Item = &'a H256>,
    ) -> Option<H256> {
        candidates
            .into_iter()
            .copied()
            .find(|hash| self.is_proven(*hash))
    }

    /// Beacon roots seen that have no proof yet, newest first, up to `limit`. What to ask
    /// a beacon node for.
    pub fn unproven_heads(&self, limit: usize) -> Vec<H256> {
        self.order
            .iter()
            .rev()
            .filter(|root| !self.proofs.contains_key(*root))
            .take(limit)
            .copied()
            .collect()
    }

    /// Verified proofs, for status.
    pub fn proof_count(&self) -> usize {
        self.proofs.values().map(BTreeMap::len).sum()
    }

    /// Execution blocks proven by a chain proof, for status (the checkpoint not counted).
    pub fn proven_count(&self) -> usize {
        self.proven_payloads.len().saturating_sub(1)
    }

    /// A reorg: forgets blocks at or after `slot` and the proofs that hung on them.
    pub fn forget_from(&mut self, slot: u64) {
        let dropped: Vec<H256> = self
            .seen
            .iter()
            .filter(|(_, b)| b.slot >= slot)
            .map(|(root, _)| *root)
            .collect();
        for root in dropped {
            if let Some(block) = self.seen.remove(&root)
                && let Some(payload) = block.committed_payload
            {
                self.committed_by.remove(&payload);
                if payload != self.checkpoint.execution_block_hash {
                    self.proven_payloads.remove(&payload);
                }
            }
            self.proofs.remove(&root);
            self.order.retain(|r| *r != root);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn head(slot: u64, parent: u8, me: u8, exec: u8) -> LcHead {
        LcHead {
            slot,
            beacon_root: H256::repeat_byte(me),
            parent_root: H256::repeat_byte(parent),
            execution_block_hash: H256::repeat_byte(exec),
        }
    }

    #[test]
    fn a_full_block_is_recognized_by_its_child_and_its_proof_proves_the_payload() {
        let checkpoint = head(100, 0, 1, 0xa0);
        let mut tracker = ProofTracker::new(checkpoint);
        // Block 2 (slot 101) is full: block 3 sees a new payload. Block 3 is empty: block 4
        // sees the same payload as block 3.
        assert_eq!(tracker.observe(head(101, 1, 2, 0xa0)), None);
        assert_eq!(
            tracker.observe(head(102, 2, 3, 0xa1)),
            Some((H256::repeat_byte(2), H256::repeat_byte(0xa1)))
        );
        assert_eq!(tracker.observe(head(103, 3, 4, 0xa1)), None);
        assert_eq!(
            tracker.committer_of(H256::repeat_byte(0xa1)),
            Some(H256::repeat_byte(2))
        );
        assert!(!tracker.is_proven(H256::repeat_byte(0xa1)));
        assert!(
            tracker.is_proven(checkpoint.execution_block_hash),
            "the checkpoint is the trust root"
        );
        assert_eq!(tracker.proven_count(), 0);

        // A proof for block 2 whose origin is the checkpoint proves payload a1.
        let accepted = tracker
            .accept(H256::repeat_byte(2), checkpoint.beacon_root, None, 3)
            .unwrap();
        assert_eq!(accepted.execution_block_hash, Some(H256::repeat_byte(0xa1)));
        assert!(tracker.is_proven(H256::repeat_byte(0xa1)));
        assert!(!tracker.wants(H256::repeat_byte(2), 3));
        assert!(tracker.wants(H256::repeat_byte(2), 5));
        assert_eq!(
            tracker.accept(H256::repeat_byte(2), checkpoint.beacon_root, None, 3),
            Err(ProofRejected::Duplicate {
                head: H256::repeat_byte(2),
                proof_type: 3
            })
        );
        assert_eq!(
            tracker.unproven_heads(10),
            vec![
                H256::repeat_byte(4),
                H256::repeat_byte(3),
                H256::repeat_byte(1)
            ]
        );
    }

    #[test]
    fn origins_after_the_checkpoint_and_unknown_heads_are_rejected() {
        let checkpoint = head(100, 0, 1, 0xa0);
        let mut tracker = ProofTracker::new(checkpoint);
        tracker.observe(head(101, 1, 2, 0xa0));
        tracker.observe(head(102, 2, 3, 0xa1));
        assert_eq!(
            tracker.accept(H256::repeat_byte(9), checkpoint.beacon_root, None, 3),
            Err(ProofRejected::UnknownHead(H256::repeat_byte(9)))
        );
        assert!(matches!(
            tracker.accept(H256::repeat_byte(2), H256::repeat_byte(2), None, 3),
            Err(ProofRejected::OriginTooRecent {
                origin_slot: 101,
                checkpoint_slot: 100
            })
        ));
        // An origin older than the checkpoint, known only by a fetched header's slot.
        let accepted = tracker
            .accept(H256::repeat_byte(2), H256::repeat_byte(0x77), Some(50), 3)
            .unwrap();
        assert_eq!(accepted.origin_slot, Some(50));
        assert!(tracker.is_proven(H256::repeat_byte(0xa1)));
    }

    #[test]
    fn a_proof_seen_before_the_child_maps_once_the_child_arrives() {
        let checkpoint = head(100, 0, 1, 0xa0);
        let mut tracker = ProofTracker::new(checkpoint);
        tracker.observe(head(101, 1, 2, 0xa0));
        let accepted = tracker
            .accept(H256::repeat_byte(2), checkpoint.beacon_root, None, 3)
            .unwrap();
        assert_eq!(accepted.execution_block_hash, None);
        assert!(!tracker.is_proven(H256::repeat_byte(0xa1)));
        tracker.observe(head(102, 2, 3, 0xa1));
        assert!(tracker.is_proven(H256::repeat_byte(0xa1)));
        let newest =
            tracker.newest_proven([H256::repeat_byte(0xa2), H256::repeat_byte(0xa1)].iter());
        assert_eq!(newest, Some(H256::repeat_byte(0xa1)));

        tracker.forget_from(101);
        assert!(!tracker.is_proven(H256::repeat_byte(0xa1)));
        assert_eq!(tracker.proof_count(), 0);
    }
}
