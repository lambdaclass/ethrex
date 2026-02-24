use ethereum_types::H256;
use ethrex_rlp::constants::RLP_NULL;
use ethrex_rlp::encode::{RLPEncode, encode_length};
use rayon::prelude::*;
use rustc_hash::FxHashMap;

use crate::EMPTY_TRIE_HASH;
use crate::error::TrieError;
use crate::nibbles::encode_compact_into;
use crate::node_hash::NodeHash;

use super::{LowerSubtrie, PathVec, SparseNode, SparseSubtrie, SubtrieBuffers};

/// Read the cached hash from a node that has already been hashed.
/// Used after `hash_subtrie` to avoid recomputing hashes.
#[inline]
fn cached_hash(node: &SparseNode) -> NodeHash {
    match node {
        SparseNode::Leaf { hash: Some(h), .. }
        | SparseNode::Extension { hash: Some(h), .. }
        | SparseNode::Branch { hash: Some(h), .. } => *h,
        SparseNode::Hash(h) => *h,
        SparseNode::Empty => NodeHash::from_encoded(&[RLP_NULL]),
        _ => NodeHash::default(),
    }
}

/// Compute the root hash of the entire SparseTrie.
///
/// 1. Hash all lower subtries in parallel via rayon.
/// 2. Propagate lower subtrie root hashes to upper subtrie.
/// 3. Hash the upper subtrie using the propagated hashes.
/// 4. Return the root hash.
pub fn compute_root(
    upper: &mut SparseSubtrie,
    lower: &mut [LowerSubtrie],
) -> Result<H256, TrieError> {
    // Hash all lower subtries in parallel via rayon.
    // Each lower subtrie is independent, so they can be hashed concurrently.
    lower.par_iter_mut().try_for_each(|lower_subtrie| {
        let subtrie = match lower_subtrie {
            LowerSubtrie::Revealed(s) => s,
            LowerSubtrie::Blind(Some(s)) => s,
            LowerSubtrie::Blind(None) => return Ok(()),
        };
        // Skip subtries with no dirty nodes — all hashes are still valid
        if subtrie.dirty_nodes.is_empty() {
            return Ok(());
        }
        hash_subtrie(subtrie)
    })?;

    finalize_root(upper, lower)
}

/// Sequential variant of compute_root.
/// Use when this trie is already being processed inside a parallel context
/// (e.g., storage tries computed via outer rayon parallelism) to avoid
/// nested rayon overhead.
pub fn compute_root_sequential(
    upper: &mut SparseSubtrie,
    lower: &mut [LowerSubtrie],
) -> Result<H256, TrieError> {
    // Fast path for flat mode: everything is in the upper subtrie.
    // Skip lower iteration, cross-boundary propagation, and finalize overhead.
    if lower.is_empty() {
        if !upper.dirty_nodes.is_empty() {
            hash_subtrie(upper)?;
        }
        let empty: &[u8] = &[];
        return match upper.nodes.get(empty) {
            Some(SparseNode::Empty) | None => Ok(*EMPTY_TRIE_HASH),
            Some(node) => Ok(cached_hash(node).finalize()),
        };
    }

    // Hash all lower subtries sequentially.
    for lower_subtrie in lower.iter_mut() {
        let subtrie = match lower_subtrie {
            LowerSubtrie::Revealed(s) => s,
            LowerSubtrie::Blind(Some(s)) => s,
            LowerSubtrie::Blind(None) => continue,
        };
        if subtrie.dirty_nodes.is_empty() {
            continue;
        }
        hash_subtrie(subtrie)?;
    }

    finalize_root(upper, lower)
}

/// Common finalization: propagate hashes and compute upper root.
fn finalize_root(upper: &mut SparseSubtrie, lower: &mut [LowerSubtrie]) -> Result<H256, TrieError> {
    // Propagate lower subtrie root hashes to the upper subtrie.
    // Read cached hashes directly instead of recomputing them.
    for (i, lower_subtrie) in lower.iter().enumerate() {
        let subtrie = match lower_subtrie {
            LowerSubtrie::Revealed(s) => s,
            LowerSubtrie::Blind(Some(s)) => s,
            LowerSubtrie::Blind(None) => continue,
        };
        let n0 = (i / 16) as u8;
        let n1 = (i % 16) as u8;
        let root_path = PathVec::from_slice(&[n0, n1]);
        if let Some(root_node) = subtrie.nodes.get(root_path.as_slice()) {
            let root_hash = cached_hash(root_node);
            upper.nodes.insert(root_path, SparseNode::Hash(root_hash));
        }
    }

    // Propagate hashes for upper subtrie nodes whose children are inside
    // lower subtries (past the depth-2 boundary). This handles extensions
    // with long keys that span the upper/lower boundary.
    propagate_cross_boundary_hashes(upper, lower);

    // Then hash the upper subtrie
    hash_subtrie(upper)?;

    // Get the root hash (already cached by hash_subtrie)
    let empty: &[u8] = &[];
    let root = upper.nodes.get(empty);
    match root {
        Some(SparseNode::Empty) | None => Ok(*EMPTY_TRIE_HASH),
        Some(node) => Ok(cached_hash(node).finalize()),
    }
}

/// For each extension in the upper subtrie whose child is inside a lower
/// subtrie (past the depth-2 boundary), read the child's cached hash and
/// insert a `SparseNode::Hash` entry in the upper subtrie.
fn propagate_cross_boundary_hashes(upper: &mut SparseSubtrie, lower: &[LowerSubtrie]) {
    // Collect (path, key) pairs first to avoid borrowing `upper.nodes` during mutation.
    // Only extensions in the upper subtrie can span the boundary (depth < 2 → depth >= 2).
    // Upper subtrie is small (at most ~17 nodes), so this is cheap.
    let extensions: Vec<(PathVec, PathVec)> = upper
        .nodes
        .iter()
        .filter_map(|(path, node)| {
            if let SparseNode::Extension { key, .. } = node {
                Some((path.clone(), key.clone()))
            } else {
                None
            }
        })
        .collect();

    for (path_data, key) in &extensions {
        let child_path: PathVec = path_data.iter().chain(key.iter()).copied().collect();
        // If the child is at depth >= 2 and not already in the upper subtrie,
        // we need to propagate its hash from the lower subtrie.
        if child_path.len() >= 2 && !upper.nodes.contains_key(child_path.as_slice()) {
            let idx = child_path[0] as usize * 16 + child_path[1] as usize;
            if idx >= lower.len() {
                continue;
            }
            let subtrie = match &lower[idx] {
                LowerSubtrie::Revealed(s) | LowerSubtrie::Blind(Some(s)) => Some(s),
                LowerSubtrie::Blind(None) => None,
            };
            if let Some(subtrie) = subtrie
                && let Some(child_node) = subtrie.nodes.get(child_path.as_slice())
            {
                let hash = cached_hash(child_node);
                upper.nodes.insert(child_path, SparseNode::Hash(hash));
            }
        }
    }
}

/// Hash all dirty nodes in a subtrie bottom-up using an iterative approach.
/// Only processes nodes whose hash was invalidated (set to None).
fn hash_subtrie(subtrie: &mut SparseSubtrie) -> Result<(), TrieError> {
    // Use the dirty_nodes set directly — O(dirty) instead of O(total_nodes)
    let mut dirty_paths: Vec<PathVec> = subtrie.dirty_nodes.iter().cloned().collect();
    // Sort deepest first for bottom-up processing.
    // Unstable sort avoids temporary allocation and is faster for this use case.
    dirty_paths.sort_unstable_by_key(|p| std::cmp::Reverse(p.len()));

    let mut buffers = std::mem::take(&mut subtrie.buffers);
    let mut rlp_cache = std::mem::take(&mut subtrie.rlp_cache);

    for path in &dirty_paths {
        if let Some(node) = subtrie.nodes.get(path.as_slice()) {
            // Skip nodes already hashed (stale dirty_nodes entries from prior passes)
            if matches!(
                node,
                SparseNode::Leaf { hash: Some(_), .. }
                    | SparseNode::Extension { hash: Some(_), .. }
                    | SparseNode::Branch { hash: Some(_), .. }
                    | SparseNode::Hash(_)
                    | SparseNode::Empty
            ) {
                continue;
            }

            let hash = node_hash(node, &subtrie.values, &subtrie.nodes, path, &mut buffers);
            // Cache the RLP encoding: swap buffers with the cache entry to reuse
            // the old allocation instead of cloning.
            let rlp = std::mem::take(&mut buffers.rlp_buf);
            if let Some(mut old) = rlp_cache.insert(path.clone(), rlp) {
                old.clear();
                buffers.rlp_buf = old;
            }
            // Store the computed hash
            if let Some(
                SparseNode::Leaf { hash: h, .. }
                | SparseNode::Extension { hash: h, .. }
                | SparseNode::Branch { hash: h, .. },
            ) = subtrie.nodes.get_mut(path.as_slice())
            {
                *h = Some(hash);
            }
        }
    }

    subtrie.buffers = buffers;
    subtrie.rlp_cache = rlp_cache;
    Ok(())
}

/// Compute the NodeHash for a single SparseNode.
#[inline]
fn node_hash(
    node: &SparseNode,
    values: &FxHashMap<PathVec, Vec<u8>>,
    nodes: &FxHashMap<PathVec, SparseNode>,
    path: &[u8],
    buffers: &mut SubtrieBuffers,
) -> NodeHash {
    buffers.rlp_buf.clear();

    match node {
        SparseNode::Empty => {
            // Empty node encodes as RLP_NULL
            NodeHash::from_encoded(&[RLP_NULL])
        }
        SparseNode::Hash(h) => *h,
        SparseNode::Leaf { key, hash } => {
            if let Some(h) = hash {
                return *h;
            }
            // Encode compact key into reusable buffer
            let is_leaf = key.last() == Some(&16);
            encode_compact_into(key, is_leaf, &mut buffers.compact_buf);

            // Build full_path for value lookup
            buffers.child_path_buf.clear();
            buffers.child_path_buf.extend_from_slice(path);
            buffers.child_path_buf.extend_from_slice(key);
            let value = values
                .get(buffers.child_path_buf.as_slice())
                .map(Vec::as_slice)
                .unwrap_or(&[]);

            // Manual RLP: [compact_key, value]
            buffers.rlp_buf.clear();
            let key_len = <[u8] as RLPEncode>::length(&buffers.compact_buf);
            let val_len = <[u8] as RLPEncode>::length(value);
            encode_length(key_len + val_len, &mut buffers.rlp_buf);
            <[u8] as RLPEncode>::encode(&buffers.compact_buf, &mut buffers.rlp_buf);
            <[u8] as RLPEncode>::encode(value, &mut buffers.rlp_buf);

            NodeHash::from_encoded(&buffers.rlp_buf)
        }
        SparseNode::Extension { key, hash } => {
            if let Some(h) = hash {
                return *h;
            }
            // Encode compact key into reusable buffer
            encode_compact_into(key, false, &mut buffers.compact_buf);

            // Build child_path for child lookup
            buffers.child_path_buf.clear();
            buffers.child_path_buf.extend_from_slice(path);
            buffers.child_path_buf.extend_from_slice(key);

            // Read cached child hash (bottom-up order guarantees children are hashed)
            let child_hash = match nodes.get(buffers.child_path_buf.as_slice()) {
                Some(child_node) => match child_node {
                    SparseNode::Leaf { hash: Some(h), .. }
                    | SparseNode::Extension { hash: Some(h), .. }
                    | SparseNode::Branch { hash: Some(h), .. } => *h,
                    SparseNode::Hash(h) => *h,
                    SparseNode::Empty => NodeHash::from_encoded(&[RLP_NULL]),
                    _ => {
                        let child_path = buffers.child_path_buf.clone();
                        node_hash(child_node, values, nodes, &child_path, buffers)
                    }
                },
                None => NodeHash::default(),
            };

            // Manual RLP: [compact_key, child_hash]
            buffers.rlp_buf.clear();
            let key_len = <[u8] as RLPEncode>::length(&buffers.compact_buf);
            let child_len = RLPEncode::length(&child_hash);
            encode_length(key_len + child_len, &mut buffers.rlp_buf);
            <[u8] as RLPEncode>::encode(&buffers.compact_buf, &mut buffers.rlp_buf);
            match &child_hash {
                NodeHash::Hashed(hash) => hash.0.encode(&mut buffers.rlp_buf),
                NodeHash::Inline((_, 0)) => buffers.rlp_buf.push(RLP_NULL),
                NodeHash::Inline((encoded, len)) => {
                    buffers.rlp_buf.extend_from_slice(&encoded[..*len as usize])
                }
            }

            NodeHash::from_encoded(&buffers.rlp_buf)
        }
        SparseNode::Branch { state_mask, hash } => {
            if let Some(h) = hash {
                return *h;
            }
            // Encode: RLP([child0, child1, ..., child15, value])
            // Read cached child hashes (bottom-up order guarantees children are hashed)
            let mut child_hashes: [NodeHash; 16] = [NodeHash::default(); 16];
            // Build path prefix once and swap the last nibble for each child
            buffers.child_path_buf.clear();
            buffers.child_path_buf.extend_from_slice(path);
            buffers.child_path_buf.push(0); // placeholder nibble
            let last_idx = buffers.child_path_buf.len() - 1;
            let mut mask = *state_mask;
            while mask != 0 {
                let i = mask.trailing_zeros() as u8;
                mask &= mask - 1; // clear lowest set bit
                buffers.child_path_buf[last_idx] = i;
                child_hashes[i as usize] = match nodes.get(buffers.child_path_buf.as_slice()) {
                    Some(child_node) => match child_node {
                        SparseNode::Leaf { hash: Some(h), .. }
                        | SparseNode::Extension { hash: Some(h), .. }
                        | SparseNode::Branch { hash: Some(h), .. } => *h,
                        SparseNode::Hash(h) => *h,
                        SparseNode::Empty => NodeHash::from_encoded(&[RLP_NULL]),
                        _ => {
                            // Fallback: clone path and recurse
                            let child_path = buffers.child_path_buf.clone();
                            node_hash(child_node, values, nodes, &child_path, buffers)
                        }
                    },
                    None => NodeHash::default(),
                };
            }

            // Now encode the branch node
            buffers.rlp_buf.clear();

            // Check for branch value: reuse child_path_buf by swapping last nibble to 16
            buffers.child_path_buf[last_idx] = 16;
            let branch_value = values
                .get(buffers.child_path_buf.as_slice())
                .map(Vec::as_slice)
                .unwrap_or(&[]);

            // Calculate payload length
            let value_len = <[u8] as RLPEncode>::length(branch_value);
            let payload_len = child_hashes
                .iter()
                .fold(value_len, |acc, child| acc + RLPEncode::length(child));

            encode_length(payload_len, &mut buffers.rlp_buf);
            for child in &child_hashes {
                match child {
                    NodeHash::Hashed(hash) => hash.0.encode(&mut buffers.rlp_buf),
                    NodeHash::Inline((_, 0)) => buffers.rlp_buf.push(RLP_NULL),
                    NodeHash::Inline((encoded, len)) => {
                        buffers.rlp_buf.extend_from_slice(&encoded[..*len as usize])
                    }
                }
            }
            <[u8] as RLPEncode>::encode(branch_value, &mut buffers.rlp_buf);

            NodeHash::from_encoded(&buffers.rlp_buf)
        }
    }
}

/// Encode a SparseNode to its RLP representation for DB persistence.
/// The `nodes` map is needed to resolve child hashes for branch nodes.
pub fn encode_node(
    node: &SparseNode,
    values: &FxHashMap<PathVec, Vec<u8>>,
    nodes: &FxHashMap<PathVec, SparseNode>,
    path_data: &[u8],
) -> Option<Vec<u8>> {
    match node {
        SparseNode::Empty => Some(vec![RLP_NULL]),
        SparseNode::Hash(_) => None,
        SparseNode::Leaf { key, .. } => {
            let is_leaf = key.last() == Some(&16);
            let mut compact_buf = Vec::new();
            encode_compact_into(key, is_leaf, &mut compact_buf);

            // Value key is the full path: position path + leaf key suffix
            let full_path: PathVec = path_data.iter().chain(key.iter()).copied().collect();
            let empty_value = Vec::new();
            let value = values.get(full_path.as_slice()).unwrap_or(&empty_value);

            let mut buf = Vec::new();
            let key_len = <[u8] as RLPEncode>::length(&compact_buf);
            let val_len = <[u8] as RLPEncode>::length(value.as_slice());
            encode_length(key_len + val_len, &mut buf);
            <[u8] as RLPEncode>::encode(&compact_buf, &mut buf);
            <[u8] as RLPEncode>::encode(value.as_slice(), &mut buf);
            Some(buf)
        }
        SparseNode::Extension { key, .. } => {
            let mut compact_buf = Vec::new();
            encode_compact_into(key, false, &mut compact_buf);

            // Look up the child node's hash (child is at path + extension key)
            let child_path: PathVec = path_data.iter().chain(key.iter()).copied().collect();
            let child_hash = match nodes.get(child_path.as_slice()) {
                Some(child_node) => match child_node {
                    SparseNode::Leaf { hash, .. }
                    | SparseNode::Extension { hash, .. }
                    | SparseNode::Branch { hash, .. } => hash.unwrap_or_default(),
                    SparseNode::Hash(h) => *h,
                    SparseNode::Empty => NodeHash::from_encoded(&[RLP_NULL]),
                },
                None => NodeHash::default(),
            };

            let mut buf = Vec::new();
            let key_len = <[u8] as RLPEncode>::length(&compact_buf);
            let child_len = RLPEncode::length(&child_hash);
            encode_length(key_len + child_len, &mut buf);
            <[u8] as RLPEncode>::encode(&compact_buf, &mut buf);
            match &child_hash {
                NodeHash::Hashed(hash) => hash.0.encode(&mut buf),
                NodeHash::Inline((_, 0)) => buf.push(RLP_NULL),
                NodeHash::Inline((encoded, len)) => {
                    buf.extend_from_slice(&encoded[..*len as usize])
                }
            }
            Some(buf)
        }
        SparseNode::Branch { state_mask, .. } => {
            // Look up cached hashes from child nodes, reusing a single buffer
            let mut child_hashes: [NodeHash; 16] = [NodeHash::default(); 16];
            let mut child_path_buf = PathVec::with_capacity(path_data.len() + 1);
            child_path_buf.extend_from_slice(path_data);
            child_path_buf.push(0); // placeholder nibble
            let last_idx = child_path_buf.len() - 1;
            let mut mask = *state_mask;
            while mask != 0 {
                let i = mask.trailing_zeros() as u8;
                mask &= mask - 1; // clear lowest set bit
                child_path_buf[last_idx] = i;
                if let Some(child_node) = nodes.get(child_path_buf.as_slice()) {
                    child_hashes[i as usize] = match child_node {
                        SparseNode::Leaf { hash, .. }
                        | SparseNode::Extension { hash, .. }
                        | SparseNode::Branch { hash, .. } => hash.unwrap_or_default(),
                        SparseNode::Hash(h) => *h,
                        SparseNode::Empty => NodeHash::from_encoded(&[RLP_NULL]),
                    };
                }
            }

            // Get branch value if any
            child_path_buf[last_idx] = 16;
            let empty_value = Vec::new();
            let branch_value = values
                .get(child_path_buf.as_slice())
                .unwrap_or(&empty_value);

            // Encode the branch as RLP
            let mut buf = Vec::new();
            let value_len = <[u8] as RLPEncode>::length(branch_value);
            let payload_len = child_hashes
                .iter()
                .fold(value_len, |acc, child| acc + RLPEncode::length(child));

            encode_length(payload_len, &mut buf);
            for child in &child_hashes {
                match child {
                    NodeHash::Hashed(hash) => hash.0.encode(&mut buf),
                    NodeHash::Inline((_, 0)) => buf.push(RLP_NULL),
                    NodeHash::Inline((encoded, len)) => {
                        buf.extend_from_slice(&encoded[..*len as usize])
                    }
                }
            }
            <[u8] as RLPEncode>::encode(branch_value, &mut buf);

            Some(buf)
        }
    }
}
