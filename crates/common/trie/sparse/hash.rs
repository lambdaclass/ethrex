use ethereum_types::H256;
use ethrex_rlp::constants::RLP_NULL;
use ethrex_rlp::encode::{RLPEncode, encode_length};
use rayon::prelude::*;
use rustc_hash::FxHashMap;

use crate::EMPTY_TRIE_HASH;
use crate::error::TrieError;
use crate::node_hash::NodeHash;

use super::{LowerSubtrie, PrefixSet, SparseNode, SparseSubtrie, SubtrieBuffers};

/// Compute the root hash of the entire SparseTrie.
///
/// 1. Hash all lower subtries in parallel via rayon.
/// 2. Propagate lower subtrie root hashes to upper subtrie.
/// 3. Hash the upper subtrie using the propagated hashes.
/// 4. Return the root hash.
pub fn compute_root(
    upper: &mut SparseSubtrie,
    lower: &mut [LowerSubtrie],
    prefix_set: &mut PrefixSet,
) -> Result<H256, TrieError> {
    // Hash all lower subtries in parallel via rayon.
    // Each lower subtrie is independent, so they can be hashed concurrently.
    // PrefixSet is read-only after sort (triggered by first contains() call).
    // Force the sort now so parallel reads are safe.
    prefix_set.ensure_sorted();

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
        hash_subtrie(subtrie, prefix_set)
    })?;

    // Propagate lower subtrie root hashes to the upper subtrie.
    // Each lower subtrie's root node (at path [n0, n1]) needs its hash
    // visible to the upper subtrie's branch nodes.
    for (i, lower_subtrie) in lower.iter().enumerate() {
        let subtrie = match lower_subtrie {
            LowerSubtrie::Revealed(s) => s,
            LowerSubtrie::Blind(Some(s)) => s,
            LowerSubtrie::Blind(None) => continue,
        };
        let n0 = (i / 16) as u8;
        let n1 = (i % 16) as u8;
        let root_path = vec![n0, n1];
        if let Some(root_node) = subtrie.nodes.get(&root_path) {
            let mut tmp_buffers = SubtrieBuffers::default();
            let root_hash = node_hash(
                root_node,
                &subtrie.values,
                &subtrie.nodes,
                &root_path,
                &mut tmp_buffers,
            );
            upper.nodes.insert(root_path, SparseNode::Hash(root_hash));
        }
    }

    // Propagate hashes for upper subtrie nodes whose children are inside
    // lower subtries (past the depth-2 boundary). This handles extensions
    // with long keys that span the upper/lower boundary.
    propagate_cross_boundary_hashes(upper, lower);

    // Then hash the upper subtrie
    hash_subtrie(upper, prefix_set)?;

    // Get the root hash
    let root = upper.nodes.get(&Vec::<u8>::new());
    match root {
        Some(SparseNode::Empty) | None => Ok(*EMPTY_TRIE_HASH),
        Some(node) => {
            let hash = node_hash(node, &upper.values, &upper.nodes, &[], &mut upper.buffers);
            Ok(hash.finalize())
        }
    }
}

/// For each extension/branch in the upper subtrie whose child is inside a lower
/// subtrie (past the depth-2 boundary), compute the child's hash in the lower
/// subtrie and insert a `SparseNode::Hash` entry in the upper subtrie. This
/// allows `node_hash` and `encode_node` to find cross-boundary children.
fn propagate_cross_boundary_hashes(upper: &mut SparseSubtrie, lower: &[LowerSubtrie]) {
    let upper_paths: Vec<Vec<u8>> = upper.nodes.keys().cloned().collect();
    let mut to_propagate = Vec::new();

    for path_data in &upper_paths {
        let node = upper.nodes.get(path_data);
        if let Some(SparseNode::Extension { key, .. }) = node {
            let child_path: Vec<u8> = path_data
                .iter()
                .chain(key.as_ref().iter())
                .copied()
                .collect();
            // If the child is at depth >= 2 and not already in the upper subtrie,
            // we need to propagate its hash from the lower subtrie.
            if child_path.len() >= 2 && !upper.nodes.contains_key(&child_path) {
                let idx = child_path[0] as usize * 16 + child_path[1] as usize;
                let subtrie = match &lower[idx] {
                    LowerSubtrie::Revealed(s) | LowerSubtrie::Blind(Some(s)) => Some(s),
                    LowerSubtrie::Blind(None) => None,
                };
                if let Some(subtrie) = subtrie
                    && let Some(child_node) = subtrie.nodes.get(&child_path)
                {
                    let mut tmp_buffers = SubtrieBuffers::default();
                    let child_hash = node_hash(
                        child_node,
                        &subtrie.values,
                        &subtrie.nodes,
                        &child_path,
                        &mut tmp_buffers,
                    );
                    to_propagate.push((child_path, child_hash));
                }
            }
        }
    }

    for (path, hash) in to_propagate {
        upper.nodes.insert(path, SparseNode::Hash(hash));
    }
}

/// Hash all nodes in a subtrie bottom-up using an iterative stack-based approach.
fn hash_subtrie(subtrie: &mut SparseSubtrie, _prefix_set: &PrefixSet) -> Result<(), TrieError> {
    // Only collect paths where hash needs recomputation (hash == None),
    // avoiding the cost of sorting all revealed-but-clean nodes.
    let mut paths: Vec<Vec<u8>> = subtrie
        .nodes
        .iter()
        .filter(|(_, node)| {
            matches!(
                node,
                SparseNode::Leaf { hash: None, .. }
                    | SparseNode::Extension { hash: None, .. }
                    | SparseNode::Branch { hash: None, .. }
            )
        })
        .map(|(k, _)| k.clone())
        .collect();
    paths.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));

    let mut buffers = std::mem::take(&mut subtrie.buffers);

    for path in &paths {
        if let Some(node) = subtrie.nodes.get(path) {
            let hash = node_hash(node, &subtrie.values, &subtrie.nodes, path, &mut buffers);
            // Store the computed hash
            if let Some(node) = subtrie.nodes.get_mut(path) {
                match node {
                    SparseNode::Leaf { hash: h, .. } => *h = Some(hash),
                    SparseNode::Extension { hash: h, .. } => *h = Some(hash),
                    SparseNode::Branch { hash: h, .. } => *h = Some(hash),
                    _ => {}
                }
            }
        }
    }

    subtrie.buffers = buffers;
    Ok(())
}

/// Compute the NodeHash for a single SparseNode.
fn node_hash(
    node: &SparseNode,
    values: &FxHashMap<Vec<u8>, Vec<u8>>,
    nodes: &FxHashMap<Vec<u8>, SparseNode>,
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
            // Encode: RLP([compact_key, value])
            let compact = key.encode_compact();
            let full_path: Vec<u8> = path.iter().chain(key.as_ref().iter()).copied().collect();
            let empty_value = Vec::new();
            let value = values.get(&full_path).unwrap_or(&empty_value);
            ethrex_rlp::structs::Encoder::new(&mut buffers.rlp_buf)
                .encode_bytes(&compact)
                .encode_bytes(value)
                .finish();
            NodeHash::from_encoded(&buffers.rlp_buf)
        }
        SparseNode::Extension { key, hash } => {
            if let Some(h) = hash {
                return *h;
            }
            // Encode: RLP([compact_key, child_hash])
            let compact = key.encode_compact();
            let child_path: Vec<u8> = path.iter().chain(key.as_ref().iter()).copied().collect();

            let child_hash = match nodes.get(&child_path) {
                Some(child_node) => node_hash(child_node, values, nodes, &child_path, buffers),
                None => NodeHash::default(),
            };

            buffers.rlp_buf.clear();
            let mut encoder =
                ethrex_rlp::structs::Encoder::new(&mut buffers.rlp_buf).encode_bytes(&compact);
            encoder = child_hash.encode(encoder);
            encoder.finish();
            NodeHash::from_encoded(&buffers.rlp_buf)
        }
        SparseNode::Branch { state_mask, hash } => {
            if let Some(h) = hash {
                return *h;
            }
            // Encode: RLP([child0, child1, ..., child15, value])
            // First compute all child hashes, reusing the child_path_buf.
            let mut child_hashes: [NodeHash; 16] = [NodeHash::default(); 16];
            for i in 0..16u8 {
                if state_mask & (1 << i) != 0 {
                    buffers.child_path_buf.clear();
                    buffers.child_path_buf.extend_from_slice(path);
                    buffers.child_path_buf.push(i);
                    child_hashes[i as usize] = match nodes.get(&buffers.child_path_buf) {
                        Some(child_node) => {
                            // Clone the path out since node_hash needs mutable buffers
                            let child_path = buffers.child_path_buf.clone();
                            node_hash(child_node, values, nodes, &child_path, buffers)
                        }
                        None => NodeHash::default(),
                    };
                }
            }

            // Now encode the branch node
            buffers.rlp_buf.clear();

            // Check for branch value using the reusable buffer
            buffers.child_path_buf.clear();
            buffers.child_path_buf.extend_from_slice(path);
            buffers.child_path_buf.push(16);
            let empty_value = Vec::new();
            let branch_value = values.get(&buffers.child_path_buf).unwrap_or(&empty_value);

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
    values: &FxHashMap<Vec<u8>, Vec<u8>>,
    nodes: &FxHashMap<Vec<u8>, SparseNode>,
    path_data: &[u8],
) -> Option<Vec<u8>> {
    match node {
        SparseNode::Empty => Some(vec![RLP_NULL]),
        SparseNode::Hash(_) => None,
        SparseNode::Leaf { key, .. } => {
            let compact = key.encode_compact();
            // Value key is the full path: position path + leaf key suffix
            let full_path: Vec<u8> = path_data
                .iter()
                .chain(key.as_ref().iter())
                .copied()
                .collect();
            let empty_value = Vec::new();
            let value = values.get(&full_path).unwrap_or(&empty_value);
            let mut buf = Vec::new();
            ethrex_rlp::structs::Encoder::new(&mut buf)
                .encode_bytes(&compact)
                .encode_bytes(value)
                .finish();
            Some(buf)
        }
        SparseNode::Extension { key, .. } => {
            let compact = key.encode_compact();
            // Look up the child node's hash (child is at path + extension key)
            let child_path: Vec<u8> = path_data
                .iter()
                .chain(key.as_ref().iter())
                .copied()
                .collect();
            let child_hash = match nodes.get(&child_path) {
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
            let mut encoder = ethrex_rlp::structs::Encoder::new(&mut buf).encode_bytes(&compact);
            encoder = child_hash.encode(encoder);
            encoder.finish();
            Some(buf)
        }
        SparseNode::Branch { state_mask, .. } => {
            // Look up cached hashes from child nodes, reusing a single buffer
            let mut child_hashes: [NodeHash; 16] = [NodeHash::default(); 16];
            let mut child_path_buf = Vec::with_capacity(path_data.len() + 1);
            for i in 0..16u8 {
                if state_mask & (1 << i) != 0 {
                    child_path_buf.clear();
                    child_path_buf.extend_from_slice(path_data);
                    child_path_buf.push(i);
                    if let Some(child_node) = nodes.get(&child_path_buf) {
                        child_hashes[i as usize] = match child_node {
                            SparseNode::Leaf { hash, .. }
                            | SparseNode::Extension { hash, .. }
                            | SparseNode::Branch { hash, .. } => hash.unwrap_or_default(),
                            SparseNode::Hash(h) => *h,
                            SparseNode::Empty => NodeHash::from_encoded(&[RLP_NULL]),
                        };
                    }
                }
            }

            // Get branch value if any
            child_path_buf.clear();
            child_path_buf.extend_from_slice(path_data);
            child_path_buf.push(16);
            let empty_value = Vec::new();
            let branch_value = values.get(&child_path_buf).unwrap_or(&empty_value);

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
