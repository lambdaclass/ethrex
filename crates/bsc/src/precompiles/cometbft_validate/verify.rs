//! Header/validator hashing, vote sign-bytes, ed25519, and commit verification
//! for the CometBFT `0x67` precompile. Mirrors greenfield-cometbft v1.3.2.

use ed25519_dalek::{Signature, VerifyingKey};
use prost::Message;
use sha2::{Digest, Sha256};

use super::merkle::hash_from_byte_slices;
use super::proto::{
    self, BLOCK_ID_FLAG_COMMIT, BlockId, CanonicalBlockId, CanonicalPartSetHeader, CanonicalVote,
    Commit, Header, PRECOMMIT_TYPE, PartSetHeader, PublicKey, SimpleValidator, marshal,
};

/// Chain-agnostic validator info, built from either the parsed consensus state
/// or a decoded light-block validator. Field order/content matches the
/// `SimpleValidator` Merkle leaf.
pub struct ValidatorInfo {
    pub pubkey: Vec<u8>, // ed25519, 32 bytes
    pub voting_power: i64,
    pub bls_key: Vec<u8>,         // 48 bytes
    pub relayer_address: Vec<u8>, // 20 bytes
}

impl ValidatorInfo {
    /// `Address = SHA256(pubkey)[:20]` (tmhash.SumTruncated).
    pub fn address(&self) -> [u8; 20] {
        let digest = Sha256::digest(&self.pubkey);
        let mut out = [0u8; 20];
        out.copy_from_slice(&digest[..20]);
        out
    }
}

/// `SimpleValidator` proto leaf for the validator-set Merkle hash.
fn validator_leaf(v: &ValidatorInfo) -> Vec<u8> {
    marshal(&SimpleValidator {
        pub_key: Some(PublicKey {
            ed25519: Some(v.pubkey.clone()),
        }),
        voting_power: v.voting_power,
        bls_key: v.bls_key.clone(),
        relayer_address: v.relayer_address.clone(),
    })
}

/// `ValidatorSet.Hash()` — Merkle root over the `SimpleValidator` leaves.
pub fn validator_set_hash(validators: &[ValidatorInfo]) -> [u8; 32] {
    let leaves: Vec<Vec<u8>> = validators.iter().map(validator_leaf).collect();
    hash_from_byte_slices(&leaves)
}

/// Sum of voting powers (`ValidatorSet.TotalVotingPower()`), saturating.
pub fn total_voting_power(validators: &[ValidatorInfo]) -> i64 {
    validators
        .iter()
        .fold(0i64, |acc, v| acc.saturating_add(v.voting_power))
}

// ── cdcEncode helpers (header hash leaves) ────────────────────────────────────

fn cdc_encode_string(s: &str) -> Vec<u8> {
    if s.is_empty() {
        Vec::new()
    } else {
        marshal(&proto::StringValue {
            value: s.to_string(),
        })
    }
}

fn cdc_encode_i64(v: i64) -> Vec<u8> {
    if v == 0 {
        Vec::new()
    } else {
        marshal(&proto::Int64Value { value: v })
    }
}

fn cdc_encode_bytes(b: &[u8]) -> Vec<u8> {
    if b.is_empty() {
        Vec::new()
    } else {
        marshal(&proto::BytesValue { value: b.to_vec() })
    }
}

/// `BlockID.ToProto().Marshal()` — Go always promotes `PartSetHeader` to a
/// present sub-message, so force it `Some` regardless of how it decoded.
fn blockid_to_proto_marshal(bid: &Option<BlockId>) -> Vec<u8> {
    let bid = bid.clone().unwrap_or_default();
    let normalized = BlockId {
        hash: bid.hash,
        part_set_header: Some(bid.part_set_header.unwrap_or_default()),
    };
    marshal(&normalized)
}

/// `Header.Hash()` — Merkle root over the 15 leaves (14 standard + greenfield
/// `RandaoMix`), in order.
pub fn header_hash(h: &Header) -> [u8; 32] {
    let leaves: Vec<Vec<u8>> = vec![
        marshal(&h.version.clone().unwrap_or_default()),
        cdc_encode_string(&h.chain_id),
        cdc_encode_i64(h.height),
        marshal(&h.time.clone().unwrap_or_default()),
        blockid_to_proto_marshal(&h.last_block_id),
        cdc_encode_bytes(&h.last_commit_hash),
        cdc_encode_bytes(&h.data_hash),
        cdc_encode_bytes(&h.validators_hash),
        cdc_encode_bytes(&h.next_validators_hash),
        cdc_encode_bytes(&h.consensus_hash),
        cdc_encode_bytes(&h.app_hash),
        cdc_encode_bytes(&h.last_results_hash),
        cdc_encode_bytes(&h.evidence_hash),
        cdc_encode_bytes(&h.proposer_address),
        cdc_encode_bytes(&h.randao_mix),
    ];
    hash_from_byte_slices(&leaves)
}

// ── Vote sign-bytes ───────────────────────────────────────────────────────────

/// A zero `BlockID` canonicalizes to an omitted `block_id`.
fn blockid_is_zero(bid: &BlockId) -> bool {
    let psh_zero = match &bid.part_set_header {
        None => true,
        Some(p) => p.total == 0 && p.hash.is_empty(),
    };
    bid.hash.is_empty() && psh_zero
}

fn canonical_block_id(bid: &BlockId) -> Option<CanonicalBlockId> {
    if blockid_is_zero(bid) {
        return None;
    }
    let psh = bid.part_set_header.clone().unwrap_or(PartSetHeader {
        total: 0,
        hash: Vec::new(),
    });
    Some(CanonicalBlockId {
        hash: bid.hash.clone(),
        part_set_header: Some(CanonicalPartSetHeader {
            total: psh.total,
            hash: psh.hash,
        }),
    })
}

/// `commit.VoteSignBytes(chainID, idx)` for a `BlockIDFlagCommit` signature:
/// length-delimited proto marshal of the `CanonicalVote`.
fn vote_sign_bytes(chain_id: &str, commit: &Commit, idx: usize) -> Vec<u8> {
    let sig = &commit.signatures[idx];
    // ForBlock => BlockID = commit.BlockID; otherwise zero (nil votes are never
    // reached here — the callers skip non-commit sigs before signing).
    let block_id = if sig.block_id_flag == BLOCK_ID_FLAG_COMMIT {
        commit.block_id.clone().unwrap_or_default()
    } else {
        BlockId::default()
    };
    let cv = CanonicalVote {
        r#type: PRECOMMIT_TYPE,
        height: commit.height,
        round: commit.round as i64,
        block_id: canonical_block_id(&block_id),
        timestamp: sig.timestamp.clone(),
        chain_id: chain_id.to_string(),
    };
    // protoio.MarshalDelimited = uvarint(len) || proto3 body.
    cv.encode_length_delimited_to_vec()
}

// ── Ed25519 ───────────────────────────────────────────────────────────────────

/// Go-stdlib-compatible ed25519 verify (cofactorless, canonical). `verify_strict`
/// additionally rejects small-order A/R, a divergence only reachable with
/// adversarial keys, never real validator sets.
fn ed25519_verify(pubkey: &[u8], msg: &[u8], sig: &[u8]) -> bool {
    let Ok(pk_bytes): Result<[u8; 32], _> = pubkey.try_into() else {
        return false;
    };
    let Ok(sig_bytes): Result<[u8; 64], _> = sig.try_into() else {
        return false;
    };
    let Ok(vk) = VerifyingKey::from_bytes(&pk_bytes) else {
        return false;
    };
    let signature = Signature::from_bytes(&sig_bytes);
    vk.verify_strict(msg, &signature).is_ok()
}

// ── Commit verification ───────────────────────────────────────────────────────

/// True if two `BlockID`s are equal (hash + part-set-header).
pub fn block_id_equals(a: &BlockId, b: &BlockId) -> bool {
    let psh_eq = match (&a.part_set_header, &b.part_set_header) {
        (None, None) => true,
        (Some(x), Some(y)) => x.total == y.total && x.hash == y.hash,
        (Some(x), None) | (None, Some(x)) => x.total == 0 && x.hash.is_empty(),
    };
    a.hash == b.hash && psh_eq
}

/// `VerifyCommitLight` — ≥2/3 of the given (new) validator set signed for the
/// block. Strict 1-to-1 signature/validator indexing.
pub fn verify_commit_light(
    chain_id: &str,
    expected_block_id: &BlockId,
    height: i64,
    commit: &Commit,
    validators: &[ValidatorInfo],
) -> bool {
    if validators.len() != commit.signatures.len() {
        return false;
    }
    if commit.height != height {
        return false;
    }
    let commit_bid = commit.block_id.clone().unwrap_or_default();
    if !block_id_equals(expected_block_id, &commit_bid) {
        return false;
    }
    // Go: `votingPowerNeeded := TotalVotingPower() * 2 / 3` (multiply, then
    // integer-divide). Accept on strictly greater.
    let voting_power_needed = total_voting_power(validators).saturating_mul(2) / 3;

    let mut tallied: i64 = 0;
    for (idx, sig) in commit.signatures.iter().enumerate() {
        if sig.block_id_flag != BLOCK_ID_FLAG_COMMIT {
            continue;
        }
        let val = &validators[idx];
        let msg = vote_sign_bytes(chain_id, commit, idx);
        if !ed25519_verify(&val.pubkey, &msg, &sig.signature) {
            return false;
        }
        tallied = tallied.saturating_add(val.voting_power);
        if tallied > voting_power_needed {
            return true;
        }
    }
    false
}

/// `VerifyCommitLightTrusting` — ≥1/3 of the trusted (old) validator set, looked
/// up by address, with a double-signature guard.
pub fn verify_commit_light_trusting(
    chain_id: &str,
    commit: &Commit,
    trusted: &[ValidatorInfo],
) -> bool {
    // votingPowerNeeded = total * 1 / 3.
    let voting_power_needed = total_voting_power(trusted) / 3;

    let mut seen: std::collections::HashSet<[u8; 20]> = std::collections::HashSet::new();
    let mut tallied: i64 = 0;
    for (idx, sig) in commit.signatures.iter().enumerate() {
        if sig.block_id_flag != BLOCK_ID_FLAG_COMMIT {
            continue;
        }
        // Lookup by validator address in the trusted set.
        let Some(val) = trusted
            .iter()
            .find(|v| v.address().as_slice() == sig.validator_address.as_slice())
        else {
            continue;
        };
        let addr = val.address();
        if !seen.insert(addr) {
            // Double vote from the same validator — reject.
            return false;
        }
        let msg = vote_sign_bytes(chain_id, commit, idx);
        if !ed25519_verify(&val.pubkey, &msg, &sig.signature) {
            return false;
        }
        tallied = tallied.saturating_add(val.voting_power);
        if tallied > voting_power_needed {
            return true;
        }
    }
    false
}
