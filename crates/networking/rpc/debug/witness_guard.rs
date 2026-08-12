//! Which execution-witness method answers for a given header.
//!
//! The V1 methods (`debug_executionWitness`,
//! `debug_executionWitnessByBlockHash`) return an MPT witness: a flat list of
//! RLP-encoded Merkle-Patricia nodes a stateless verifier rebuilds a trie from
//! and hashes against `header.stateRoot`. Past the EIP-8297 activation
//! `header.stateRoot` is a *binary*-trie root, and no MPT witness can reproduce
//! it — so V1 must refuse those headers, and `debug_executionWitnessV2` serves
//! them instead.
//!
//! **Per header, never per chain.** A block from before the activation on a
//! chain that has since flipped genuinely carries an MPT root, has a real MPT
//! witness, and must keep serving it from V1 forever — after the flip, across
//! restarts, and on either side of a reorg. Asking the chain-level question
//! (`binary_tree_scheduled()`, "have we passed it") instead would make the
//! whole pre-flip history unwitnessable; that mistake is what wedged a devnet
//! on this branch. Both guards below therefore take a header and ask
//! [`ChainConfig::is_binary_tree_active`] about *that header's* timestamp,
//! which is the same question `StoreVmDatabase::open` and
//! `Store::header_addresses_binary_trie` ask, and the same one the
//! `eth_getProof` guard asks (`crates/networking/rpc/eth/account.rs`).
//!
//! # Why refusing is better than answering
//!
//! Refusing beats what V1 does today on both sides of the boundary:
//!
//! - at the **first** binary-committed block the parent is still pre-flip, so
//!   the unchecked `Store::state_trie` open succeeds and V1 returns a complete,
//!   well-formed MPT witness over the *parent's MPT state*. It looks like an
//!   answer. A verifier only discovers otherwise after re-executing the whole
//!   block and finding a root mismatch, which reads as "bad block" rather than
//!   "wrong method". Note `state_trie_checked` would not have caught this: the
//!   parent's MPT state really is held.
//! - at any **later** block the parent's `state_root` is itself a binary root,
//!   which names no MPT node, and the open fails with an internal
//!   `Root node with hash ... not found` — a missing-state error naming state
//!   the node does hold, in the other trie.
//!
//! [`ChainConfig::is_binary_tree_active`]: ethrex_common::types::ChainConfig::is_binary_tree_active

use ethrex_common::types::BlockHeader;
use ethrex_storage::Store;

use crate::RpcErr;

/// Whether `header`'s `state_root` addresses the EIP-8297 binary trie.
fn addresses_binary_trie(storage: &Store, header: &BlockHeader) -> bool {
    storage
        .get_chain_config()
        .is_binary_tree_active(header.timestamp)
}

/// Refuse a binary-committed header, pointing at V2.
///
/// The guard the V1 handlers run before anything else — before the cached
/// witness lookup in particular, since a cache populated on a
/// binary-committed block would hand back the same wrong MPT witness without
/// ever reaching the generator.
pub fn refuse_binary_committed(storage: &Store, header: &BlockHeader) -> Result<(), RpcErr> {
    if addresses_binary_trie(storage, header) {
        return Err(RpcErr::UnsupportedFork(format!(
            "debug_executionWitness is not available at block {}: the chain has reached the \
             binary-tree commitment (EIP-8297), whose state root cannot be witnessed in the \
             Merkle-Patricia format this method returns. Use debug_executionWitnessV2",
            header.number
        )));
    }
    Ok(())
}

/// Refuse a header that is *not* binary-committed, pointing back at V1.
///
/// The converse guard, and it matters as much as the first one: a pre-flip
/// block on a scheduled chain has no binary root in its header, so a V2
/// witness for it would be a witness against a root the header does not
/// commit to. Nothing downstream could check it.
pub fn require_binary_committed(storage: &Store, header: &BlockHeader) -> Result<(), RpcErr> {
    if !addresses_binary_trie(storage, header) {
        return Err(RpcErr::UnsupportedFork(format!(
            "debug_executionWitnessV2 is not available at block {}: the header commits a \
             Merkle-Patricia state root, either because the chain has no binary-tree \
             commitment (EIP-8297) scheduled or because this block predates it. Use \
             debug_executionWitness",
            header.number
        )));
    }
    Ok(())
}
