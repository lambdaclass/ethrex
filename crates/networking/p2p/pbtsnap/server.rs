//! Serving `pbtsnap/1` leaf ranges from the persistent binary trie.
//!
//! Much smaller than the snap server it sits beside, because the binary tree
//! needs no serving index: opening a trie at a root loads nothing (the root is
//! a bare stored reference until a traversal touches it), so a request is a
//! root resolution plus one cursor walk plus two boundary walks. There is
//! nothing to build, cache or thrash.

use bytes::Bytes;
use ethereum_types::H256;
use ethrex_storage::Store;

use crate::rlpx::pbtsnap::{GetPbtLeafRange, PbtLeaf, PbtLeafRange};
use crate::snap::constants::MAX_RESPONSE_BYTES;

use super::error::PbtSnapError;

/// The largest number of wire bytes one leaf can cost: the longest tree key
/// plus its 32-byte value.
///
/// Read off the embedding rather than written down, and deliberately the
/// *longest* of the zones' key lengths. The budget has to be turned into a
/// leaf count before the walk — a range cannot be truncated afterwards without
/// invalidating its own right-hand proof, which is a walk of the last leaf —
/// so the per-leaf charge must be an upper bound or the response could exceed
/// the budget it was given.
const MAX_LEAF_WIRE_BYTES: u64 =
    ethrex_binary_trie::embedding::STORAGE_KEY_LENGTH as u64 + H256::len_bytes() as u64;

/// How many leaves a `response_bytes` request may be answered with.
///
/// Two clamps and a floor:
///
/// - the peer-supplied budget is capped at [`MAX_RESPONSE_BYTES`], so a peer
///   cannot ask one request to buffer an unbounded slice of the state;
/// - the budget is divided by the worst-case per-leaf cost, so the response
///   never exceeds it;
/// - and the result is floored at **one**, which is the progress rule made
///   structural. A server must return the first leaf at or after `origin` if
///   the tree holds one anywhere, whatever the budget, because that is what
///   makes an empty response provable. Expressing it as a floor here means no
///   arithmetic below can violate it.
pub(crate) fn leaf_budget(response_bytes: u64) -> usize {
    let clamped = response_bytes.min(MAX_RESPONSE_BYTES);
    (clamped / MAX_LEAF_WIRE_BYTES).max(1) as usize
}

/// Answer a `GetPbtLeafRange` from this node's binary trie.
///
/// Runs on a blocking thread like the snap handlers: the walk is disk-bound
/// trie traversal, not async work.
///
/// An error is not a protocol violation and must not be treated as one — see
/// [`PbtSnapError::UnservableRoot`]. The connection layer answers one with an
/// empty [`PbtLeafRange`], mirroring the `TrieNodes` precedent: an empty
/// response fails the client's own verification (the completeness rule rejects
/// a forged emptiness) and triggers a retry or a re-pivot, never a silent gap.
pub async fn process_pbt_leaf_range_request(
    request: GetPbtLeafRange,
    store: Store,
) -> Result<PbtLeafRange, PbtSnapError> {
    tokio::task::spawn_blocking(move || {
        let root = request.root_hash;
        // Resolve before opening. A root is only servable if a canonical,
        // post-activation header carries it *and* the layered trie really
        // resolves to it; the walk that answers that is bounded by the layer
        // window, because past it the single-version trie holds a different
        // tree entirely.
        if store.canonical_block_for_binary_root(root)?.is_none() {
            return Err(PbtSnapError::UnservableRoot(root));
        }

        let slice = store.binary_leaf_range_proof(
            root,
            &request.origin,
            &request.limit,
            leaf_budget(request.response_bytes),
        )?;

        Ok(PbtLeafRange {
            id: request.id,
            leaves: slice
                .leaves
                .into_iter()
                .map(|(key, value)| PbtLeaf {
                    key: Bytes::from(key),
                    value: H256(value),
                })
                .collect(),
            left_proof: slice.left_proof.into_iter().map(Bytes::from).collect(),
            right_proof: slice.right_proof.into_iter().map(Bytes::from).collect(),
        })
    })
    .await
    .map_err(|e| PbtSnapError::TaskPanic(e.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_binary_trie::trie::verify_range;
    use ethrex_common::types::{BlockHeader, ChainConfig, GenesisAccount};
    use ethrex_common::{Address, U256};
    use ethrex_storage::{EngineType, Store};
    use std::collections::BTreeMap;

    const ACTIVE_TIMESTAMP: u64 = 1_000;

    fn genesis_account(nonce: u64, balance: u64, storage: &[(u64, u64)]) -> GenesisAccount {
        GenesisAccount {
            code: Bytes::new(),
            storage: storage
                .iter()
                .map(|(slot, value)| (U256::from(*slot), U256::from(*value)))
                .collect(),
            balance: U256::from(balance),
            nonce,
        }
    }

    /// A store whose canonical head is a post-activation block holding binary
    /// state — the minimum a server needs to be able to answer at all.
    async fn served_store() -> (Store, BlockHeader) {
        let mut store = Store::new("", EngineType::InMemory).expect("in-memory store");
        let config = ChainConfig {
            binary_tree_time: Some(0),
            ..Default::default()
        };
        store.set_chain_config(&config).await.expect("chain config");

        let mut alloc = BTreeMap::new();
        alloc.insert(
            Address::repeat_byte(0x11),
            genesis_account(1, 1_000, &[(1, 2), (900, 3)]),
        );
        alloc.insert(
            Address::repeat_byte(0x22),
            genesis_account(2, 500, &[(5, 7)]),
        );
        alloc.insert(Address::repeat_byte(0x33), genesis_account(0, 1, &[]));
        let root = store
            .setup_genesis_binary_trie(alloc)
            .await
            .expect("genesis binary trie");

        let header = BlockHeader {
            number: 1,
            timestamp: ACTIVE_TIMESTAMP,
            state_root: root,
            ..Default::default()
        };
        let hash = header.hash();
        store
            .add_block_header(hash, header.clone())
            .await
            .expect("header");
        store
            .forkchoice_update(vec![(1, hash)], 1, hash, None, None)
            .await
            .expect("fcu");
        store.set_binary_trie_root(hash, root).expect("record root");
        (store, header)
    }

    fn whole_keyspace(root: H256, response_bytes: u64) -> GetPbtLeafRange {
        GetPbtLeafRange {
            id: 1,
            root_hash: root,
            origin: Bytes::new(),
            limit: Bytes::new(),
            response_bytes,
        }
    }

    fn to_leaves(response: &PbtLeafRange) -> Vec<(Vec<u8>, [u8; 32])> {
        response
            .leaves
            .iter()
            .map(|leaf| (leaf.key.to_vec(), leaf.value.0))
            .collect()
    }

    fn to_proof(proof: &[Bytes]) -> Vec<Vec<u8>> {
        proof.iter().map(|node| node.to_vec()).collect()
    }

    #[tokio::test]
    async fn a_served_range_verifies_against_the_pivot_root() {
        let (store, header) = served_store().await;
        let request = whole_keyspace(header.state_root, MAX_RESPONSE_BYTES);
        let response = process_pbt_leaf_range_request(request.clone(), store)
            .await
            .expect("serve");

        assert_eq!(response.id, request.id, "the request id must be mirrored");
        assert!(!response.leaves.is_empty(), "the fixture has state");
        verify_range(
            header.state_root,
            &request.origin,
            &to_leaves(&response),
            &to_proof(&response.left_proof),
            &to_proof(&response.right_proof),
        )
        .expect("an honestly served range must verify against the header's root");
    }

    /// A root this node cannot answer for is refused. The client re-pivots; it
    /// must not conclude the state does not exist.
    #[tokio::test]
    async fn a_root_this_node_does_not_hold_is_refused() {
        let (store, _header) = served_store().await;
        let unknown = H256::repeat_byte(9);
        let error =
            process_pbt_leaf_range_request(whole_keyspace(unknown, MAX_RESPONSE_BYTES), store)
                .await
                .expect_err("an unheld root must not be served");
        assert!(
            matches!(error, PbtSnapError::UnservableRoot(root) if root == unknown),
            "got {error}",
        );
    }

    /// The progress rule survives a budget that cannot pay for a single leaf.
    /// Without it, "the budget ran out" and "there is nothing left" would look
    /// identical to a client and an empty response would stop being provable.
    #[tokio::test]
    async fn a_budget_below_one_leaf_still_returns_and_proves_the_first_leaf() {
        let (store, header) = served_store().await;
        let whole = process_pbt_leaf_range_request(
            whole_keyspace(header.state_root, MAX_RESPONSE_BYTES),
            store.clone(),
        )
        .await
        .expect("serve");
        let starved = process_pbt_leaf_range_request(whole_keyspace(header.state_root, 0), store)
            .await
            .expect("serve");

        assert_eq!(starved.leaves.len(), 1, "the floor is one leaf");
        assert_eq!(starved.leaves[0], whole.leaves[0]);
        assert!(starved.leaves.len() < whole.leaves.len());

        let verified = verify_range(
            header.state_root,
            &[],
            &to_leaves(&starved),
            &to_proof(&starved.left_proof),
            &to_proof(&starved.right_proof),
        )
        .expect("a starved response must still verify");
        assert!(
            verified.has_more,
            "the client must be able to see there is more to ask for"
        );
    }

    /// The budget bounds the response, and it does so *before* the walk: a
    /// range cannot be truncated after the fact without invalidating its right
    /// walk, which is a walk of the last leaf it returns.
    #[tokio::test]
    async fn the_response_never_exceeds_the_budget_it_was_given() {
        let (store, header) = served_store().await;
        for budget in [0u64, 1, MAX_LEAF_WIRE_BYTES, 3 * MAX_LEAF_WIRE_BYTES] {
            let response = process_pbt_leaf_range_request(
                whole_keyspace(header.state_root, budget),
                store.clone(),
            )
            .await
            .expect("serve");
            let leaf_bytes: u64 = response
                .leaves
                .iter()
                .map(|leaf| leaf.key.len() as u64 + 32)
                .sum();
            // The progress rule outranks the budget, so one leaf may exceed it;
            // nothing beyond that may.
            let allowance = budget.max(MAX_LEAF_WIRE_BYTES);
            assert!(
                leaf_bytes <= allowance,
                "budget {budget} produced {leaf_bytes} leaf bytes",
            );
            assert!(!response.leaves.is_empty(), "the progress rule still holds");
        }
    }

    /// A peer-supplied budget is clamped to the server's own maximum, so one
    /// request cannot make the server buffer an unbounded slice of state.
    #[test]
    fn an_unbounded_budget_is_clamped_to_the_server_maximum() {
        assert_eq!(
            leaf_budget(u64::MAX),
            leaf_budget(MAX_RESPONSE_BYTES),
            "a peer must not be able to raise the cap by asking",
        );
        assert_eq!(
            leaf_budget(MAX_RESPONSE_BYTES),
            (MAX_RESPONSE_BYTES / MAX_LEAF_WIRE_BYTES) as usize,
        );
    }

    /// The budget invariant stated against a worst case spelled out here, not
    /// against [`MAX_LEAF_WIRE_BYTES`].
    ///
    /// Asserting it through the constant is asserting that the constant agrees
    /// with itself: setting the per-leaf charge to the shorter account-zone key
    /// shrinks both sides of such a comparison at once and nothing fails, while
    /// a real range of overflow-storage leaves (66-byte keys) would overrun the
    /// budget the peer set. A mutation check caught exactly that.
    #[test]
    fn the_leaf_count_cannot_overrun_the_budget_even_for_all_storage_keys() {
        // The longest tree key the embedding defines, plus the leaf value.
        const WORST_CASE_LEAF: u64 = 66 + 32;
        for budget in [0u64, 1, 97, 98, 200, 1_000, MAX_RESPONSE_BYTES, u64::MAX] {
            let bytes = leaf_budget(budget) as u64 * WORST_CASE_LEAF;
            // The progress rule outranks the budget for the first leaf, and the
            // server's own cap outranks a peer asking for more.
            let allowance = budget.clamp(WORST_CASE_LEAF, MAX_RESPONSE_BYTES);
            assert!(
                bytes <= allowance,
                "budget {budget} admits {bytes} worst-case bytes, over {allowance}",
            );
        }
    }

    #[test]
    fn the_leaf_budget_never_reaches_zero() {
        for budget in [0u64, 1, MAX_LEAF_WIRE_BYTES - 1] {
            assert_eq!(leaf_budget(budget), 1, "budget {budget}");
        }
    }

    /// An origin past every leaf is the provable-emptiness case, and it must
    /// survive the trip through the wire types: no leaves, no right walk, and
    /// a left walk that still verifies.
    #[tokio::test]
    async fn an_origin_past_every_leaf_serves_a_verifiable_empty_range() {
        let (store, header) = served_store().await;
        let origin = Bytes::from(vec![0xffu8; 66]);
        let response = process_pbt_leaf_range_request(
            GetPbtLeafRange {
                origin: origin.clone(),
                ..whole_keyspace(header.state_root, MAX_RESPONSE_BYTES)
            },
            store,
        )
        .await
        .expect("serve");

        assert!(response.leaves.is_empty());
        assert!(response.right_proof.is_empty(), "no last leaf to walk");
        verify_range(
            header.state_root,
            &origin,
            &[],
            &to_proof(&response.left_proof),
            &to_proof(&response.right_proof),
        )
        .expect("provable emptiness must verify");
    }
}
