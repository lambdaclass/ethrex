use rayon::prelude::*;

use ethrex_rlp::decode::RLPDecode;

use crate::error::TrieError;
use crate::nibbles::Nibbles;
use crate::node::Node;
use crate::node_hash::NodeHash;

use super::{LowerSubtrie, PathVec, SparseNode, SparseSubtrie, SparseTrieProvider};

/// Determine which subtrie a path belongs to.
/// Returns None for upper subtrie, Some(idx) for lower subtrie.
fn route_path(path: &[u8]) -> Option<usize> {
    if path.len() < 2 {
        None
    } else {
        Some(path[0] as usize * 16 + path[1] as usize)
    }
}

/// Get or create the subtrie for a given lower index.
fn get_or_create_lower(lower: &mut [LowerSubtrie], idx: usize) -> &mut SparseSubtrie {
    // First check if already revealed
    let needs_create = !matches!(&lower[idx], LowerSubtrie::Revealed(_));
    if needs_create {
        let subtrie = match &mut lower[idx] {
            LowerSubtrie::Blind(opt) => opt.take().unwrap_or_else(|| {
                let n0 = (idx / 16) as u8;
                let n1 = (idx % 16) as u8;
                SparseSubtrie::new(Nibbles::from_hex(vec![n0, n1]))
            }),
            LowerSubtrie::Revealed(_) => unreachable!(),
        };
        lower[idx] = LowerSubtrie::Revealed(subtrie);
    }
    match &mut lower[idx] {
        LowerSubtrie::Revealed(s) => s,
        _ => unreachable!(),
    }
}

/// Decode an RLP-encoded trie node into SparseNode entries and insert them
/// into the appropriate subtrie.
pub fn reveal_node_into(
    upper: &mut SparseSubtrie,
    lower: &mut [LowerSubtrie],
    path: Nibbles,
    rlp: &[u8],
) -> Result<(), TrieError> {
    let node = Node::decode(rlp).map_err(TrieError::RLPDecode)?;
    let path_data = PathVec::from_slice(path.as_ref());
    // Compute the hash of this node from the original RLP, so revealed-but-unmodified
    // nodes retain their hash. Only nodes explicitly modified during update_leaf/remove_leaf
    // will have hash set to None.
    let revealed_hash = Some(NodeHash::from_encoded(rlp));

    match node {
        Node::Leaf(leaf) => {
            let full_path = path.concat(&leaf.partial);
            let key = PathVec::from_slice(leaf.partial.as_ref());
            // Route both node and value to the same subtrie (based on node path)
            let target = match route_path(&path_data) {
                Some(idx) => get_or_create_lower(lower, idx),
                None => upper,
            };
            target.nodes.insert(
                path_data,
                SparseNode::Leaf {
                    key,
                    hash: revealed_hash,
                },
            );
            target
                .values
                .insert(PathVec::from_slice(full_path.as_ref()), leaf.value);
        }
        Node::Extension(ext) => {
            let child_path = path.concat(&ext.prefix);
            let child_hash = ext.child.compute_hash();
            let key = PathVec::from_slice(ext.prefix.as_ref());

            let target = match route_path(&path_data) {
                Some(idx) => get_or_create_lower(lower, idx),
                None => upper,
            };
            target.nodes.insert(
                path_data,
                SparseNode::Extension {
                    key,
                    hash: revealed_hash,
                },
            );

            let child_path_data = PathVec::from_slice(child_path.as_ref());
            let child_target = match route_path(&child_path_data) {
                Some(idx) => get_or_create_lower(lower, idx),
                None => upper,
            };
            child_target
                .nodes
                .entry(child_path_data)
                .or_insert(SparseNode::Hash(child_hash));
        }
        Node::Branch(branch) => {
            let mut state_mask = 0u16;

            for (i, child_ref) in branch.choices.iter().enumerate() {
                if !child_ref.is_valid() {
                    continue;
                }
                state_mask |= 1 << i;
                let child_hash = child_ref.compute_hash();
                let mut child_path_data = path_data.clone();
                child_path_data.push(i as u8);

                let child_target = match route_path(&child_path_data) {
                    Some(idx) => get_or_create_lower(lower, idx),
                    None => upper,
                };
                child_target
                    .nodes
                    .entry(child_path_data)
                    .or_insert(SparseNode::Hash(child_hash));
            }

            // Handle branch value - store in same subtrie as branch node
            if !branch.value.is_empty() {
                let mut value_path = path_data.clone();
                value_path.push(16);
                let target = match route_path(&path_data) {
                    Some(idx) => get_or_create_lower(lower, idx),
                    None => upper,
                };
                target.values.insert(value_path, branch.value);
            }

            // Insert the branch node
            let target = match route_path(&path_data) {
                Some(idx) => get_or_create_lower(lower, idx),
                None => upper,
            };
            target.nodes.insert(
                path_data,
                SparseNode::Branch {
                    state_mask,
                    hash: revealed_hash,
                },
            );
        }
    }
    Ok(())
}

/// Ensure a Hash node at the given path is revealed (loaded from DB and decoded).
///
/// For `NodeHash::Hashed` nodes (RLP >= 32 bytes), loads from the DB by path.
/// For `NodeHash::Inline` nodes (RLP < 32 bytes), decodes directly from the
/// embedded bytes — these nodes are never stored separately in the DB.
fn ensure_revealed(
    upper: &mut SparseSubtrie,
    lower: &mut [LowerSubtrie],
    path_data: &[u8],
    provider: &dyn SparseTrieProvider,
) -> Result<(), TrieError> {
    let rlp = match get_node(upper, lower, path_data) {
        Some(SparseNode::Hash(hash)) => match hash {
            NodeHash::Inline(_) => {
                // Inline nodes are embedded in their parent's RLP — not stored
                // separately in the DB. Decode directly from the inline bytes.
                hash.as_ref().to_vec()
            }
            NodeHash::Hashed(h256) => {
                let h256 = *h256;
                provider.get_node(path_data)?.ok_or_else(|| {
                    TrieError::InconsistentTree(Box::new(
                        crate::error::InconsistentTreeError::SparseNodeNotFound {
                            path: Nibbles::from_hex(path_data.to_vec()),
                            hash: h256,
                        },
                    ))
                })?
            }
        },
        _ => return Ok(()),
    };

    remove_node(upper, lower, path_data);
    reveal_node_into(upper, lower, Nibbles::from_hex(path_data.to_vec()), &rlp)
}

fn get_node<'a>(
    upper: &'a SparseSubtrie,
    lower: &'a [LowerSubtrie],
    path_data: &[u8],
) -> Option<&'a SparseNode> {
    match route_path(path_data) {
        None => upper.nodes.get(path_data),
        Some(idx) => match &lower[idx] {
            LowerSubtrie::Revealed(s) | LowerSubtrie::Blind(Some(s)) => s.nodes.get(path_data),
            LowerSubtrie::Blind(None) => None,
        },
    }
}

fn get_node_mut<'a>(
    upper: &'a mut SparseSubtrie,
    lower: &'a mut [LowerSubtrie],
    path_data: &[u8],
) -> Option<&'a mut SparseNode> {
    match route_path(path_data) {
        None => upper.nodes.get_mut(path_data),
        Some(idx) => match &mut lower[idx] {
            LowerSubtrie::Revealed(s) | LowerSubtrie::Blind(Some(s)) => s.nodes.get_mut(path_data),
            LowerSubtrie::Blind(None) => None,
        },
    }
}

fn insert_node(
    upper: &mut SparseSubtrie,
    lower: &mut [LowerSubtrie],
    path_data: PathVec,
    node: SparseNode,
) {
    match route_path(&path_data) {
        None => {
            upper.dirty_nodes.insert(path_data.clone());
            upper.nodes.insert(path_data, node);
        }
        Some(idx) => {
            let subtrie = get_or_create_lower(lower, idx);
            subtrie.dirty_nodes.insert(path_data.clone());
            subtrie.nodes.insert(path_data, node);
        }
    }
}

fn remove_node(
    upper: &mut SparseSubtrie,
    lower: &mut [LowerSubtrie],
    path_data: &[u8],
) -> Option<SparseNode> {
    match route_path(path_data) {
        None => upper.nodes.remove(path_data),
        Some(idx) => match &mut lower[idx] {
            LowerSubtrie::Revealed(s) | LowerSubtrie::Blind(Some(s)) => s.nodes.remove(path_data),
            LowerSubtrie::Blind(None) => None,
        },
    }
}

/// Insert a value into the same subtrie as its leaf node.
/// `node_path` is the path of the leaf node in the nodes HashMap (used for routing).
/// `value_key` is the full leaf path (used as the key in the values HashMap).
fn insert_value(
    upper: &mut SparseSubtrie,
    lower: &mut [LowerSubtrie],
    node_path: &[u8],
    value_key: PathVec,
    value: Vec<u8>,
) {
    match route_path(node_path) {
        None => {
            upper.dirty_values.insert(value_key.clone());
            upper.values.insert(value_key, value);
        }
        Some(idx) => {
            let subtrie = get_or_create_lower(lower, idx);
            subtrie.dirty_values.insert(value_key.clone());
            subtrie.values.insert(value_key, value);
        }
    }
}

/// Remove a value from the same subtrie as its leaf node.
/// `node_path` is the path of the leaf node (used for routing).
/// `value_key` is the full leaf path (used as the key in the values HashMap).
fn remove_value(
    upper: &mut SparseSubtrie,
    lower: &mut [LowerSubtrie],
    node_path: &[u8],
    value_key: &[u8],
) -> Option<Vec<u8>> {
    match route_path(node_path) {
        None => upper.values.remove(value_key),
        Some(idx) => match &mut lower[idx] {
            LowerSubtrie::Revealed(s) | LowerSubtrie::Blind(Some(s)) => s.values.remove(value_key),
            LowerSubtrie::Blind(None) => None,
        },
    }
}

/// Look up a value from the same subtrie as its leaf node.
/// `node_path` is the path of the leaf node (used for routing).
/// `value_key` is the full leaf path (used as the key in the values HashMap).
fn get_value<'a>(
    upper: &'a SparseSubtrie,
    lower: &'a [LowerSubtrie],
    node_path: &[u8],
    value_key: &[u8],
) -> Option<&'a Vec<u8>> {
    match route_path(node_path) {
        None => upper.values.get(value_key),
        Some(idx) => match &lower[idx] {
            LowerSubtrie::Revealed(s) | LowerSubtrie::Blind(Some(s)) => s.values.get(value_key),
            LowerSubtrie::Blind(None) => None,
        },
    }
}

/// Mark a node path as dirty in the correct subtrie (for collect_updates).
fn mark_node_dirty(upper: &mut SparseSubtrie, lower: &mut [LowerSubtrie], path_data: &[u8]) {
    match route_path(path_data) {
        None => {
            upper.dirty_nodes.insert(PathVec::from_slice(path_data));
        }
        Some(idx) => match &mut lower[idx] {
            LowerSubtrie::Revealed(s) | LowerSubtrie::Blind(Some(s)) => {
                s.dirty_nodes.insert(PathVec::from_slice(path_data));
            }
            LowerSubtrie::Blind(None) => {}
        },
    }
}

fn invalidate_branch_hash(upper: &mut SparseSubtrie, lower: &mut [LowerSubtrie], path: &[u8]) {
    if let Some(SparseNode::Branch { hash, .. }) = get_node_mut(upper, lower, path) {
        *hash = None;
    }
    mark_node_dirty(upper, lower, path);
}

/// Update or insert a leaf in the sparse trie.
pub fn update_leaf(
    upper: &mut SparseSubtrie,
    lower: &mut [LowerSubtrie],
    full_path: Nibbles,
    value: Vec<u8>,
    provider: &dyn SparseTrieProvider,
) -> Result<(), TrieError> {
    let path_data = PathVec::from_slice(full_path.as_ref());
    let mut current_path = PathVec::new();

    loop {
        ensure_revealed(upper, lower, &current_path, provider)?;

        let node = get_node(upper, lower, &current_path).cloned();
        let Some(node) = node else {
            // No node at this path - insert a new leaf
            let node_path = current_path.clone();
            let cp_len = current_path.len();
            insert_node(
                upper,
                lower,
                current_path,
                SparseNode::Leaf {
                    key: PathVec::from_slice(&path_data[cp_len..]),
                    hash: None,
                },
            );
            insert_value(upper, lower, &node_path, path_data, value);
            return Ok(());
        };

        match node {
            SparseNode::Empty => {
                let node_path = current_path.clone();
                let cp_len = current_path.len();
                insert_node(
                    upper,
                    lower,
                    current_path,
                    SparseNode::Leaf {
                        key: PathVec::from_slice(&path_data[cp_len..]),
                        hash: None,
                    },
                );
                insert_value(upper, lower, &node_path, path_data, value);
                return Ok(());
            }
            SparseNode::Hash(_) => {
                return Err(TrieError::InconsistentTree(Box::new(
                    crate::error::InconsistentTreeError::RootNotFoundNoHash,
                )));
            }
            SparseNode::Leaf { key, .. } => {
                let existing_key_data: Vec<u8> = key.to_vec();
                let remaining = path_data[current_path.len()..].to_vec();

                if existing_key_data == remaining {
                    // Same key - update value in place
                    if let Some(SparseNode::Leaf { hash, .. }) =
                        get_node_mut(upper, lower, &current_path)
                    {
                        *hash = None;
                    }
                    mark_node_dirty(upper, lower, &current_path);
                    insert_value(upper, lower, &current_path, path_data, value);
                    return Ok(());
                }

                let common_len = existing_key_data
                    .iter()
                    .zip(remaining.iter())
                    .take_while(|(a, b)| a == b)
                    .count();

                let old_full_path: PathVec = current_path
                    .iter()
                    .chain(existing_key_data.iter())
                    .copied()
                    .collect();
                // Remove old value from the subtrie where the old leaf node lives
                let old_value = remove_value(upper, lower, &current_path, &old_full_path);
                remove_node(upper, lower, &current_path);

                let branch_insert_path: PathVec = if common_len > 0 {
                    insert_node(
                        upper,
                        lower,
                        current_path.clone(),
                        SparseNode::Extension {
                            key: PathVec::from_slice(&remaining[..common_len]),
                            hash: None,
                        },
                    );
                    current_path
                        .iter()
                        .chain(remaining[..common_len].iter())
                        .copied()
                        .collect()
                } else {
                    current_path.clone()
                };

                let old_branch_nibble = existing_key_data[common_len];
                let new_branch_nibble = remaining[common_len];
                let state_mask = (1u16 << old_branch_nibble) | (1u16 << new_branch_nibble);

                insert_node(
                    upper,
                    lower,
                    branch_insert_path.clone(),
                    SparseNode::Branch {
                        state_mask,
                        hash: None,
                    },
                );

                // Old leaf child
                let mut old_child_path = branch_insert_path.clone();
                old_child_path.push(old_branch_nibble);
                let old_child_node_path = old_child_path.clone();
                insert_node(
                    upper,
                    lower,
                    old_child_path,
                    SparseNode::Leaf {
                        key: PathVec::from_slice(&existing_key_data[common_len + 1..]),
                        hash: None,
                    },
                );
                if let Some(old_val) = old_value {
                    insert_value(upper, lower, &old_child_node_path, old_full_path, old_val);
                }

                // New leaf child
                let mut new_child_path = branch_insert_path;
                new_child_path.push(new_branch_nibble);
                let new_child_node_path = new_child_path.clone();
                insert_node(
                    upper,
                    lower,
                    new_child_path,
                    SparseNode::Leaf {
                        key: PathVec::from_slice(&remaining[common_len + 1..]),
                        hash: None,
                    },
                );
                insert_value(upper, lower, &new_child_node_path, path_data, value);

                return Ok(());
            }
            SparseNode::Extension { key, .. } => {
                let ext_key_data: Vec<u8> = key.to_vec();
                let remaining = path_data[current_path.len()..].to_vec();

                let common_len = ext_key_data
                    .iter()
                    .zip(remaining.iter())
                    .take_while(|(a, b)| a == b)
                    .count();

                if common_len == ext_key_data.len() {
                    // Invalidate extension hash since a descendant will be modified
                    if let Some(SparseNode::Extension { hash, .. }) =
                        get_node_mut(upper, lower, &current_path)
                    {
                        *hash = None;
                    }
                    mark_node_dirty(upper, lower, &current_path);
                    current_path.extend_from_slice(&ext_key_data);
                    continue;
                }

                remove_node(upper, lower, &current_path);

                let branch_insert_path: PathVec = if common_len > 0 {
                    insert_node(
                        upper,
                        lower,
                        current_path.clone(),
                        SparseNode::Extension {
                            key: PathVec::from_slice(&ext_key_data[..common_len]),
                            hash: None,
                        },
                    );
                    current_path
                        .iter()
                        .chain(ext_key_data[..common_len].iter())
                        .copied()
                        .collect()
                } else {
                    current_path.clone()
                };

                let ext_nibble = ext_key_data[common_len];
                let new_nibble = remaining[common_len];
                let state_mask = (1u16 << ext_nibble) | (1u16 << new_nibble);

                insert_node(
                    upper,
                    lower,
                    branch_insert_path.clone(),
                    SparseNode::Branch {
                        state_mask,
                        hash: None,
                    },
                );

                let mut old_child_path = branch_insert_path.clone();
                old_child_path.push(ext_nibble);
                let ext_remainder = &ext_key_data[common_len + 1..];
                if ext_remainder.is_empty() {
                    let old_ext_child_path: PathVec = current_path
                        .iter()
                        .chain(ext_key_data.iter())
                        .copied()
                        .collect();
                    if let Some(child_node) = remove_node(upper, lower, &old_ext_child_path) {
                        insert_node(upper, lower, old_child_path, child_node);
                    }
                } else {
                    insert_node(
                        upper,
                        lower,
                        old_child_path.clone(),
                        SparseNode::Extension {
                            key: PathVec::from_slice(ext_remainder),
                            hash: None,
                        },
                    );
                    let old_ext_child_path: PathVec = current_path
                        .iter()
                        .chain(ext_key_data.iter())
                        .copied()
                        .collect();
                    let new_ext_child_path: PathVec = old_child_path
                        .iter()
                        .chain(ext_remainder.iter())
                        .copied()
                        .collect();
                    if old_ext_child_path != new_ext_child_path
                        && let Some(child_node) = remove_node(upper, lower, &old_ext_child_path)
                    {
                        insert_node(upper, lower, new_ext_child_path, child_node);
                    }
                }

                // New leaf child
                let mut new_child_path = branch_insert_path;
                new_child_path.push(new_nibble);
                let new_child_node_path = new_child_path.clone();
                insert_node(
                    upper,
                    lower,
                    new_child_path,
                    SparseNode::Leaf {
                        key: PathVec::from_slice(&remaining[common_len + 1..]),
                        hash: None,
                    },
                );
                insert_value(upper, lower, &new_child_node_path, path_data, value);

                return Ok(());
            }
            SparseNode::Branch { state_mask, .. } => {
                let remaining = &path_data[current_path.len()..];
                if remaining.is_empty() {
                    // Value stored at the branch itself
                    let mut value_path = current_path.clone();
                    value_path.push(16);
                    insert_value(upper, lower, &current_path, value_path, value);
                    invalidate_branch_hash(upper, lower, &current_path);
                    return Ok(());
                }

                let nibble = remaining[0];
                let mut child_path = current_path.clone();
                child_path.push(nibble);

                if state_mask & (1 << nibble) == 0 {
                    if let Some(SparseNode::Branch {
                        state_mask: mask,
                        hash,
                    }) = get_node_mut(upper, lower, &current_path)
                    {
                        *mask |= 1 << nibble;
                        *hash = None;
                    }
                    mark_node_dirty(upper, lower, &current_path);
                } else {
                    invalidate_branch_hash(upper, lower, &current_path);
                }

                current_path = child_path;
            }
        }
    }
}

/// Remove a leaf from the sparse trie.
pub fn remove_leaf(
    upper: &mut SparseSubtrie,
    lower: &mut [LowerSubtrie],
    full_path: Nibbles,
    provider: &dyn SparseTrieProvider,
) -> Result<(), TrieError> {
    let path_data = PathVec::from_slice(full_path.as_ref());
    let mut walk_path = PathVec::new();
    let mut parent_stack: Vec<PathVec> = Vec::new();

    loop {
        ensure_revealed(upper, lower, &walk_path, provider)?;

        let node = get_node(upper, lower, &walk_path).cloned();
        let Some(node) = node else {
            return Ok(());
        };

        match node {
            SparseNode::Empty => return Ok(()),
            SparseNode::Hash(_) => {
                return Err(TrieError::InconsistentTree(Box::new(
                    crate::error::InconsistentTreeError::RootNotFoundNoHash,
                )));
            }
            SparseNode::Leaf { key, .. } => {
                let remaining = &path_data[walk_path.len()..];
                if key.as_ref() != remaining {
                    return Ok(());
                }
                // Leaf node is at walk_path, value key is path_data
                remove_node(upper, lower, &walk_path);
                remove_value(upper, lower, &walk_path, &path_data);
                collapse_after_removal(upper, lower, &parent_stack, provider)?;
                return Ok(());
            }
            SparseNode::Extension { key, .. } => {
                let remaining = &path_data[walk_path.len()..];
                let ext_key_data = key.as_ref();
                if !remaining.starts_with(ext_key_data) {
                    return Ok(());
                }
                // Invalidate extension hash since a descendant will be removed
                if let Some(SparseNode::Extension { hash, .. }) =
                    get_node_mut(upper, lower, &walk_path)
                {
                    *hash = None;
                }
                mark_node_dirty(upper, lower, &walk_path);
                parent_stack.push(walk_path.clone());
                walk_path.extend_from_slice(ext_key_data);
            }
            SparseNode::Branch { state_mask, .. } => {
                let remaining = &path_data[walk_path.len()..];
                if remaining.is_empty() {
                    // Branch value removal - branch is at walk_path
                    let mut value_path = walk_path.clone();
                    value_path.push(16);
                    remove_value(upper, lower, &walk_path, &value_path);
                    invalidate_branch_hash(upper, lower, &walk_path);
                    collapse_branch_if_needed(upper, lower, &walk_path, &parent_stack, provider)?;
                    return Ok(());
                }

                let nibble = remaining[0];
                if state_mask & (1 << nibble) == 0 {
                    return Ok(());
                }
                // Invalidate branch hash since a descendant will be removed
                invalidate_branch_hash(upper, lower, &walk_path);
                parent_stack.push(walk_path.clone());
                walk_path.push(nibble);
            }
        }
    }
}

fn collapse_after_removal(
    upper: &mut SparseSubtrie,
    lower: &mut [LowerSubtrie],
    parent_stack: &[PathVec],
    provider: &dyn SparseTrieProvider,
) -> Result<(), TrieError> {
    for (i, parent_path) in parent_stack.iter().enumerate().rev() {
        let parent = get_node(upper, lower, parent_path).cloned();
        match parent {
            Some(SparseNode::Branch { .. }) => {
                collapse_branch_if_needed(upper, lower, parent_path, &parent_stack[..i], provider)?;
                // After branch collapse, the node at parent_path may have been replaced
                // with a leaf or extension. Check if this needs further merging with
                // its own parent extension.
                merge_extension_child(upper, lower, parent_path);
            }
            Some(SparseNode::Extension { .. }) => {
                merge_extension_child(upper, lower, parent_path);
            }
            _ => {}
        }
    }
    Ok(())
}

/// If the node at `ext_path` is an extension and its child is a leaf or another
/// extension, merge them into a single node.
fn merge_extension_child(upper: &mut SparseSubtrie, lower: &mut [LowerSubtrie], ext_path: &[u8]) {
    let ext = get_node(upper, lower, ext_path).cloned();
    let Some(SparseNode::Extension { key: ext_key, .. }) = ext else {
        return;
    };

    let child_path: PathVec = ext_path
        .iter()
        .chain(ext_key.as_ref().iter())
        .copied()
        .collect();
    let child = get_node(upper, lower, &child_path).cloned();

    match child {
        Some(SparseNode::Leaf { key: leaf_key, .. }) => {
            // Extension + Leaf → single Leaf with combined key
            let child_full_path: PathVec = child_path
                .iter()
                .chain(leaf_key.as_ref().iter())
                .copied()
                .collect();
            let value = remove_value(upper, lower, &child_path, &child_full_path);
            remove_node(upper, lower, &child_path);
            remove_node(upper, lower, ext_path);

            let mut merged_key: PathVec = ext_key;
            merged_key.extend_from_slice(&leaf_key);

            let new_full_path: PathVec = ext_path
                .iter()
                .copied()
                .chain(merged_key.iter().copied())
                .collect();

            insert_node(
                upper,
                lower,
                PathVec::from_slice(ext_path),
                SparseNode::Leaf {
                    key: merged_key,
                    hash: None,
                },
            );
            if let Some(v) = value {
                insert_value(upper, lower, ext_path, new_full_path, v);
            }
        }
        Some(SparseNode::Extension {
            key: child_ext_key, ..
        }) => {
            // Extension + Extension → single Extension with combined key
            let grandchild_path: PathVec = child_path
                .iter()
                .chain(child_ext_key.as_ref().iter())
                .copied()
                .collect();
            remove_node(upper, lower, &child_path);
            remove_node(upper, lower, ext_path);

            let mut merged_key: PathVec = ext_key;
            merged_key.extend_from_slice(&child_ext_key);

            insert_node(
                upper,
                lower,
                PathVec::from_slice(ext_path),
                SparseNode::Extension {
                    key: merged_key.clone(),
                    hash: None,
                },
            );

            // Move the grandchild node to the new child path
            let new_child_path: PathVec =
                ext_path.iter().chain(merged_key.iter()).copied().collect();
            if *grandchild_path != *new_child_path
                && let Some(grandchild) = remove_node(upper, lower, &grandchild_path)
            {
                insert_node(upper, lower, new_child_path, grandchild);
            }
        }
        None => {
            // Child is gone, remove the extension
            remove_node(upper, lower, ext_path);
        }
        _ => {}
    }
}

fn collapse_branch_if_needed(
    upper: &mut SparseSubtrie,
    lower: &mut [LowerSubtrie],
    branch_path: &[u8],
    _parent_stack: &[PathVec],
    provider: &dyn SparseTrieProvider,
) -> Result<(), TrieError> {
    let branch = get_node(upper, lower, branch_path).cloned();
    let Some(SparseNode::Branch { state_mask, .. }) = branch else {
        return Ok(());
    };

    let mut remaining_children = Vec::new();
    for i in 0..16u8 {
        if state_mask & (1 << i) != 0 {
            let mut child_path = PathVec::from_slice(branch_path);
            child_path.push(i);
            if get_node(upper, lower, &child_path).is_some() {
                remaining_children.push(i);
            }
        }
    }

    let mut value_path = PathVec::from_slice(branch_path);
    value_path.push(16);
    // Branch value is stored in the same subtrie as the branch node
    let has_value = get_value(upper, lower, branch_path, &value_path).is_some();

    match (remaining_children.len(), has_value) {
        (0, false) => {
            remove_node(upper, lower, branch_path);
            insert_node(
                upper,
                lower,
                PathVec::from_slice(branch_path),
                SparseNode::Empty,
            );
        }
        (0, true) => {
            // Branch with only a value, no children - collapse to leaf
            let value = remove_value(upper, lower, branch_path, &value_path);
            remove_node(upper, lower, branch_path);
            insert_node(
                upper,
                lower,
                PathVec::from_slice(branch_path),
                SparseNode::Leaf {
                    key: smallvec::smallvec![16],
                    hash: None,
                },
            );
            if let Some(v) = value {
                let leaf_full_path: PathVec = branch_path
                    .iter()
                    .chain(std::iter::once(&16u8))
                    .copied()
                    .collect();
                // New leaf is at branch_path
                insert_value(upper, lower, branch_path, leaf_full_path, v);
            }
        }
        (1, false) => {
            let only_child_nibble = remaining_children[0];
            let mut child_path = PathVec::from_slice(branch_path);
            child_path.push(only_child_nibble);

            ensure_revealed(upper, lower, &child_path, provider)?;

            let child = get_node(upper, lower, &child_path).cloned();
            match child {
                Some(SparseNode::Leaf { key, .. }) => {
                    let child_full_path: PathVec =
                        child_path.iter().chain(key.iter()).copied().collect();
                    // Remove value from child leaf's subtrie
                    let value = remove_value(upper, lower, &child_path, &child_full_path);
                    remove_node(upper, lower, &child_path);
                    remove_node(upper, lower, branch_path);

                    let mut new_key: PathVec = smallvec::smallvec![only_child_nibble];
                    new_key.extend_from_slice(&key);

                    let new_full_path: PathVec = branch_path
                        .iter()
                        .copied()
                        .chain(new_key.iter().copied())
                        .collect();

                    insert_node(
                        upper,
                        lower,
                        PathVec::from_slice(branch_path),
                        SparseNode::Leaf {
                            key: new_key,
                            hash: None,
                        },
                    );
                    if let Some(v) = value {
                        // New leaf is at branch_path
                        insert_value(upper, lower, branch_path, new_full_path, v);
                    }
                }
                Some(SparseNode::Extension { key, .. }) => {
                    let old_ext_child_path: PathVec =
                        child_path.iter().chain(key.iter()).copied().collect();
                    remove_node(upper, lower, &child_path);
                    remove_node(upper, lower, branch_path);

                    let mut new_key: PathVec = smallvec::smallvec![only_child_nibble];
                    new_key.extend_from_slice(&key);

                    insert_node(
                        upper,
                        lower,
                        PathVec::from_slice(branch_path),
                        SparseNode::Extension {
                            key: new_key.clone(),
                            hash: None,
                        },
                    );

                    let new_ext_child_path: PathVec =
                        branch_path.iter().chain(new_key.iter()).copied().collect();
                    if *old_ext_child_path != *new_ext_child_path
                        && let Some(ext_child) = remove_node(upper, lower, &old_ext_child_path)
                    {
                        insert_node(upper, lower, new_ext_child_path, ext_child);
                    }
                }
                Some(SparseNode::Branch { .. }) => {
                    remove_node(upper, lower, branch_path);
                    insert_node(
                        upper,
                        lower,
                        PathVec::from_slice(branch_path),
                        SparseNode::Extension {
                            key: smallvec::smallvec![only_child_nibble],
                            hash: None,
                        },
                    );
                }
                _ => {
                    if let Some(SparseNode::Branch {
                        state_mask: mask,
                        hash,
                    }) = get_node_mut(upper, lower, branch_path)
                    {
                        *mask = 1 << only_child_nibble;
                        *hash = None;
                    }
                    mark_node_dirty(upper, lower, branch_path);
                }
            }
        }
        _ => {
            let mut new_mask = 0u16;
            for &nibble in &remaining_children {
                new_mask |= 1 << nibble;
            }
            if let Some(SparseNode::Branch {
                state_mask: mask,
                hash,
            }) = get_node_mut(upper, lower, branch_path)
            {
                *mask = new_mask;
                *hash = None;
            }
            mark_node_dirty(upper, lower, branch_path);
        }
    }

    Ok(())
}

// --- Parallel prefetch support ---

/// Reveal a node within a single subtrie (without routing to upper/lower).
/// Used during parallel prefetching where each subtrie is processed independently.
fn reveal_node_into_subtrie(
    subtrie: &mut SparseSubtrie,
    path_data: &[u8],
    rlp: &[u8],
) -> Result<(), TrieError> {
    let node = Node::decode(rlp).map_err(TrieError::RLPDecode)?;
    let revealed_hash = Some(NodeHash::from_encoded(rlp));

    match node {
        Node::Leaf(leaf) => {
            let full_path: PathVec = path_data
                .iter()
                .chain(leaf.partial.as_ref().iter())
                .copied()
                .collect();
            let key = PathVec::from_slice(leaf.partial.as_ref());
            subtrie.nodes.insert(
                PathVec::from_slice(path_data),
                SparseNode::Leaf {
                    key,
                    hash: revealed_hash,
                },
            );
            subtrie.values.insert(full_path, leaf.value);
        }
        Node::Extension(ext) => {
            let key = PathVec::from_slice(ext.prefix.as_ref());
            let child_hash = ext.child.compute_hash();
            let child_path: PathVec = path_data
                .iter()
                .chain(ext.prefix.as_ref().iter())
                .copied()
                .collect();
            subtrie.nodes.insert(
                PathVec::from_slice(path_data),
                SparseNode::Extension {
                    key,
                    hash: revealed_hash,
                },
            );
            subtrie
                .nodes
                .entry(child_path)
                .or_insert(SparseNode::Hash(child_hash));
        }
        Node::Branch(branch) => {
            let mut state_mask = 0u16;
            for (i, child_ref) in branch.choices.iter().enumerate() {
                if !child_ref.is_valid() {
                    continue;
                }
                state_mask |= 1 << i;
                let child_hash = child_ref.compute_hash();
                let mut child_path = PathVec::from_slice(path_data);
                child_path.push(i as u8);
                subtrie
                    .nodes
                    .entry(child_path)
                    .or_insert(SparseNode::Hash(child_hash));
            }
            if !branch.value.is_empty() {
                let mut value_path = PathVec::from_slice(path_data);
                value_path.push(16);
                subtrie.values.insert(value_path, branch.value);
            }
            subtrie.nodes.insert(
                PathVec::from_slice(path_data),
                SparseNode::Branch {
                    state_mask,
                    hash: revealed_hash,
                },
            );
        }
    }
    Ok(())
}

/// Ensure a Hash node at the given path is revealed within a single subtrie.
/// Like `ensure_revealed` but doesn't use upper/lower routing.
fn ensure_revealed_in_subtrie(
    subtrie: &mut SparseSubtrie,
    path_data: &[u8],
    provider: &dyn SparseTrieProvider,
) -> Result<(), TrieError> {
    let rlp = match subtrie.nodes.get(path_data) {
        Some(SparseNode::Hash(hash)) => match hash {
            NodeHash::Inline(_) => hash.as_ref().to_vec(),
            NodeHash::Hashed(h256) => {
                let h256 = *h256;
                provider.get_node(path_data)?.ok_or_else(|| {
                    TrieError::InconsistentTree(Box::new(
                        crate::error::InconsistentTreeError::SparseNodeNotFound {
                            path: Nibbles::from_hex(path_data.to_vec()),
                            hash: h256,
                        },
                    ))
                })?
            }
        },
        _ => return Ok(()),
    };

    subtrie.nodes.remove(path_data);
    reveal_node_into_subtrie(subtrie, path_data, &rlp)
}

/// Pre-reveal Hash nodes along the paths that will be updated.
/// Upper subtrie is walked sequentially (at most ~17 DB reads),
/// then lower subtries are walked in parallel via rayon.
pub fn prefetch_paths(
    upper: &mut SparseSubtrie,
    lower: &mut [LowerSubtrie],
    paths: &[PathVec],
    provider: &dyn SparseTrieProvider,
) -> Result<(), TrieError> {
    if paths.is_empty() {
        return Ok(());
    }

    // Phase 1: Walk upper subtrie for each path (sequential).
    // Determine where each path enters a lower subtrie.
    // Upper subtrie has nodes at depth < 2, so this is at most ~17 DB reads.
    let mut lower_entries: Vec<Vec<(usize, PathVec)>> = vec![Vec::new(); 256];

    for (path_idx, path) in paths.iter().enumerate() {
        let mut current = PathVec::new();
        loop {
            if let Some(idx) = route_path(&current) {
                // Path has entered a lower subtrie
                lower_entries[idx].push((path_idx, current));
                break;
            }
            // Still in upper — reveal if needed
            ensure_revealed(upper, lower, &current, provider)?;
            match upper.nodes.get(current.as_slice()) {
                Some(SparseNode::Branch { state_mask, .. }) => {
                    let state_mask = *state_mask;
                    if current.len() >= path.len() {
                        break;
                    }
                    let nibble = path[current.len()];
                    if nibble >= 16 || state_mask & (1 << nibble) == 0 {
                        break;
                    }
                    current.push(nibble);
                }
                Some(SparseNode::Extension { key, .. }) => {
                    let key_clone = key.clone();
                    current.extend_from_slice(&key_clone);
                }
                _ => break,
            }
        }
    }

    // Phase 2: Ensure needed lower subtries exist (sequential).
    for (idx, entries) in lower_entries.iter().enumerate() {
        if !entries.is_empty() {
            get_or_create_lower(lower, idx);
        }
    }

    // Phase 3: Parallel prefetch within lower subtries.
    lower.par_iter_mut().enumerate().try_for_each(
        |(idx, lower_subtrie)| -> Result<(), TrieError> {
            let entries = &lower_entries[idx];
            if entries.is_empty() {
                return Ok(());
            }
            let subtrie = match lower_subtrie {
                LowerSubtrie::Revealed(s) => s,
                _ => return Ok(()),
            };
            for (path_idx, start) in entries {
                let full_path = &paths[*path_idx];
                let mut current = start.clone();
                loop {
                    ensure_revealed_in_subtrie(subtrie, &current, provider)?;
                    match subtrie.nodes.get(current.as_slice()) {
                        Some(SparseNode::Branch { state_mask, .. }) => {
                            let state_mask = *state_mask;
                            if current.len() >= full_path.len() {
                                break;
                            }
                            let nibble = full_path[current.len()];
                            if nibble >= 16 || state_mask & (1 << nibble) == 0 {
                                break;
                            }
                            current.push(nibble);
                        }
                        Some(SparseNode::Extension { key, .. }) => {
                            let key_clone = key.clone();
                            current.extend_from_slice(&key_clone);
                        }
                        _ => break,
                    }
                }
            }
            Ok(())
        },
    )?;

    Ok(())
}
