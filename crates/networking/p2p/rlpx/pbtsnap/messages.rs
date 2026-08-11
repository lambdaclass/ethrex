//! `pbtsnap/1` message definitions.
//!
//! The wire shape of state sync for the EIP-8297 binary tree. See
//! `docs/eip-draft-pbtsnap.md` for the normative description; what follows is
//! only what the Rust types need to say.
//!
//! **The whole protocol is one request/response pair over a flat key range**,
//! because the binary tree is one flat keyspace. `snap/1` needs a second pair
//! (`GetStorageRanges`) only because its state trie's leaves name further
//! tries; there is no such decomposition here. Code is not carried at all —
//! it rides `snap/1 GetByteCodes`, which is content-addressed and self-verifying.

use bytes::Bytes;
use ethrex_common::H256;

/// Request the leaves of a binary tree from `origin` through `limit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetPbtLeafRange {
    /// Request ID — the responding peer must mirror this value.
    pub id: u64,
    /// The binary-tree root to serve against. In practice a post-activation
    /// header's `state_root`; a server that no longer holds it must refuse
    /// rather than answer against a root it does hold.
    pub root_hash: H256,
    /// Inclusive lower bound, empty for "from the first leaf".
    ///
    /// A full-length tree key in normal use, but **opaque to the server**: it
    /// is compared lexicographically and never parsed. That is what lets the
    /// empty and the past-the-end cases fall out of the ordinary path instead
    /// of needing their own. It is `Bytes` rather than a fixed-width hash
    /// because tree keys are 34 bytes in the account and code zones and 66 in
    /// the overflow-storage zone.
    pub origin: Bytes,
    /// Inclusive upper bound, empty for "no upper bound".
    ///
    /// Soft: the response carries the first leaf *past* this bound as a
    /// terminator, so a client can see where the interval ended rather than
    /// take the server's word that it was exhausted.
    pub limit: Bytes,
    /// Soft cap on the response's leaf bytes. Never suppresses the first leaf
    /// (the progress rule), and clamped by the server to its own maximum.
    pub response_bytes: u64,
}

/// One leaf of the binary tree: the key as stored, and its 32-byte value.
///
/// No preimage field, and that is a design point rather than an omission —
/// see the draft spec's "No preimages, and why". The client stores these keys
/// verbatim and derives any key it later wants from an address it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PbtLeaf {
    /// The tree key, 34 or 66 bytes depending on zone.
    pub key: Bytes,
    /// The leaf value.
    pub value: H256,
}

/// A consecutive run of leaves with the two boundary walks that pin it to the
/// requested root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PbtLeafRange {
    /// Request ID — mirrors the value from the request.
    pub id: u64,
    /// Leaves in ascending key order, starting at the first key at or after
    /// the request's `origin`.
    pub leaves: Vec<PbtLeaf>,
    /// Stored-node encodings of the walk of the request's `origin`, root
    /// first.
    pub left_proof: Vec<Bytes>,
    /// Stored-node encodings of the walk of the last returned leaf's key.
    /// Empty exactly when `leaves` is empty — there is no last leaf for it to
    /// be a walk of.
    pub right_proof: Vec<Bytes>,
}
