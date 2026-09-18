//! EIP-8070 / PeerDAS provider-vs-sampler role determination.
//!
//! A node acts as a *provider* for a blob transaction when a deterministic
//! pseudo-random function of (epoch_seed, tx_hash) falls below a threshold
//! corresponding to PROVIDER_PROBABILITY_PCT percent. This keeps the provider
//! set approximately sparse while ensuring enough redundancy.

use ethrex_common::{H256, U256};
use ethrex_crypto::keccak::keccak_hash;

use crate::mempool::PROVIDER_PROBABILITY_PCT;

/// Decide whether this node is a *provider* for `tx_hash` in the current
/// epoch.
///
/// `local_node_id` is the keccak256 node identity of the local peer (derived
/// from its public key). Including it in the preimage gives each node an
/// independent, per-node pseudo-random decision so the provider set follows
/// the intended Binomial(D, p) distribution — without it every node would
/// make the identical decision for a given (epoch_seed, tx_hash), collapsing
/// all nodes to either all-provider or all-sampler for that tx.
///
/// `epoch_seed` = `head_block_number / 32`.
///
/// When `eager` is `true` (local block builders; EIP-8070, "Execution clients ::
/// Local block builders"), always return `true`.
pub fn is_provider_role(local_node_id: H256, tx_hash: H256, epoch_seed: u64, eager: bool) -> bool {
    if eager {
        return true;
    }
    // hash = keccak256(local_node_id ++ epoch_seed_be ++ tx_hash)
    let mut preimage = [0u8; 32 + 8 + 32];
    preimage[..32].copy_from_slice(local_node_id.as_bytes());
    preimage[32..40].copy_from_slice(&epoch_seed.to_be_bytes());
    preimage[40..].copy_from_slice(tx_hash.as_bytes());
    let digest = keccak_hash(preimage);

    let value = U256::from_big_endian(&digest);
    // threshold = U256::MAX / 100 * PROVIDER_PROBABILITY_PCT
    let threshold = U256::MAX / U256::from(100) * U256::from(PROVIDER_PROBABILITY_PCT);
    value < threshold
}

/// Pick one extra column index not already in `custody_mask`, derived
/// deterministically from the local node identity and `tx_hash`.
///
/// `local_node_id` is in the preimage for the same reason as in
/// [`is_provider_role`]: derived from the tx hash alone, every sampler in the
/// network picks the *same* extra column for a given tx, so C_extra adds no
/// sampling diversity — one column gets oversampled and the other 126 spares
/// get none.
///
/// Returns `None` only when `custody_mask` already covers all 128 columns
/// (which is the provider case; samplers always have spare columns).
pub fn pick_random_extra_column(
    custody_mask: u128,
    local_node_id: H256,
    tx_hash: H256,
) -> Option<u32> {
    let available = !custody_mask; // bits NOT in custody
    let count = available.count_ones();
    if count == 0 {
        return None;
    }
    // hash = keccak256(local_node_id ++ tx_hash), 4 bytes for a uniform index.
    let mut preimage = [0u8; 64];
    preimage[..32].copy_from_slice(local_node_id.as_bytes());
    preimage[32..].copy_from_slice(tx_hash.as_bytes());
    let digest = keccak_hash(preimage);
    let idx = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) % count;
    // Walk the set bits of `available` to find the idx-th one.
    let mut remaining = idx;
    for col in 0..128u32 {
        if (available >> col) & 1 == 1 {
            if remaining == 0 {
                return Some(col);
            }
            remaining -= 1;
        }
    }
    None
}
