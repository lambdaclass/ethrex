use std::collections::{BTreeMap, VecDeque};

use ethereum_types::H256;
use ethrex_crypto::keccak::keccak_hash;
use ethrex_rlp::decode::RLPDecode;

use crate::{
    ProofTrie, Trie, TrieError,
    nibbles::Nibbles,
    node::{Node, NodeRef},
    node_hash::NodeHash,
};

/// Verifies that the key value range belongs to the trie with the given root given the edge proofs for the range
/// Also returns true if there is more state to be fetched (aka if there are more keys to the right of the given range)
pub fn verify_range<V: AsRef<[u8]>>(
    root: H256,
    left_bound: &H256,
    keys: &[H256],
    values: &[V],
    proof: &[Vec<u8>],
) -> Result<bool, TrieError> {
    // Validate range
    if keys.len() != values.len() {
        return Err(TrieError::Verify(format!(
            "inconsistent proof data, got {} keys and {} values",
            keys.len(),
            values.len()
        )));
    }
    // Check that the key range is monotonically increasing
    for keys in keys.windows(2) {
        if keys[0] >= keys[1] {
            return Err(TrieError::Verify(String::from(
                "key range is not monotonically increasing",
            )));
        }
    }
    // Check for empty values
    if values.iter().any(|value| value.as_ref().is_empty()) {
        return Err(TrieError::Verify(String::from(
            "value range contains empty value",
        )));
    }

    let mut trie = Trie::stateless();

    // Special Case: No proofs given, the range is expected to be the full set of leaves
    if proof.is_empty() {
        // Check that the trie constructed from the given keys and values has the expected root
        for (key, value) in keys.iter().zip(values.iter()) {
            trie.insert(key.0.to_vec(), value.as_ref().to_vec())?;
        }
        let hash = trie.hash()?;
        if hash != root {
            return Err(TrieError::Verify(format!(
                "invalid proof, expected root hash {root}, got  {hash}",
            )));
        }
        return Ok(false);
    }

    // Special Case: One edge proof, no range given, there are no more values in the trie
    if keys.is_empty() {
        // We need to check that the proof confirms the non-existance of the first key
        // and that there are no more elements to the right of the first key
        let result = process_proof_nodes(proof, root.into(), (*left_bound, None), None)?;
        if result.num_right_references > 0 || !result.left_value.is_empty() {
            return Err(TrieError::Verify(
                "no keys returned but more are available on the trie".to_string(),
            ));
        } else {
            return Ok(false);
        };
    }

    let last_key = keys.last().unwrap();

    // Special Case: There is only one element and the two edge keys are the same
    if keys.len() == 1 && left_bound == last_key {
        // We need to check that the proof confirms the existence of the first key
        if left_bound != &keys[0] {
            return Err(TrieError::Verify(
                "correct proof but invalid key".to_string(),
            ));
        }
        let result = process_proof_nodes(
            proof,
            root.into(),
            (*left_bound, Some(*last_key)),
            Some(*keys.first().unwrap()),
        )?;
        if result.left_value != values[0].as_ref() {
            return Err(TrieError::Verify(
                "correct proof but invalid data".to_string(),
            ));
        }
        return Ok(result.num_right_references > 0);
    }

    // Regular Case: Two edge proofs
    if left_bound >= last_key {
        return Err(TrieError::Verify("invalid edge keys".to_string()));
    }

    // Process proofs to check if they are valid.
    let result = process_proof_nodes(
        proof,
        root.into(),
        (*left_bound, Some(*last_key)),
        Some(*keys.first().unwrap()),
    )?;

    // Reconstruct the internal nodes by inserting the elements on the range
    for (key, value) in keys.iter().zip(values.iter()) {
        trie.insert(key.0.to_vec(), value.as_ref().to_vec())?;
    }

    // Fill up the state with the nodes from the proof
    let mut trie = ProofTrie::from(trie);
    for (partial_path, external_ref) in result.external_references {
        trie.insert(partial_path, external_ref)?;
    }

    // Check that the hash is the one we expected (aka the trie was properly reconstructed from the edge proofs and the range)
    let hash = trie.hash();
    if hash != root {
        return Err(TrieError::Verify(format!(
            "invalid proof, expected root hash {root}, got  {hash}",
        )));
    }
    Ok(result.num_right_references > 0)
}

/// Parsed range proof
/// Has a mapping of node hashes to the encoded node data, useful for verifying the proof.
struct RangeProof<'a> {
    node_refs: BTreeMap<H256, &'a [u8]>,
}

impl<'a> From<&'a [Vec<u8>]> for RangeProof<'a> {
    fn from(proof: &'a [Vec<u8>]) -> Self {
        let node_refs = proof
            .iter()
            .map(|node| {
                let hash = H256(keccak_hash(node));
                let encoded_data = node.as_slice();
                (hash, encoded_data)
            })
            .collect();
        RangeProof { node_refs }
    }
}

impl RangeProof<'_> {
    /// Get a node by its hash, returning `None` if the node is not present in the proof.
    /// If the node is inline in the hash, it will be decoded directly from it.
    fn get_node(&self, hash: NodeHash) -> Result<Option<Node>, TrieError> {
        let encoded_node = match hash {
            NodeHash::Hashed(hash) => self.node_refs.get(&hash).copied(),
            NodeHash::Inline(_) => Some(hash.as_ref()),
        };
        Ok(encoded_node.map(Node::decode).transpose()?)
    }
}

/// Iterate over all provided proofs starting from the root and generate a set of hashes that fall
/// outside the verification bounds.
///
/// For example, calling this function with the proofs for the range `(hash_a, hash_b)` will return
/// all node references contained within those proofs except the ones that are contained between
/// `hash_a` and `hash_b` lexicographically.
///
/// Also returns the number of references strictly to the right of the bounds. If the right bound
/// is unbounded (aka. not provided), all nodes to the right (inclusive) of the left bound will
/// be counted. Leaf nodes are not counted (the leaf nodes within the proof do not count).
struct ProofProcessingResult {
    external_references: Vec<(Nibbles, NodeHash)>,
    left_value: Vec<u8>,
    num_right_references: usize,
}

fn process_proof_nodes(
    raw_proof: &[Vec<u8>],
    root: NodeHash,
    bounds: (H256, Option<H256>),
    first_key: Option<H256>,
) -> Result<ProofProcessingResult, TrieError> {
    // Convert `H256` bounds into `Nibble` bounds for convenience.
    let bounds = (
        Nibbles::from_bytes(&bounds.0.0),
        // In case there's no right bound, we use the left bound as the right bound.
        Nibbles::from_bytes(&bounds.1.unwrap_or(bounds.0).0),
    );
    let first_key = first_key.map(|first_key| Nibbles::from_bytes(&first_key.0));

    // Generate a map of node hashes to node data for obtaining proof nodes given their hashes.
    let proof = RangeProof::from(raw_proof);

    // Initialize the external references container.
    let mut external_references = Vec::new();
    let mut left_value = Vec::new();
    let mut num_right_references = 0;

    // Iterate over the proofs tree.
    //
    // The children are processed as follows:
    //   1. Nodes that fall within bounds will be filtered out.
    //   2. Nodes for which we have the proof will push themselves into the queue.
    //   3. Nodes for which we do not have the proof are treated as external references.
    let mut stack = VecDeque::from_iter([(
        Nibbles::default(),
        proof.get_node(root)?.ok_or(TrieError::Verify(format!(
            "root node missing from proof: {root:?}"
        )))?,
    )]);
    while let Some((mut current_path, current_node)) = stack.pop_front() {
        let value = match current_node {
            Node::Branch(node) => {
                for (index, choice) in node.choices.into_iter().enumerate() {
                    if !choice.is_valid() {
                        continue;
                    }
                    num_right_references += visit_child_node(
                        &mut stack,
                        &mut external_references,
                        &proof,
                        &bounds,
                        first_key.as_ref(),
                        current_path.append_new(index as u8),
                        choice,
                    )?;
                }
                node.value
            }
            Node::Extension(node) => {
                current_path.extend(&node.prefix);
                num_right_references += visit_child_node(
                    &mut stack,
                    &mut external_references,
                    &proof,
                    &bounds,
                    first_key.as_ref(),
                    current_path.clone(),
                    node.child,
                )?;
                Default::default()
            }
            Node::Leaf(node) => node.value,
        };

        if !value.is_empty() && current_path == bounds.0 {
            left_value = value.to_vec();
        }
    }

    let result = ProofProcessingResult {
        external_references,
        left_value,
        num_right_references,
    };
    Ok(result)
}

fn visit_child_node(
    stack: &mut VecDeque<(Nibbles, Node)>,
    external_refs: &mut Vec<(Nibbles, NodeHash)>,
    proof: &RangeProof,
    (left_bound, right_bound): &(Nibbles, Nibbles),
    first_key: Option<&Nibbles>,
    mut partial_path: Nibbles,
    child: NodeRef,
) -> Result<usize, TrieError> {
    let cmp_l = left_bound.compare_prefix(&partial_path);
    let cmp_r = right_bound.compare_prefix(&partial_path);

    // We don't process nodes that lie inside bounds
    // left_bound < partial_path < right_bound
    if cmp_l.is_lt() && cmp_r.is_gt() {
        return Ok(0);
    }
    let NodeRef::Hash(hash) = child else {
        // This is unreachable because the nodes have just been decoded, therefore only
        // having hash references.
        unreachable!()
    };

    match proof.get_node(hash)? {
        Some(node) => {
            // Handle proofs of absences in the left bound.
            //
            // When the proof proves an absence, the left bound won't end up in a leaf
            // and there will not be a path that the external references can follow to
            // avoid inconsistent trie errors. In those cases, there will be subtrees
            // completely outside of the verification range. Since we have the hash of
            // the entire subtree within the proof, we can just treat it as an external
            // reference and ignore everything inside.
            //
            // This optimization should not be a problem because we're the ones that
            // have computed the hash of the subtree (it's not part of the proof)
            // therefore we can always be sure it's representing the data the proof has
            // provided.
            //
            // Note: The right bound cannot be a proof of absence because it cannot be
            //   specified externally, and is always keys.last(). In other words, if
            //   there is a right bound, it'll always exist.
            if first_key.is_some_and(|fk| fk.compare_prefix(&partial_path).is_gt()) {
                // The subtree is not part of the path to the first available key. Treat
                // the entire subtree as an external reference.
                external_refs.push((partial_path, hash));
            } else {
                // Append implicit leaf extension when pushing leaves.
                if let Node::Leaf(node) = &node {
                    partial_path.extend(&node.partial);
                }
                if right_bound.compare_prefix(&partial_path).is_lt() {
                    external_refs.push((partial_path.clone(), hash));
                }

                stack.push_back((partial_path, node));
            }
        }
        None => {
            if cmp_l.is_eq() || cmp_r.is_eq() {
                return Err(TrieError::Verify(format!("proof node missing: {hash:?}")));
            }

            external_refs.push((partial_path, hash));
        }
    }

    // left_bound < partial_path && right_bound < partial_path
    let n_right_references = if cmp_l.is_lt() && cmp_r.is_lt() { 1 } else { 0 };

    Ok(n_right_references)
}
