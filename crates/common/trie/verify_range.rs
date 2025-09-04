use std::collections::{BTreeMap, VecDeque};

use ethereum_types::H256;
use sha3::{Digest, Keccak256};

use crate::{
    ProofTrie, Trie, TrieError, ValueRLP,
    nibbles::Nibbles,
    node::{Node, NodeRef},
    node_hash::NodeHash,
};

/// Verifies that the key value range belongs to the trie with the given root given the edge proofs for the range
/// Also returns true if there is more state to be fetched (aka if there are more keys to the right of the given range)
pub fn verify_range(
    root: H256,
    left_bound: &H256,
    keys: &[H256],
    values: &[ValueRLP],
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
    if values.iter().any(|value| value.is_empty()) {
        return Err(TrieError::Verify(String::from(
            "value range contains empty value",
        )));
    }

    let mut trie = Trie::stateless();

    // Special Case: No proofs given, the range is expected to be the full set of leaves
    if proof.is_empty() {
        // Check that the trie constructed from the given keys and values has the expected root
        for (key, value) in keys.iter().zip(values.iter()) {
            trie.insert(key.0.to_vec(), value.clone())?;
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
        if result.left_value != values[0] {
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
        trie.insert(key.0.to_vec(), value.clone())?;
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
                let hash = H256::from_slice(&Keccak256::new_with_prefix(node).finalize());
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
        Ok(encoded_node.map(Node::decode_raw).transpose()?)
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
                Vec::new()
            }
            Node::Leaf(node) => node.value,
        };

        if !value.is_empty() && current_path == bounds.0 {
            left_value = value.clone();
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

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::collection::{btree_set, vec};
    use proptest::prelude::any;
    use proptest::{bool, proptest};
    use std::str::FromStr;

    #[test]
    fn verify_range_proof_of_absence() {
        let mut trie = Trie::new_temp();
        trie.insert(vec![0x00, 0x01], vec![0x00]).unwrap();
        trie.insert(vec![0x00, 0x02], vec![0x00]).unwrap();
        trie.insert(vec![0x01; 32], vec![0x00]).unwrap();

        // Obtain a proof of absence for a node that will return a branch completely outside the
        // path of the first available key.
        let mut proof = trie.get_proof(&vec![0x00, 0xFF]).unwrap();
        proof.extend(trie.get_proof(&vec![0x01; 32]).unwrap());

        let root = trie.hash_no_commit();
        let keys = &[H256([0x01u8; 32])];
        let values = &[vec![0x00u8]];

        let mut first_key = H256([0xFF; 32]);
        first_key.0[0] = 0;

        let fetch_more = verify_range(root, &first_key, keys, values, &proof).unwrap();
        assert!(!fetch_more);
    }

    #[test]
    fn verify_range_regular_case_only_branch_nodes() {
        // The trie will have keys and values ranging from 25-100
        // We will prove the range from 50-75
        // Note values are written as hashes in the form i -> [i;32]
        let mut trie = Trie::new_temp();
        for k in 25..100_u8 {
            trie.insert([k; 32].to_vec(), [k; 32].to_vec()).unwrap()
        }
        let mut proof = trie.get_proof(&[50; 32].to_vec()).unwrap();
        proof.extend(trie.get_proof(&[75; 32].to_vec()).unwrap());
        let root = trie.hash().unwrap();
        let keys = (50_u8..=75).map(|i| H256([i; 32])).collect::<Vec<_>>();
        let values = (50_u8..=75).map(|i| [i; 32].to_vec()).collect::<Vec<_>>();
        let fetch_more = verify_range(root, &keys[0], &keys, &values, &proof).unwrap();
        // Our trie contains more elements to the right
        assert!(fetch_more)
    }

    #[test]
    fn verify_range_regular_case() {
        // The account ranges were taken form a hive test state, but artificially modified
        // so that the resulting trie has a wide variety of different nodes (and not only branches)
        let account_addresses: [&str; 26] = [
            "0xaa56789abcde80cde11add7d3447cd4ca93a5f2205d9874261484ae180718bd6",
            "0xaa56789abcdeda9ae19dd26a33bd10bbf825e28b3de84fc8fe1d15a21645067f",
            "0xaa56789abc39a8284ef43790e3a511b2caa50803613c5096bc782e8de08fa4c5",
            "0xaa5678931f4754834b0502de5b0342ceff21cde5bef386a83d2292f4445782c2",
            "0xaa567896492bfe767f3d18be2aab96441c449cd945770ef7ef8555acc505b2e4",
            "0xaa5f478d53bf78add6fa3708d9e061d59bfe14b21329b2a4cf1156d4f81b3d2d",
            "0xaa67c643f67b47cac9efacf6fcf0e4f4e1b273a727ded155db60eb9907939eb6",
            "0xaa04d8eaccf0b942c468074250cbcb625ec5c4688b6b5d17d2a9bdd8dd565d5a",
            "0xaa63e52cda557221b0b66bd7285b043071df4c2ab146260f4e010970f3a0cccf",
            "0xaad9aa4f67f8b24d70a0ffd757e82456d9184113106b7d9e8eb6c3e8a8df27ee",
            "0xaa3df2c3b574026812b154a99b13b626220af85cd01bb1693b1d42591054bce6",
            "0xaa79e46a5ed8a88504ac7d579b12eb346fbe4fd7e281bdd226b891f8abed4789",
            "0xbbf68e241fff876598e8e01cd529bd76416b248caf11e0552047c5f1d516aab6",
            "0xbbf68e241fff876598e8e01cd529c908cdf0d646049b5b83629a70b0117e2957",
            "0xbbf68e241fff876598e8e0180b89744abb96f7af1171ed5f47026bdf01df1874",
            "0xbbf68e241fff876598e8a4cd8e43f08be4715d903a0b1d96b3d9c4e811cbfb33",
            "0xbbf68e241fff8765182a510994e2b54d14b731fac96b9c9ef434bc1924315371",
            "0xbbf68e241fff87655379a3b66c2d8983ba0b2ca87abaf0ca44836b2a06a2b102",
            "0xbbf68e241fffcbcec8301709a7449e2e7371910778df64c89f48507390f2d129",
            "0xbbf68e241ffff228ed3aa7a29644b1915fde9ec22e0433808bf5467d914e7c7a",
            "0xbbf68e24190b881949ec9991e48dec768ccd1980896aefd0d51fd56fd5689790",
            "0xbbf68e2419de0a0cb0ff268c677aba17d39a3190fe15aec0ff7f54184955cba4",
            "0xbbf68e24cc6cbd96c1400150417dd9b30d958c58f63c36230a90a02b076f78b5",
            "0xbbf68e2490f33f1d1ba6d1521a00935630d2c81ab12fa03d4a0f4915033134f3",
            "0xc017b10a7cc3732d729fe1f71ced25e5b7bc73dc62ca61309a8c7e5ac0af2f72",
            "0xc098f06082dc467088ecedb143f9464ebb02f19dc10bd7491b03ba68d751ce45",
        ];
        let mut account_addresses = account_addresses
            .iter()
            .map(|addr| H256::from_str(addr).unwrap())
            .collect::<Vec<_>>();
        account_addresses.sort();
        let trie_values = account_addresses
            .iter()
            .map(|addr| addr.0.to_vec())
            .collect::<Vec<_>>();
        let keys = account_addresses[7..=17].to_vec();
        let values = account_addresses[7..=17]
            .iter()
            .map(|v| v.0.to_vec())
            .collect::<Vec<_>>();
        let mut trie = Trie::new_temp();
        for val in trie_values.iter() {
            trie.insert(val.clone(), val.clone()).unwrap()
        }
        let mut proof = trie.get_proof(&trie_values[7]).unwrap();
        proof.extend(trie.get_proof(&trie_values[17]).unwrap());
        let root = trie.hash().unwrap();
        let fetch_more = verify_range(root, &keys[0], &keys, &values, &proof).unwrap();
        // Our trie contains more elements to the right
        assert!(fetch_more)
    }

    #[test]
    fn test_inlined_outside_right_bound() {
        let storage_root =
            H256::from_str("7e56f63c9dd8c6b1708d26079ff5c538a729a11d3398a0c24fe679b2bd5609b5")
                .unwrap();

        let hashed_keys = vec![
            "2000000000000000000000000000000000000000000000000000000000000000",
            "cf5fef708e5b2031bce48065c29b2550399c1f21e84621770454a2286fbd4446",
        ]
        .into_iter()
        .map(|s| H256::from_str(s).unwrap())
        .collect::<Vec<_>>();
        let proof = vec![
            // root node leading to the cf5f.. branch and the 2000..0000 leaf
            hex::decode("f8518080a051786a8d3bc13523fe2a4a4de42ba891617b2aad3a2da9a0681c6efa2263f434808080808080808080a0f62210bb6894ff56c877f572781fcddb0682669e4e0ffa8e69c309ec83cc176280808080").unwrap(),
            // extension node leading to the cf5f.. branch
            hex::decode("e6841f5fef70a0c6604c42272d88b672f55ba740994b7f87602f849fc650ae5f818189336f8439").unwrap(),
            // branch with cf5f..4446 and cf5f..bd13
            hex::decode("f84d8080808080808080de9c3e5b2031bce48065c29b2550399c1f21e84621770454a2286fbd444601de9c3e0d63e372a3003b4b5ce989b0a8bd5eeaac19e6787d5b0f078fbd130180808080808080").unwrap(),
            // leaf 2000..0000
            hex::decode("e2a0300000000000000000000000000000000000000000000000000000000000000001").unwrap()
        ];
        let start_hash =
            H256::from_str("2000000000000000000000000000000000000000000000000000000000000000")
                .unwrap();
        let encoded_values: Vec<Vec<u8>> = vec![vec![1], vec![1]];

        verify_range(
            storage_root,
            &start_hash,
            &hashed_keys,
            &encoded_values,
            &proof,
        )
        .unwrap();
    }

    // Proptests for verify_range
    proptest! {

        // Successful Cases

        #[test]
        // Regular Case: Two Edge Proofs, both keys exist
        fn proptest_verify_range_regular_case(data in btree_set(vec(any::<u8>(), 32), 200), start in 1_usize..=100_usize, end in 101..200_usize) {
            // Build trie
            let mut trie = Trie::new_temp();
            for val in data.iter() {
                trie.insert(val.clone(), val.clone()).unwrap()
            }
            let root = trie.hash().unwrap();
            // Select range to prove
            let values = data.into_iter().collect::<Vec<_>>()[start..=end].to_vec();
            let keys = values.iter().map(|a| H256::from_slice(a)).collect::<Vec<_>>();
            // Generate proofs
            let mut proof = trie.get_proof(&values[0]).unwrap();
            proof.extend(trie.get_proof(values.last().unwrap()).unwrap());
            // Verify the range proof
            let fetch_more = verify_range(root, &keys[0], &keys, &values, &proof).unwrap();
            if end == 199 {
                // The last key is at the edge of the trie
                assert!(!fetch_more)
            } else {
                // Our trie contains more elements to the right
                assert!(fetch_more)
            }
        }

        #[test]
        // Two Edge Proofs, first and last keys dont exist
        fn proptest_verify_range_nonexistant_edge_keys(data in btree_set(vec(1..u8::MAX-1, 32), 200), start in 1_usize..=100_usize, end in 101..199_usize) {
            let data = data.into_iter().collect::<Vec<_>>();
            // Build trie
            let mut trie = Trie::new_temp();
            for val in data.iter() {
                trie.insert(val.clone(), val.clone()).unwrap()
            }
            let root = trie.hash().unwrap();
            // Select range to prove
            let values = data[start..=end].to_vec();
            let keys = values.iter().map(|a| H256::from_slice(a)).collect::<Vec<_>>();
            // Select the first and last keys
            // As we will be using non-existant keys we will choose values that are `just` higer/lower than
            // the first and last values in our key range
            // Skip the test entirely in the unlucky case that the values just next to the edge keys are also part of the trie
            let mut first_key = data[start].clone();
            first_key[31] -=1;
            if first_key == data[start -1] {
                // Skip test
                return Ok(());
            }
            let mut last_key = data[end].clone();
            last_key[31] +=1;
            if last_key == data[end +1] {
                // Skip test
                return Ok(());
            }
            // Generate proofs
            let mut proof = trie.get_proof(&first_key).unwrap();
            proof.extend(trie.get_proof(&last_key).unwrap());
            // Verify the range proof
            let fetch_more = verify_range(root, &H256::from_slice(&first_key), &keys, &values, &proof).unwrap();
            // Our trie contains more elements to the right
            assert!(fetch_more)
        }

        #[test]
        // Two Edge Proofs, one key doesn't exist
        fn proptest_verify_range_one_key_doesnt_exist(data in btree_set(vec(1..u8::MAX-1, 32), 200), start in 1_usize..=100_usize, end in 101..199_usize, first_key_exists in bool::ANY) {
            let data = data.into_iter().collect::<Vec<_>>();
            // Build trie
            let mut trie = Trie::new_temp();
            for val in data.iter() {
                trie.insert(val.clone(), val.clone()).unwrap()
            }
            let root = trie.hash().unwrap();
            // Select range to prove
            let values = data[start..=end].to_vec();
            let keys = values.iter().map(|a| H256::from_slice(a)).collect::<Vec<_>>();
            // Select the first and last keys
            // As we will be using non-existant keys we will choose values that are `just` higer/lower than
            // the first and last values in our key range
            // Skip the test entirely in the unlucky case that the values just next to the edge keys are also part of the trie
            let mut first_key = data[start].clone();
            let mut last_key = data[end].clone();
            if first_key_exists {
                last_key[31] +=1;
                if last_key == data[end +1] {
                    // Skip test
                    return Ok(());
                }
            } else {
                first_key[31] -=1;
                if first_key == data[start -1] {
                    // Skip test
                    return Ok(());
                }
            }
            // Generate proofs
            let mut proof = trie.get_proof(&first_key).unwrap();
            proof.extend(trie.get_proof(&last_key).unwrap());
            // Verify the range proof
            let fetch_more = verify_range(root, &H256::from_slice(&first_key), &keys, &values, &proof).unwrap();
            // Our trie contains more elements to the right
            assert!(fetch_more)
        }

        #[test]
        // Special Case: Range contains all the leafs in the trie, no proofs
        fn proptest_verify_range_full_leafset(data in btree_set(vec(any::<u8>(), 32), 100..200)) {
            // Build trie
            let mut trie = Trie::new_temp();
            for val in data.iter() {
                trie.insert(val.clone(), val.clone()).unwrap()
            }
            let root = trie.hash().unwrap();
            // Select range to prove
            let values = data.into_iter().collect::<Vec<_>>();
            let keys = values.iter().map(|a| H256::from_slice(a)).collect::<Vec<_>>();
            // The keyset contains the entire trie so we don't need edge proofs
            let proof = vec![];
            // Verify the range proof
            let fetch_more = verify_range(root, &keys[0], &keys, &values, &proof).unwrap();
            // Our range is the full leafset, there shouldn't be more values left in the trie
            assert!(!fetch_more)
        }

        #[test]
        // Special Case: No values, one edge proof (of non-existance)
        fn proptest_verify_range_no_values(mut data in btree_set(vec(any::<u8>(), 32), 100..200)) {
            // Remove the last element so we can use it as key for the proof of non-existance
            let last_element = data.pop_last().unwrap();
            // Build trie
            let mut trie = Trie::new_temp();
            for val in data.iter() {
                trie.insert(val.clone(), val.clone()).unwrap()
            }
            let root = trie.hash().unwrap();
            // Range is empty
            let values = vec![];
            let keys = vec![];
            let first_key = H256::from_slice(&last_element);
            // Generate proof (last element)
            let proof = trie.get_proof(&last_element).unwrap();
            // Verify the range proof
            let fetch_more = verify_range(root, &first_key, &keys, &values, &proof).unwrap();
            // There are no more elements to the right of the range
            assert!(!fetch_more)
        }

        #[test]
        // Special Case: One element range
        fn proptest_verify_range_one_element(data in btree_set(vec(any::<u8>(), 32), 200), start in 0_usize..200_usize) {
            // Build trie
            let mut trie = Trie::new_temp();
            for val in data.iter() {
                trie.insert(val.clone(), val.clone()).unwrap()
            }
            let root = trie.hash().unwrap();
            // Select range to prove
            let values = vec![data.iter().collect::<Vec<_>>()[start].clone()];
            let keys = values.iter().map(|a| H256::from_slice(a)).collect::<Vec<_>>();
            // Generate proofs
            let proof = trie.get_proof(&values[0]).unwrap();
            // Verify the range proof
            let fetch_more = verify_range(root, &keys[0], &keys, &values, &proof).unwrap();
            if start == 199 {
                // The last key is at the edge of the trie
                assert!(!fetch_more)
            } else {
                // Our trie contains more elements to the right
                assert!(fetch_more)
            }
        }

    // Unsuccesful Cases

        #[test]
        // Regular Case: Only one edge proof, both keys exist
        fn proptest_verify_range_regular_case_only_one_edge_proof(data in btree_set(vec(any::<u8>(), 32), 200), start in 1_usize..=100_usize, end in 101..200_usize) {
            // Build trie
            let mut trie = Trie::new_temp();
            for val in data.iter() {
                trie.insert(val.clone(), val.clone()).unwrap()
            }
            let root = trie.hash().unwrap();
            // Select range to prove
            let values = data.into_iter().collect::<Vec<_>>()[start..=end].to_vec();
            let keys = values.iter().map(|a| H256::from_slice(a)).collect::<Vec<_>>();
            // Generate proofs (only prove first key)
            let proof = trie.get_proof(&values[0]).unwrap();
            // Verify the range proof
            assert!(verify_range(root, &keys[0], &keys, &values, &proof).is_err());
        }

        #[test]
        // Regular Case: Two Edge Proofs, both keys exist, but there is a missing node in the proof
        fn proptest_verify_range_regular_case_gap_in_proof(data in btree_set(vec(any::<u8>(), 32), 200), start in 1_usize..=100_usize, end in 101..200_usize) {
            // Build trie
            let mut trie = Trie::new_temp();
            for val in data.iter() {
                trie.insert(val.clone(), val.clone()).unwrap()
            }
            let root = trie.hash().unwrap();
            // Select range to prove
            let values = data.into_iter().collect::<Vec<_>>()[start..=end].to_vec();
            let keys = values.iter().map(|a| H256::from_slice(a)).collect::<Vec<_>>();
            // Generate proofs
            let mut proof = trie.get_proof(&values[0]).unwrap();
            proof.extend(trie.get_proof(values.last().unwrap()).unwrap());
            // Remove the last node of the second proof (to make sure we don't remove a node that is also part of the first proof)
            proof.pop();
            // Verify the range proof
            assert!(verify_range(root, &keys[0], &keys, &values, &proof).is_err());
        }

        #[test]
        // Regular Case: Two Edge Proofs, both keys exist, but there is a missing node in the proof
        fn proptest_verify_range_regular_case_gap_in_middle_of_proof(data in btree_set(vec(any::<u8>(), 32), 200), start in 1_usize..=100_usize, end in 101..200_usize) {
            // Build trie
            let mut trie = Trie::new_temp();
            for val in data.iter() {
                trie.insert(val.clone(), val.clone()).unwrap()
            }
            let root = trie.hash().unwrap();
            // Select range to prove
            let values = data.into_iter().collect::<Vec<_>>()[start..=end].to_vec();
            let keys = values.iter().map(|a| H256::from_slice(a)).collect::<Vec<_>>();
            // Generate proofs
            let mut proof = trie.get_proof(&values[0]).unwrap();
            let mut second_proof = trie.get_proof(&values[0]).unwrap();
            proof.extend(trie.get_proof(values.last().unwrap()).unwrap());
            // Remove the middle node of the second proof
            let gap_idx = second_proof.len() / 2;
            let removed = second_proof.remove(gap_idx);
            // Remove the node from the first proof if it is also there
            proof.retain(|n| n != &removed);
            proof.extend(second_proof);
            // Verify the range proof
            assert!(verify_range(root, &keys[0], &keys, &values, &proof).is_err());
        }

        #[test]
        // Regular Case: No proofs both keys exist
        fn proptest_verify_range_regular_case_no_proofs(data in btree_set(vec(any::<u8>(), 32), 200), start in 1_usize..=100_usize, end in 101..200_usize) {
            // Build trie
            let mut trie = Trie::new_temp();
            for val in data.iter() {
                trie.insert(val.clone(), val.clone()).unwrap()
            }
            let root = trie.hash().unwrap();
            // Select range to prove
            let values = data.into_iter().collect::<Vec<_>>()[start..=end].to_vec();
            let keys = values.iter().map(|a| H256::from_slice(a)).collect::<Vec<_>>();
            // Dont generate proof
            let proof = vec![];
            // Verify the range proof
            assert!(verify_range(root, &keys[0], &keys, &values, &proof).is_err());
        }

        #[test]
        // Special Case: No values, one edge proof (of existance)
        fn proptest_verify_range_no_values_proof_of_existance(data in btree_set(vec(any::<u8>(), 32), 100..200)) {
            // Fetch the last element so we can use it as key for the proof
            let last_element = data.last().unwrap();
            // Build trie
            let mut trie = Trie::new_temp();
            for val in data.iter() {
                trie.insert(val.clone(), val.clone()).unwrap()
            }
            let root = trie.hash().unwrap();
            // Range is empty
            let values = vec![];
            let keys = vec![];
            let first_key = H256::from_slice(last_element);
            // Generate proof (last element)
            let proof = trie.get_proof(last_element).unwrap();
            // Verify the range proof
            assert!(verify_range(root, &first_key, &keys, &values, &proof).is_err());
        }

        #[test]
        // Special Case: One element range (but the proof is of nonexistance)
        fn proptest_verify_range_one_element_bad_proof(data in btree_set(vec(any::<u8>(), 32), 200), start in 0_usize..200_usize) {
            // Build trie
            let mut trie = Trie::new_temp();
            for val in data.iter() {
                trie.insert(val.clone(), val.clone()).unwrap()
            }
            let root = trie.hash().unwrap();
            // Select range to prove
            let values = vec![data.iter().collect::<Vec<_>>()[start].clone()];
            let keys = values.iter().map(|a| H256::from_slice(a)).collect::<Vec<_>>();
            // Remove the value to generate a proof of non-existance
            trie.remove(&values[0]).unwrap();
            // Generate proofs
            let proof = trie.get_proof(&values[0]).unwrap();
            // Verify the range proof
            assert!(verify_range(root, &keys[0], &keys, &values, &proof).is_err());
        }
    }
}
