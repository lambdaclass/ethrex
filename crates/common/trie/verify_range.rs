use std::{
    cmp::Ordering,
    collections::{HashMap, VecDeque},
};

use ethereum_types::H256;
use sha3::{Digest, Keccak256};

use crate::{
    nibbles::Nibbles,
    node::{Node, NodeRef},
    node_hash::NodeHash,
    ProofTrie, Trie, TrieError, ValueRLP,
};

/// Verifies that the key value range belongs to the trie with the given root given the edge proofs for the range
/// Also returns true if there is more state to be fetched (aka if there are more keys to the right of the given range)
pub fn verify_range(
    root: H256,
    first_key: &H256,
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
                "invalid proof, expected root hash {}, got  {}",
                root, hash
            )));
        }
        return Ok(false);
    }

    // Special Case: One edge proof, no range given, there are no more values in the trie
    if keys.is_empty() {
        // We need to check that the proof confirms the non-existance of the first key
        // and that there are no more elements to the right of the first key
        let (_, (left_value, _), num_right_refs) =
            process_proof_nodes(proof, root.into(), (*first_key, None))?;
        if num_right_refs > 0 || !left_value.is_empty() {
            return Err(TrieError::Verify(
                "no keys returned but more are available on the trie".to_string(),
            ));
        } else {
            return Ok(false);
        };
    }

    let last_key = keys.last().unwrap();

    // Special Case: There is only one element and the two edge keys are the same
    if keys.len() == 1 && first_key == last_key {
        // We need to check that the proof confirms the existance of the first key
        if first_key != &keys[0] {
            return Err(TrieError::Verify(
                "correct proof but invalid key".to_string(),
            ));
        }
        let (_, (left_value, _), num_right_refs) =
            process_proof_nodes(proof, root.into(), (*first_key, Some(*last_key)))?;
        if left_value != values[0] {
            return Err(TrieError::Verify(
                "correct proof but invalid data".to_string(),
            ));
        }
        return Ok(num_right_refs > 0);
    }

    // Regular Case: Two edge proofs
    if first_key >= last_key {
        return Err(TrieError::Verify("invalid edge keys".to_string()));
    }

    // Process proofs to check if they are valid.
    let (external_refs, _, num_right_refs) =
        process_proof_nodes(proof, root.into(), (*first_key, Some(*last_key)))?;

    // Reconstruct the internal nodes by inserting the elements on the range
    for (key, value) in keys.iter().zip(values.iter()) {
        trie.insert(key.0.to_vec(), value.clone())?;
    }

    // Fill up the state with the nodes from the proof
    let mut trie = ProofTrie::from(trie);
    for (partial_path, external_ref) in external_refs {
        trie.insert(partial_path, external_ref)?;
    }

    // Check that the hash is the one we expected (aka the trie was properly reconstructed from the edge proofs and the range)
    let hash = trie.hash();
    if hash != root {
        return Err(TrieError::Verify(format!(
            "invalid proof, expected root hash {}, got  {}",
            root, hash
        )));
    }
    Ok(num_right_refs > 0)
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
type ProcessProofNodesResult = (Vec<(Nibbles, NodeHash)>, (Vec<u8>, Vec<u8>), usize);
fn process_proof_nodes(
    proof: &[Vec<u8>],
    root: NodeHash,
    bounds: (H256, Option<H256>),
) -> Result<ProcessProofNodesResult, TrieError> {
    // Convert `H256` bounds into `Nibble` bounds for convenience.
    let bounds = (
        Nibbles::from_bytes(&bounds.0 .0),
        bounds.1.map(|x| Nibbles::from_bytes(&x.0)),
    );

    // Generate a map of node hashes to node data for obtaining proof nodes given their hashes.
    let proof = proof
        .iter()
        .map(|node| {
            (
                H256::from_slice(&Keccak256::new_with_prefix(node).finalize()),
                node.as_slice(),
            )
        })
        .collect::<HashMap<_, _>>();
    fn get_node(proof: &HashMap<H256, &[u8]>, hash: NodeHash) -> Result<Option<Node>, TrieError> {
        Ok(Some(Node::decode_raw(match hash {
            NodeHash::Hashed(hash) => match proof.get(&hash) {
                Some(x) => x,
                None => return Ok(None),
            },
            NodeHash::Inline(_) => hash.as_ref(),
        })?))
    }

    // Initialize the external refs container.
    let mut external_refs = Vec::new();
    let (mut left_value, mut right_value) = (Vec::new(), Vec::new());
    let mut num_right_refs = 0;

    // Iterate over the proofs tree.
    //
    // The children are processed as follows:
    //   1. Nodes that fall within bounds will be filtered out.
    //   2. Nodes for which we have the proof will push themselves into the queue.
    //   3. Nodes for which we do not have the proof are treated as external references.
    let mut process_child =
        |stack: &mut VecDeque<_>, mut partial_path: Nibbles, child| -> Result<(), TrieError> {
            let cmp_l = bounds.0.compare_prefix(&partial_path);
            let cmp_r = bounds.1.as_ref().map(|x| x.compare_prefix(&partial_path));

            if cmp_l != Ordering::Less || cmp_r.is_none_or(|x| x != Ordering::Greater) {
                let NodeRef::Hash(hash) = child else {
                    // This is unreachable because the nodes have just been decoded, therefore only
                    // having hash references.
                    unreachable!()
                };

                match get_node(&proof, hash)? {
                    Some(node) => {
                        // Append implicit leaf extension when pushing leaves.
                        if let Node::Leaf(node) = &node {
                            partial_path.extend(&node.partial);
                        }

                        stack.push_back((partial_path, node));
                    }
                    None => {
                        if cmp_l == Ordering::Equal || cmp_r.is_some_and(|x| x == Ordering::Equal) {
                            return Err(TrieError::Verify(format!("proof node missing: {hash:?}")));
                        }

                        external_refs.push((partial_path, hash));
                    }
                }

                // Increment right-reference counter.
                if cmp_l == Ordering::Less && cmp_r.is_none_or(|x| x == Ordering::Less) {
                    num_right_refs += 1;
                }
            }

            Ok(())
        };

    let mut stack = VecDeque::from_iter([(
        Nibbles::default(),
        get_node(&proof, root)?
            .ok_or(TrieError::Verify(format!("proof node missing: {root:?}")))?,
    )]);
    while let Some((mut current_path, current_node)) = stack.pop_front() {
        let value = match current_node {
            Node::Branch(node) => {
                for (index, choice) in node.choices.into_iter().enumerate() {
                    if choice.is_valid() {
                        process_child(&mut stack, current_path.append_new(index as u8), choice)?;
                    }
                }
                node.value
            }
            Node::Extension(node) => {
                current_path.extend(&node.prefix);
                process_child(&mut stack, current_path.clone(), node.child)?;
                Vec::new()
            }
            Node::Leaf(node) => node.value,
        };

        if !value.is_empty() {
            if current_path == bounds.0 {
                left_value = value.clone();
            }
            if bounds.1.as_ref().is_some_and(|x| &current_path == x) {
                right_value = value.clone();
            }
        }
    }

    Ok((external_refs, (left_value, right_value), num_right_refs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::collection::{btree_set, vec};
    use proptest::prelude::any;
    use proptest::{bool, proptest};
    use std::str::FromStr;

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
    fn verify_range_hive_case() {
        let root =
            H256::from_str("0x03a85ee8ac085ddc8a7da70ab060e76222d1ffd9d1a571858dc7912e3f9ca4b8")
                .unwrap();
        let first_key =
            H256::from_str("0x7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff")
                .unwrap();
        let keys = [
            "0x80a2c1f38f8e2721079a0de39f187adedcb81b2ab5ae718ec1b8d64e4aa6930e",
            "0x80cd4a7b601d4ba0cb09e527a246c2b5dd25b6dbf862ac4e87c6b189bfce82d7",
            "0x84c7ee50e102d0abf5750e781c1635d60346f20ab0d5e5f9830db1a592c658ff",
            "0x850230317245879e019174479e1dd7ba3538a6865720e5ed747c65b667384ce6",
            "0x8557ff8523edd726094b0b9f5bceb50753d549632df12736b13402f8d068d14f",
            "0x8678559b30b321b0f0420a4a3e8cecfde90c6e56766b78c1723062c93c1f041f",
            "0x86a73e3c668eb065ecac3402c6dc912e8eb886788ea147c770f119dcd30780c6",
            "0x86d03d0f6bed220d046a4712ec4f451583b276df1aed33f96495d22569dc3485",
            "0x873429def7829ff8227e4ef554591291907892fc8f3a1a0667dada3dc2a3eb84",
            "0x878040f46b1b4a065e6b82abd35421eb69eededc0c9598b82e3587ae47c8a651",
            "0x88a5635dabc83e4e021167be484b62cbed0ecdaa9ac282dab2cd9405e97ed602",
            "0x88bf4121c2d189670cb4d0a16e68bdf06246034fd0a59d0d46fb5cec0209831e",
            "0x89dd13890319148c4989299bd99c3483e33401380c9a626992d45ace6f3c0e7b",
            "0x8a716dfca419673e653bf29ccf6a8df031c6e1fd2f5b452bdb5a8c5517c2bcb8",
            "0x8a8266874b43f78d4097f27b2842132faed7e7e430469eec7354541eb97c3ea0",
            "0x8bd40b3239fc3d05374ff09fd7087466a793b8f87b1708d04292ff284d305d54",
            "0x8c7bfaa19ea367dec5272872114c46802724a27d9b67ea3eed85431df664664e",
            "0x8daddc3366b521af0bcd527a6ceecff8425f72932c84711d41dc54821f4de63f",
            "0x8db48990065b060b91135e54a91165d726cca865223422854dcf4d6405849d42",
            "0x8e11480987056c309d7064ebbd887f086d815353cdbaadb796891ed25f8dcf61",
            "0x8f08a5ecec7948b42d05454821b6bdcaa35fae1508f744622375e5f8d191fcc4",
            "0x903f24b3d3d45bc50c082b2e71c7339c7060f633f868db2065ef611885abe37e",
            "0x90cd283c9a7dc155092d19e7dcf148e0c98e31e81b567b1e734eed970edd397f",
            "0x910fb8b22867289cb57531ad39070ef8dbdbbe7aee941886a0e9f572b63ae9ee",
            "0x9238cfd6ecae722136f6fd8a31e6e08f08bdd0e10a73eb10d517be81657be31b",
            "0x92d0f0954f4ec68bd32163a2bd7bc69f933c7cdbfc6f3d2457e065f841666b1c",
            "0x93843d6fa1fe5709a3035573f61cc06832f0377544d16d3a0725e78a0fa0267c",
            "0x9399fea8fe2dcec489f635d05119f6dd7124ed5b015537b45f16b7464558b3ac",
            "0x93bc05fdba0a59c3416b6957c9b286e6f4063311492793903b4229b42066edc9",
            "0x943f42ad91e8019f75695946d491bb95729f0dfc5dbbb953a7239ac73f208943",
            "0x946bfb429d90f1b39bb47ada75376a8d90a5778068027d4b8b8514ac13f53eca",
            "0x957115849c50a5ff3b9d5c6c174154225ba7a4baa75a3062645fae271f7c41b0",
            "0x961508ac3c93b30ee9a5a34a862c9fe1659e570546ac6c2e35da20f6d2bb5393",
            "0x96c43ef9dce3410b78df97be69e7ccef8ed40d6e5bfe6582ea4cd7d577aa4569",
            "0x97b25febb46f44607c87a3498088c605086df207c7ddcd8ee718836a516a9153",
            "0x97f72ff641eb40ee1f1163544931635acb7550a0d44bfb9f4cc3aeae829b6d7d",
            "0x997531c8eb537c6111b00f195417fd7018a2d8a150ae8b9cc87c07927ea47b19",
            "0x99ce1680f73f2adfa8e6bed135baa3360e3d17f185521918f9341fc236526321",
            "0x99e56541f21039c9b7c63655333841a3415de0d27b79d18ade9ec7ecde7a1139",
            "0x9a3624883da81006fef2bdebf9342465f307673dda9578b50598fd154a17ca94",
            "0x9a46f4f897800366085cfa965e0b17b37008d60ba5c94098a1b1cb35eb1e86b1",
            "0x9acf0e2b4e1a33eb81b6eae71e00346c3d1a7ee2cc6d571ff28b3f100715c8cc",
            "0x9d42947ac5e61285567f65d4b400d90343dbd3192534c4c1f9d941c04f48f17c",
            "0x9d502b9aeadd66e5ccccea4a229e43cd964117384921b2688b3b319b28d12d6c",
            "0x9de451c4f48bdb56c6df198ff8e1f5e349a84a4dc11de924707718e6ac897aa6",
            "0x9feaf0bd45df0fbf327c964c243b2fbc2f0a3cb48fedfeea1ae87ac1e66bc02f",
            "0xa02abeb418f26179beafd96457bda8c690c6b1f3fbabac392d0920863edddbc6",
            "0xa02c8b02efb52fad3056fc96029467937c38c96d922250f6d2c0f77b923c85aa",
            "0xa0f5dc2d18608f8e522ffffd86828e3d792b36d924d5505c614383ddff9be2eb",
            "0xa13bfef92e05edee891599aa5e447ff2baa1708d9a6473a04ef66ab94f2a11e4",
            "0xa3abdaefbb886078dc6c5c72e4bc8d12e117dbbd588236c3fa7e0c69420eb24a",
            "0xa3d8baf7ae7c96b1020753d12154e28cc7206402037c28c49c332a08cf7c4b51",
            "0xa5541b637a896d30688a80b7affda987d9597aac7ccd9799c15999a1d7d094e2",
            "0xa62cd8ff415dce92be57e7288210062ffc9de4cd5880435829383ae6a287ba26",
            "0xa80323126353155fd019e4a55043f42f1d41aa4dfd0e7170aa7fd9cfe987ce59",
            "0xa813429b7214dd4113147914a8fc44667bf69b3822143cd108eb54df62415aed",
            "0xa87387b50b481431c6ccdb9ae99a54d4dcdd4a3eff75d7b17b4818f7bbfc21e9",
            "0xa91cf4eba735e7b7751789989e849de8a9c902bb427012321f9b384ff225910e",
            "0xa9233a729f0468c9c309c48b82934c99ba1fd18447947b3bc0621adb7a5fc643",
            "0xa95c88d7dc0f2373287c3b2407ba8e7419063833c424b06d8bb3b29181bb632e",
            "0xa9970b3744a0e46b248aaf080a001441d24175b5534ad80755661d271b976d67",
            "0xa9fd2e3a6de5a9da5badd719bd6e048acefa6d29399d8a99e19fd9626805b60b",
            "0xab7bdc41a80ae9c8fcb9426ba716d8d47e523f94ffb4b9823512d259c9eca8cd",
            "0xabd8afe9fbf5eaa36c506d7c8a2d48a35d013472f8182816be9c833be35e50da",
            "0xac7183ebb421005a660509b070d3d47fc4e134cb7379c31dc35dc03ebd02e1cf",
            "0xae88076d02b19c4d09cb13fca14303687417b632444f3e30fc4880c225867be3",
            "0xae8f01c2fafb7db3a971e088f046a501bf9f1b8a82d982f7f19154a54a61f5d3",
            "0xaeaf19d38b69be4fb41cc89e4888708daa6b9b1c3f519fa28fe9a0da70cd8697",
            "0xb07f58e45b8382bca9398ca2e2fe4435a125d725314f4ec0ae6bc74d3c303659",
            "0xb0c275c6fce5ae4aa0babc65d59e240caa0dc1d2cc54b187318d28641bd150ac",
            "0xb17ea61d092bd5d77edd9d5214e9483607689cdcc35a30f7ea49071b3be88c64",
            "0xb1b2c1c59637202bb0e0d21255e44e0df719fe990be05f213b1b813e3d8179d7",
            "0xb3a33a7f35ca5d08552516f58e9f76219716f9930a3a11ce9ae5db3e7a81445d",
            "0xb4bebe91acc1375c8dd5ff4d4c349d93399e6e6116c3c9bc55df5408d7cd447e",
            "0xb55b2b00601f5506513656c4acf716e21fdd2a583dd959bad1617534f66a2665",
            "0xb58e67c536550fdf7140c8333ca62128df469a7270b16d528bc778909e0ac9a5",
            "0xb70873babf752cfab566ed2b334bbcf176cb8ca46b8c68c3cfd68a2a3e3ec5cb",
            "0xb72d08d098e99cf3523b929f339dfaace4c24bca6fd050e421cac5b16a5ac910",
            "0xb7c2ef96238f635f86f9950700e36368efaaa70e764865dddc43ff6e96f6b346",
            "0xb888c9946a84be90a9e77539b5ac68a3c459761950a460f3e671b708bb39c41f",
            "0xb938a3796588c2aae3ee9ce2146c2a1dbf53a012fd60991a02de28e213cde980",
            "0xb9400acf38453fd206bc18f67ba04f55b807b20e4efc2157909d91d3a9f7bed2",
            "0xb990eaca858ea15fda296f3f47baa2939e8aa8bbccc12ca0c3746d9b5d5fb2ae",
            "0xb9cddc73dfdacd009e55f27bdfd1cd37eef022ded5ce686ab0ffe890e6bf311e",
            "0xba1d0afdfee510e8852f24dff964afd824bf36d458cf5f5d45f02f04b7c0b35d",
            "0xbaae09901e990935de19456ac6a6c8bc1e339d0b80ca129b8622d989b5c79120",
            "0xbb1c4d93d2a595dfdb9417c3c961111c4f11592229a5732a217df67f99c8624e",
            "0xbbdc59572cc62c338fb6e027ab00c57cdeed233c8732680a56a5747141d20c7c",
            "0xbccd85b63dba6300f84c561c5f52ce08a240564421e382e6f550ce0c12f2f632",
            "0xbea55c1dc9f4a9fb50cbedc70448a4e162792b9502bb28b936c7e0a2fd7fe41d",
            "0xbfaac98225451c56b2f9aec858cffc1eb253909615f3d9617627c793b938694f",
            "0xbfc9f55c6960006afaceee51464698180f7ff67cbb0ff54ced0ee23d5d201ca3",
            "0xbfe5dee42bddd2860a8ebbcdd09f9c52a588ba38659cf5e74b07d20f396e04d4",
            "0xc0ce77c6a355e57b89cca643e70450612c0744c9f0f8bf7dee51d6633dc850b1",
            "0xc157e0d637d64b90e2c59bc8bed2acd75696ea1ac6b633661c12ce8f2bce0d62",
            "0xc192ea2d2bb89e9bb7f17f3a282ebe8d1dd672355b5555f516b99b91799b01f6",
            "0xc22e6efe4b61b855da79c457f026cbb54c34a6d1cbd530e81ece6b2fa4e82aff",
            "0xc2406cbd93e511ef493ac81ebe2b6a3fbecd05a3ba52d82a23a88eeb9d8604f0",
            "0xc250f30c01f4b7910c2eb8cdcd697cf493f6417bb2ed61d637d625a85a400912",
            "0xc3791fc487a84f3731eb5a8129a7e26f357089971657813b48a821f5582514b3",
            "0xc3ac56e9e7f2f2c2c089e966d1b83414951586c3afeb86300531dfa350e38929",
            "0xc3c8e2dc64e67baa83b844263fe31bfe24de17bb72bfed790ab345b97b007816",
            "0xc4bab059ee8f7b36c82ada44d22129671d8f47f254ca6a48fded94a8ff591c88",
            "0xc54ffffcbaa5b566a7cf37386c4ce5a338d558612343caaa99788343d516aa5f",
            "0xc7529a2dd368422fc377941a52135f8978913eaf9740abcb79cb2da2383f4184",
            "0xc9ea69dc9e84712b1349c9b271956cc0cb9473106be92d7a937b29e78e7e970e",
            "0xcaa115f5cbd968827657398d1a5cdd850a74c66f00d4a9e84f453c764804489f",
            "0xcbf5aba52af035d7e95cb678ca89ae9c3c80ebb21e39b00db9b14f170e7310b7",
            "0xcd6b3739d4dbce17dafc156790f2a3936eb75ce95e9bba039dd76661f40ea309",
            "0xcfc0ea8ef2fdab9472408d8ec76f3a5c82dd823f94f9b82cc6b1c59ba661f000",
            "0xd1691564c6a5ab1391f0495634e749b9782de33756b6a058f4a9536c1b37bca6",
            "0xd240548736d890e158d46a2c3de5934e2b8078b8c05cb25b6f129336746415fc",
            "0xd2501ae11a14bf0c2283a24b7e77c846c00a63e71908c6a5e1caff201bad0762",
            "0xd2792a9505a6c15062bd50efd76200d6993fefcd0e284d18df985469733efed7",
            "0xd2f394b4549b085fb9b9a8b313a874ea660808a4323ab2598ee15ddd1eb7e897",
            "0xd3443fa37ee617edc09a9c930be4873c21af2c47c99601d5e20483ce6d01960a",
            "0xd352b05571154d9a2061143fe6df190a740a2d321c59eb94a54acb7f3054e489",
            "0xd37b6f5e5f0fa6a1b3fd15c9b3cf0fb595ba245ab912ad8059e672fa55f061b8",
            "0xd3f5769b7363732272d0f83260279387ef39177260e9e7ed8e92a0c2d691539c",
            "0xd52564daf6d32a6ae29470732726859261f5a7409b4858101bd233ed5cc2f662",
            "0xd54456ee399fcc104e3892257e7e6a98210d2b668c19f13670f292b6dba952af",
            "0xd546137e495e1fda28880f7d9e05fb0848c5576756f77f07b1d7d7999d08efff",
            "0xd57eafe6d4c5b91fe7114e199318ab640e55d67a1e9e3c7833253808b7dca75f",
            "0xd5c582cf097fdfd241d8e99e8972c46cdf4d18600f72da475a0a6e7c454da23f",
            "0xd5e252ab2fba10107258010f154445cf7dffc42b7d8c5476de9a7adb533d73f1",
            "0xd5e5e7be8a61bb5bfa271dfc265aa9744dea85de957b6cffff0ecb403f9697db",
            "0xd623b1845175b206c127c08046281c013e4a3316402a771f1b3b77a9831143f5",
            "0xd72e318c1cea7baf503950c9b1bd67cf7caf2f663061fcde48d379047a38d075",
            "0xd8489fd0ce5e1806b24d1a7ce0e4ba8f0856b87696456539fcbb625a9bed2ccc",
            "0xd84f7711be2f8eca69c742153230995afb483855b7c555b08da330139cdb9579",
            "0xd9f987fec216556304eba05bcdae47bb736eea5a4183eb3e2c3a5045734ae8c7",
            "0xda795b5c380772286934f9ae8496108f5cc6d1f012f0cc3a6e034c6de2dbf5f3",
            "0xda81833ff053aff243d305449775c3fb1bd7f62c4a3c95dc9fb91b85e032faee",
            "0xdbea1fd70fe1c93dfef412ce5d8565d87d6843aac044d3a015fc3db4d20a351b",
            "0xdc9ea08bdea052acab7c990edbb85551f2af3e1f1a236356ab345ac5bcc84562",
            "0xdd18bd6000c68b447e4f10a72f35dd03e3ebb9d083802ced0f0469945d9c2c2f",
            "0xdddd8c1dc5a96268a54d72a97a51769aa78db00398ada7d317431210e54c762a",
            "0xe02ec497b66cb57679eb01de1bed2ad385a3d18130441a9d337bd14897e85d39",
            "0xe09e5f27b8a7bf61805df6e5fefc24eb6894281550c2d06250adecfe1e6581d7",
            "0xe0c5acf66bda927704953fdf7fb4b99e116857121c069eca7fb9bd8acfc25434",
            "0xe1eb1e18ae510d0066d60db5c2752e8c33604d4da24c38d2bda07c0cb6ad19e4",
            "0xe333845edc60ed469a894c43ed8c06ec807dafd079b3c948077da56e18436290",
            "0xe3c2e12be28e2e36dc852e76dd32e091954f99f2a6480853cd7b9e01ec6cd889",
            "0xe3c79e424fd3a7e5bf8e0426383abd518604272fda87ecd94e1633d36f55bbb6",
            "0xe42a85d04a1d0d9fe0703020ef98fa89ecdeb241a48de2db73f2feeaa2e49b0f",
            "0xe4d9c31cc9b4a9050bbbf77cc08ac26d134253dcb6fd994275c5c3468f5b7810",
            "0xe5302e42ca6111d3515cbbb2225265077da41d997f069a6c492fa3fcb0fdf284",
            "0xe6388bfcbbd6000e90a10633c72c43b0b0fed7cf38eab785a71e6f0c5b80a26a",
            "0xe69f40f00148bf0d4dfa28b3f3f5a0297790555eca01a00e49517c6645096a6c",
            "0xe6c5edf6a0fbdcff100e5ceafb63cba9aea355ba397a93fdb42a1a67b91375f8",
            "0xe6d72f72fd2fc8af227f75ab3ab199f12dfb939bdcff5f0acdac06a90084def8",
            "0xe7c6828e1fe8c586b263a81aafc9587d313c609c6db8665a42ae1267cd9ade59",
            "0xe99460a483f3369006e3edeb356b3653699f246ec71f30568617ebc702058f59",
            "0xea810ea64a420acfa917346a4a02580a50483890cba1d8d1d158d11f1c59ed02",
            "0xec3e92967d10ac66eff64a5697258b8acf87e661962b2938a0edcd78788f360d",
            "0xed263a22f0e8be37bcc1873e589c54fe37fdde92902dc75d656997a7158a9d8c",
            "0xedd9b1f966f1dfe50234523b479a45e95a1a8ec4a057ba5bfa7b69a13768197c",
            "0xf0877d51b7712e08f2a3c96cddf50ff61b8b90f80b8b9817ea613a8a157b0c45",
            "0xf164775805f47d8970d3282188009d4d7a2da1574fe97e5d7bc9836a2eed1d5b",
            "0xf19ee923ed66b7b9264c2644aa20e5268a251b4914ca81b1dffee96ecb074cb1",
            "0xf2ea1f55938163cad3604ee1b79f88d518f59110eff5e24e9d9331626cdb7221",
            "0xf3155a4cb488da0adba397b5b72ee3ac4ad90aa8cf9b6e237119e27e1504d0d4",
            "0xf33a7b66489679fa665dbfb4e6dd4b673495f853850eedc81d5f28bd2f4bd3b5",
            "0xf7ebc5bd3f57e9bc707b37bb1d12f3b6b5a6f40bf507032fc7d9170fd03023ff",
            "0xfb2ab315988de92dcf6ba848e756676265b56e4b84778a2c955fb2b3c848c51c",
            "0xfb5a31c5cfd33dce2c80a30c5efc28e5f4025624adcc2205a2504a78c57bdd1c",
            "0xfc3d2e27841c0913d10aa11fc4af4793bf376efe3d90ce8360aa392d0ecefa24",
            "0xfdaf2549ea901a469b3e91cd1c4290fab376ef687547046751e10b7b461ff297",
            "0xfdbb8ddca8cecfe275da1ea1c36e494536f581d64ddf0c4f2e6dae9c7d891427",
        ]
        .into_iter()
        .map(|str| H256::from_str(str).unwrap())
        .collect::<Vec<_>>();
        let values = vec![
            vec![
                248, 68, 1, 128, 160, 150, 239, 215, 169, 133, 154, 95, 110, 131, 97, 147, 110,
                216, 92, 166, 94, 253, 159, 131, 51, 124, 63, 70, 31, 214, 33, 94, 46, 196, 236,
                11, 156, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3,
                192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 73, 128, 133, 23, 72, 118, 232, 0, 160, 86, 232, 31, 23, 27, 204, 85, 166,
                255, 131, 69, 230, 146, 192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98,
                47, 181, 227, 99, 180, 33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125,
                178, 220, 199, 3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93,
                133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 193, 145, 23, 203, 42, 201, 129, 99, 48, 73, 64, 24, 15, 169,
                225, 27, 113, 223, 17, 112, 98, 44, 63, 91, 47, 74, 184, 248, 188, 208, 43, 192,
                160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192, 229,
                0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 174, 67, 97, 222, 50, 11, 67, 170, 0, 66, 167, 229, 142, 122,
                153, 43, 232, 101, 216, 247, 81, 239, 164, 192, 226, 133, 141, 248, 19, 195, 98,
                136, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 81, 46, 104, 48, 103, 173, 87, 196, 172, 133, 119, 30, 61,
                132, 49, 205, 179, 229, 83, 149, 122, 233, 115, 49, 44, 214, 202, 108, 6, 104, 24,
                210, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 83, 128, 143, 192, 151, 206, 123, 201, 7, 21, 179, 75, 159, 16, 0, 0, 0, 0,
                160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146, 192, 248, 110, 91,
                72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180, 33, 160, 197, 210,
                70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192, 229, 0, 182, 83,
                202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 51, 19, 248, 84, 93, 43, 86, 187, 21, 200, 197, 225, 233, 58,
                45, 219, 179, 69, 98, 184, 4, 83, 207, 171, 8, 168, 74, 214, 239, 51, 26, 89, 160,
                197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192, 229, 0,
                182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 245, 130, 250, 124, 149, 137, 45, 132, 30, 149, 129, 47, 91,
                129, 236, 50, 109, 166, 123, 99, 89, 231, 140, 39, 77, 42, 0, 130, 136, 35, 34, 67,
                160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192, 229,
                0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 73, 128, 133, 23, 72, 118, 232, 0, 160, 86, 232, 31, 23, 27, 204, 85, 166,
                255, 131, 69, 230, 146, 192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98,
                47, 181, 227, 99, 180, 33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125,
                178, 220, 199, 3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93,
                133, 164, 112,
            ],
            vec![
                248, 73, 128, 133, 23, 72, 118, 232, 0, 160, 86, 232, 31, 23, 27, 204, 85, 166,
                255, 131, 69, 230, 146, 192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98,
                47, 181, 227, 99, 180, 33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125,
                178, 220, 199, 3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93,
                133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 73, 128, 133, 23, 72, 118, 232, 0, 160, 86, 232, 31, 23, 27, 204, 85, 166,
                255, 131, 69, 230, 146, 192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98,
                47, 181, 227, 99, 180, 33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125,
                178, 220, 199, 3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93,
                133, 164, 112,
            ],
            vec![
                248, 73, 128, 133, 23, 72, 118, 232, 0, 160, 86, 232, 31, 23, 27, 204, 85, 166,
                255, 131, 69, 230, 146, 192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98,
                47, 181, 227, 99, 180, 33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125,
                178, 220, 199, 3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93,
                133, 164, 112,
            ],
            vec![
                248, 83, 128, 143, 192, 151, 206, 123, 201, 7, 21, 179, 75, 159, 16, 0, 0, 0, 0,
                160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146, 192, 248, 110, 91,
                72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180, 33, 160, 197, 210,
                70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192, 229, 0, 182, 83,
                202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 117, 206, 254, 133, 210, 108, 61, 162, 228, 157, 112, 34,
                208, 4, 191, 68, 177, 163, 7, 111, 101, 6, 113, 25, 184, 85, 26, 242, 226, 84, 106,
                7, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 73, 128, 133, 23, 72, 118, 232, 0, 160, 86, 232, 31, 23, 27, 204, 85, 166,
                255, 131, 69, 230, 146, 192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98,
                47, 181, 227, 99, 180, 33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125,
                178, 220, 199, 3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93,
                133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 104, 62, 116, 158, 188, 21, 214, 246, 216, 174, 13, 96, 210,
                254, 20, 247, 23, 167, 44, 31, 196, 215, 197, 80, 222, 237, 181, 205, 102, 73, 196,
                67, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 73, 128, 133, 23, 72, 118, 232, 0, 160, 86, 232, 31, 23, 27, 204, 85, 166,
                255, 131, 69, 230, 146, 192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98,
                47, 181, 227, 99, 180, 33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125,
                178, 220, 199, 3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93,
                133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 183, 187, 196, 208, 68, 19, 243, 222, 205, 57, 87, 9, 236,
                84, 234, 33, 96, 74, 145, 136, 173, 70, 87, 235, 37, 77, 71, 8, 243, 202, 75, 175,
                160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192, 229,
                0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 66, 228, 0, 9, 219, 173, 224, 238, 202, 100, 251, 215, 250,
                239, 140, 104, 20, 92, 160, 85, 22, 210, 56, 137, 47, 156, 170, 39, 24, 1, 249, 85,
                160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192, 229,
                0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 73, 128, 133, 23, 72, 118, 232, 0, 160, 86, 232, 31, 23, 27, 204, 85, 166,
                255, 131, 69, 230, 146, 192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98,
                47, 181, 227, 99, 180, 33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125,
                178, 220, 199, 3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93,
                133, 164, 112,
            ],
            vec![
                248, 73, 128, 133, 23, 72, 118, 232, 0, 160, 86, 232, 31, 23, 27, 204, 85, 166,
                255, 131, 69, 230, 146, 192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98,
                47, 181, 227, 99, 180, 33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125,
                178, 220, 199, 3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93,
                133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 83, 128, 143, 192, 151, 206, 123, 201, 7, 21, 179, 75, 159, 16, 0, 0, 0, 0,
                160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146, 192, 248, 110, 91,
                72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180, 33, 160, 197, 210,
                70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192, 229, 0, 182, 83,
                202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 231, 160, 206, 166, 190, 144, 44, 101, 5, 42, 40, 108, 219,
                222, 215, 244, 78, 42, 217, 233, 188, 112, 210, 72, 182, 84, 215, 44, 170, 224, 7,
                81, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 83, 128, 143, 192, 151, 206, 123, 201, 7, 21, 179, 75, 159, 16, 0, 0, 0, 0,
                160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146, 192, 248, 110, 91,
                72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180, 33, 160, 197, 210,
                70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192, 229, 0, 182, 83,
                202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 73, 237, 236, 84, 242, 111, 186, 161, 80, 113, 170, 95, 79,
                115, 86, 175, 229, 132, 226, 52, 103, 206, 248, 246, 148, 218, 189, 19, 48, 116,
                226, 77, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3,
                192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 73, 128, 133, 23, 72, 118, 232, 0, 160, 86, 232, 31, 23, 27, 204, 85, 166,
                255, 131, 69, 230, 146, 192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98,
                47, 181, 227, 99, 180, 33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125,
                178, 220, 199, 3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93,
                133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 73, 128, 133, 23, 72, 118, 232, 0, 160, 86, 232, 31, 23, 27, 204, 85, 166,
                255, 131, 69, 230, 146, 192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98,
                47, 181, 227, 99, 180, 33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125,
                178, 220, 199, 3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93,
                133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 154, 118, 86, 161, 133, 68, 92, 130, 103, 20, 33, 20, 181,
                43, 250, 222, 255, 31, 122, 36, 153, 233, 184, 87, 30, 27, 37, 238, 166, 146, 209,
                76, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 73, 128, 133, 23, 72, 118, 232, 0, 160, 86, 232, 31, 23, 27, 204, 85, 166,
                255, 131, 69, 230, 146, 192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98,
                47, 181, 227, 99, 180, 33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125,
                178, 220, 199, 3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93,
                133, 164, 112,
            ],
            vec![
                248, 83, 128, 143, 192, 151, 206, 123, 201, 7, 21, 179, 75, 159, 16, 0, 0, 0, 0,
                160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146, 192, 248, 110, 91,
                72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180, 33, 160, 197, 210,
                70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192, 229, 0, 182, 83,
                202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 160, 170, 147, 117, 89, 208, 134, 3, 137, 72, 250, 118, 50,
                91, 21, 158, 2, 114, 12, 187, 227, 63, 78, 61, 3, 173, 78, 77, 223, 17, 25, 114,
                160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192, 229,
                0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 83, 128, 143, 192, 151, 206, 123, 201, 7, 21, 179, 75, 159, 16, 0, 0, 0, 0,
                160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146, 192, 248, 110, 91,
                72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180, 33, 160, 197, 210,
                70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192, 229, 0, 182, 83,
                202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 73, 128, 133, 23, 72, 118, 232, 0, 160, 86, 232, 31, 23, 27, 204, 85, 166,
                255, 131, 69, 230, 146, 192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98,
                47, 181, 227, 99, 180, 33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125,
                178, 220, 199, 3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93,
                133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 73, 128, 133, 23, 72, 118, 232, 0, 160, 86, 232, 31, 23, 27, 204, 85, 166,
                255, 131, 69, 230, 146, 192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98,
                47, 181, 227, 99, 180, 33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125,
                178, 220, 199, 3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93,
                133, 164, 112,
            ],
            vec![
                248, 73, 128, 133, 23, 72, 118, 232, 0, 160, 86, 232, 31, 23, 27, 204, 85, 166,
                255, 131, 69, 230, 146, 192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98,
                47, 181, 227, 99, 180, 33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125,
                178, 220, 199, 3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93,
                133, 164, 112,
            ],
            vec![
                248, 83, 128, 143, 192, 151, 206, 123, 201, 7, 21, 179, 75, 159, 16, 0, 0, 0, 0,
                160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146, 192, 248, 110, 91,
                72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180, 33, 160, 197, 210,
                70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192, 229, 0, 182, 83,
                202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 12, 221, 138, 137, 138, 96, 236, 150, 102, 240, 202, 110,
                234, 107, 156, 154, 70, 223, 60, 9, 170, 142, 9, 4, 70, 134, 143, 159, 250, 94, 31,
                24, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 73, 128, 133, 23, 72, 118, 232, 0, 160, 86, 232, 31, 23, 27, 204, 85, 166,
                255, 131, 69, 230, 146, 192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98,
                47, 181, 227, 99, 180, 33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125,
                178, 220, 199, 3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93,
                133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 102, 178, 55, 255, 204, 116, 6, 171, 209, 220, 135, 1, 50,
                228, 74, 155, 147, 24, 249, 25, 103, 150, 19, 158, 186, 127, 29, 44, 221, 82, 12,
                128, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 73, 128, 133, 23, 72, 118, 232, 0, 160, 86, 232, 31, 23, 27, 204, 85, 166,
                255, 131, 69, 230, 146, 192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98,
                47, 181, 227, 99, 180, 33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125,
                178, 220, 199, 3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93,
                133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 125, 5, 245, 246, 49, 136, 195, 102, 75, 17, 56, 91, 132,
                195, 68, 37, 126, 93, 181, 87, 51, 74, 60, 172, 244, 147, 46, 205, 183, 128, 81,
                244, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 7, 90, 155, 112, 187, 93, 95, 161, 111, 251, 215, 180, 124,
                138, 190, 95, 39, 131, 9, 32, 169, 13, 121, 171, 157, 170, 57, 129, 12, 39, 22, 8,
                160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192, 229,
                0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 65, 65, 158, 160, 177, 230, 106, 232, 214, 213, 198, 168,
                237, 154, 47, 119, 244, 182, 56, 116, 149, 32, 169, 71, 53, 56, 52, 137, 35, 116,
                71, 185, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3,
                192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 73, 128, 133, 23, 72, 118, 232, 0, 160, 86, 232, 31, 23, 27, 204, 85, 166,
                255, 131, 69, 230, 146, 192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98,
                47, 181, 227, 99, 180, 33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125,
                178, 220, 199, 3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93,
                133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 111, 237, 23, 74, 219, 160, 172, 233, 221, 56, 52, 63, 129,
                147, 143, 186, 81, 164, 39, 112, 230, 171, 184, 163, 120, 37, 0, 77, 53, 6, 159,
                157, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 158, 68, 171, 90, 165, 53, 130, 209, 4, 163, 85, 76, 156,
                243, 91, 129, 240, 5, 131, 206, 64, 172, 54, 196, 11, 192, 171, 108, 232, 56, 153,
                9, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 177, 167, 92, 94, 166, 151, 88, 237, 223, 182, 181, 40, 228,
                101, 28, 203, 203, 5, 14, 99, 220, 171, 199, 16, 139, 228, 184, 102, 49, 80, 251,
                46, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 73, 128, 133, 23, 72, 118, 232, 0, 160, 86, 232, 31, 23, 27, 204, 85, 166,
                255, 131, 69, 230, 146, 192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98,
                47, 181, 227, 99, 180, 33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125,
                178, 220, 199, 3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93,
                133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 127, 178, 215, 81, 88, 172, 81, 67, 226, 163, 2, 70, 205,
                232, 234, 237, 95, 65, 142, 243, 117, 187, 248, 104, 230, 6, 237, 202, 27, 89, 91,
                61, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 73, 128, 133, 23, 72, 118, 232, 0, 160, 86, 232, 31, 23, 27, 204, 85, 166,
                255, 131, 69, 230, 146, 192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98,
                47, 181, 227, 99, 180, 33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125,
                178, 220, 199, 3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93,
                133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 55, 179, 131, 2, 62, 49, 141, 142, 0, 72, 135, 130, 184, 6,
                16, 222, 215, 126, 251, 58, 194, 80, 74, 156, 180, 193, 84, 252, 89, 8, 233, 129,
                160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192, 229,
                0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 162, 145, 46, 169, 201, 236, 3, 16, 178, 44, 21, 43, 47, 73,
                167, 145, 22, 17, 64, 233, 111, 188, 128, 248, 172, 83, 24, 150, 176, 17, 62, 50,
                160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192, 229,
                0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 83, 128, 143, 192, 151, 206, 123, 201, 7, 21, 179, 75, 159, 16, 0, 0, 0, 0,
                160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146, 192, 248, 110, 91,
                72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180, 33, 160, 197, 210,
                70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192, 229, 0, 182, 83,
                202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 205, 205, 79, 44, 46, 172, 219, 243, 134, 208, 245, 79, 251,
                225, 249, 158, 212, 41, 124, 241, 153, 126, 131, 252, 118, 211, 226, 247, 48, 3, 3,
                48, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 83, 128, 143, 192, 151, 206, 123, 201, 7, 21, 179, 75, 159, 16, 0, 0, 0, 0,
                160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146, 192, 248, 110, 91,
                72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180, 33, 160, 197, 210,
                70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192, 229, 0, 182, 83,
                202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 73, 128, 133, 23, 72, 118, 232, 0, 160, 86, 232, 31, 23, 27, 204, 85, 166,
                255, 131, 69, 230, 146, 192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98,
                47, 181, 227, 99, 180, 33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125,
                178, 220, 199, 3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93,
                133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 68, 230, 60, 109, 176, 72, 240, 81, 70, 86, 216, 160, 43,
                141, 97, 251, 145, 112, 46, 74, 60, 188, 206, 170, 114, 135, 104, 168, 168, 126,
                70, 94, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3,
                192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 73, 128, 133, 23, 72, 118, 232, 0, 160, 86, 232, 31, 23, 27, 204, 85, 166,
                255, 131, 69, 230, 146, 192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98,
                47, 181, 227, 99, 180, 33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125,
                178, 220, 199, 3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93,
                133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 247, 75, 64, 94, 59, 8, 5, 179, 248, 244, 192, 182, 43, 7,
                90, 30, 63, 189, 136, 136, 145, 230, 27, 46, 86, 236, 171, 173, 208, 224, 9, 29,
                160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192, 229,
                0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 73, 128, 133, 23, 72, 118, 232, 0, 160, 86, 232, 31, 23, 27, 204, 85, 166,
                255, 131, 69, 230, 146, 192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98,
                47, 181, 227, 99, 180, 33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125,
                178, 220, 199, 3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93,
                133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 127, 147, 191, 38, 120, 144, 60, 172, 45, 103, 37, 174, 95,
                236, 227, 186, 161, 226, 28, 177, 232, 170, 132, 57, 72, 249, 213, 2, 27, 32, 177,
                201, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 73, 128, 133, 23, 72, 118, 232, 0, 160, 86, 232, 31, 23, 27, 204, 85, 166,
                255, 131, 69, 230, 146, 192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98,
                47, 181, 227, 99, 180, 33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125,
                178, 220, 199, 3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93,
                133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 73, 128, 133, 23, 72, 118, 232, 0, 160, 86, 232, 31, 23, 27, 204, 85, 166,
                255, 131, 69, 230, 146, 192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98,
                47, 181, 227, 99, 180, 33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125,
                178, 220, 199, 3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93,
                133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 170, 170, 254, 188, 177, 85, 157, 225, 0, 219, 232, 164, 102,
                56, 47, 207, 244, 56, 112, 240, 68, 40, 45, 162, 107, 135, 254, 225, 151, 251, 143,
                211, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 83, 128, 143, 192, 151, 206, 123, 201, 7, 21, 179, 75, 159, 16, 0, 0, 0, 0,
                160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146, 192, 248, 110, 91,
                72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180, 33, 160, 197, 210,
                70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192, 229, 0, 182, 83,
                202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 73, 128, 133, 23, 72, 118, 232, 0, 160, 86, 232, 31, 23, 27, 204, 85, 166,
                255, 131, 69, 230, 146, 192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98,
                47, 181, 227, 99, 180, 33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125,
                178, 220, 199, 3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93,
                133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 249, 14, 113, 69, 198, 189, 251, 243, 14, 205, 181, 121, 217,
                7, 84, 132, 78, 201, 237, 127, 249, 138, 155, 64, 146, 159, 41, 167, 244, 204, 205,
                143, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 97, 197, 151, 80, 222, 239, 189, 231, 91, 21, 68, 185, 125,
                193, 75, 31, 92, 247, 223, 42, 41, 186, 127, 185, 74, 115, 39, 233, 106, 41, 48,
                131, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 90, 173, 20, 110, 237, 43, 97, 235, 217, 235, 144, 148, 186,
                190, 234, 120, 67, 143, 86, 177, 166, 102, 53, 102, 204, 72, 177, 146, 35, 165,
                182, 67, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3,
                192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 73, 128, 133, 23, 72, 118, 232, 0, 160, 86, 232, 31, 23, 27, 204, 85, 166,
                255, 131, 69, 230, 146, 192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98,
                47, 181, 227, 99, 180, 33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125,
                178, 220, 199, 3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93,
                133, 164, 112,
            ],
            vec![
                248, 83, 128, 143, 192, 151, 206, 123, 201, 7, 21, 179, 75, 159, 16, 0, 0, 0, 0,
                160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146, 192, 248, 110, 91,
                72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180, 33, 160, 197, 210,
                70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192, 229, 0, 182, 83,
                202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 101, 22, 206, 60, 97, 77, 172, 254, 197, 43, 186, 39, 20,
                146, 95, 81, 65, 226, 202, 252, 246, 64, 31, 56, 219, 118, 117, 159, 105, 59, 59,
                232, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 84, 156, 58, 201, 115, 1, 199, 64, 101, 10, 2, 29, 46, 33,
                226, 146, 17, 219, 115, 197, 204, 88, 157, 227, 84, 2, 243, 1, 148, 149, 210, 249,
                160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192, 229,
                0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 73, 128, 133, 23, 72, 118, 232, 0, 160, 86, 232, 31, 23, 27, 204, 85, 166,
                255, 131, 69, 230, 146, 192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98,
                47, 181, 227, 99, 180, 33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125,
                178, 220, 199, 3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93,
                133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 17, 127, 250, 83, 162, 155, 76, 12, 121, 143, 92, 218, 151,
                142, 11, 2, 5, 26, 140, 42, 6, 49, 186, 229, 155, 154, 171, 115, 46, 93, 58, 76,
                160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192, 229,
                0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 111, 159, 57, 143, 96, 100, 17, 64, 178, 17, 4, 117, 133,
                178, 113, 124, 17, 66, 194, 8, 46, 232, 4, 183, 24, 62, 76, 1, 188, 104, 36, 19,
                160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192, 229,
                0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 83, 128, 143, 192, 151, 206, 123, 201, 7, 21, 179, 75, 159, 16, 0, 0, 0, 0,
                160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146, 192, 248, 110, 91,
                72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180, 33, 160, 197, 210,
                70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192, 229, 0, 182, 83,
                202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 73, 128, 133, 23, 72, 118, 232, 0, 160, 86, 232, 31, 23, 27, 204, 85, 166,
                255, 131, 69, 230, 146, 192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98,
                47, 181, 227, 99, 180, 33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125,
                178, 220, 199, 3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93,
                133, 164, 112,
            ],
            vec![
                248, 73, 128, 133, 23, 72, 118, 232, 0, 160, 86, 232, 31, 23, 27, 204, 85, 166,
                255, 131, 69, 230, 146, 192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98,
                47, 181, 227, 99, 180, 33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125,
                178, 220, 199, 3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93,
                133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 228, 17, 143, 12, 150, 239, 105, 70, 242, 202, 199, 168, 202,
                141, 77, 64, 59, 105, 103, 237, 209, 170, 57, 128, 202, 175, 92, 181, 54, 131, 219,
                232, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 83, 128, 143, 192, 151, 206, 123, 201, 7, 21, 179, 75, 159, 16, 0, 0, 0, 0,
                160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146, 192, 248, 110, 91,
                72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180, 33, 160, 197, 210,
                70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192, 229, 0, 182, 83,
                202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 154, 64, 122, 234, 245, 86, 119, 59, 101, 139, 30, 2, 228,
                242, 201, 77, 1, 85, 1, 151, 77, 124, 208, 31, 66, 206, 205, 206, 155, 215, 144,
                27, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 124, 78, 190, 8, 179, 57, 162, 194, 119, 30, 213, 233, 157,
                237, 106, 183, 61, 59, 115, 231, 72, 53, 7, 206, 161, 71, 161, 102, 5, 138, 99, 15,
                160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192, 229,
                0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 73, 128, 133, 23, 72, 118, 232, 0, 160, 86, 232, 31, 23, 27, 204, 85, 166,
                255, 131, 69, 230, 146, 192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98,
                47, 181, 227, 99, 180, 33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125,
                178, 220, 199, 3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93,
                133, 164, 112,
            ],
            vec![
                248, 73, 128, 133, 23, 72, 118, 232, 0, 160, 86, 232, 31, 23, 27, 204, 85, 166,
                255, 131, 69, 230, 146, 192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98,
                47, 181, 227, 99, 180, 33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125,
                178, 220, 199, 3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93,
                133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 128, 1, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 154, 192, 157, 150, 178, 2, 101, 37, 224, 210, 14, 145, 203,
                69, 202, 57, 195, 22, 115, 74, 74, 145, 55, 52, 25, 86, 191, 15, 81, 236, 139, 170,
                160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192, 229,
                0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 83, 79, 95, 213, 129, 17, 59, 58, 184, 193, 1, 227, 27, 25,
                166, 20, 244, 155, 66, 186, 202, 143, 163, 89, 86, 217, 195, 110, 231, 51, 9, 105,
                160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192, 229,
                0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 86, 232, 31, 23, 27, 204, 85, 166, 255, 131, 69, 230, 146,
                192, 248, 110, 91, 72, 224, 27, 153, 108, 173, 192, 1, 98, 47, 181, 227, 99, 180,
                33, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3, 192,
                229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
            vec![
                248, 68, 1, 128, 160, 6, 195, 234, 204, 226, 209, 156, 156, 158, 174, 254, 14, 204,
                147, 212, 112, 247, 125, 143, 243, 238, 67, 77, 26, 115, 96, 143, 44, 165, 154,
                145, 29, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199, 3,
                192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
        ];
        let proof = vec![
            vec![
                249, 2, 17, 160, 96, 119, 26, 127, 144, 102, 30, 40, 37, 242, 34, 134, 59, 168,
                132, 233, 42, 145, 93, 172, 178, 187, 194, 234, 151, 57, 202, 216, 246, 231, 236,
                84, 160, 219, 194, 219, 90, 34, 177, 71, 145, 99, 254, 15, 22, 69, 63, 15, 87, 2,
                240, 247, 42, 235, 214, 48, 71, 255, 115, 21, 207, 149, 60, 186, 91, 160, 206, 58,
                30, 242, 221, 18, 202, 88, 232, 130, 107, 157, 107, 58, 204, 230, 111, 204, 52,
                109, 118, 108, 59, 143, 35, 33, 179, 239, 151, 99, 114, 165, 160, 251, 251, 150,
                195, 199, 66, 92, 240, 170, 164, 68, 211, 201, 56, 236, 35, 189, 241, 127, 48, 161,
                125, 113, 168, 162, 141, 196, 136, 165, 157, 131, 138, 160, 41, 180, 117, 235, 213,
                27, 199, 34, 134, 176, 74, 210, 137, 159, 216, 146, 13, 240, 72, 34, 67, 143, 224,
                113, 125, 234, 242, 207, 189, 243, 230, 104, 160, 224, 147, 120, 208, 75, 100, 145,
                213, 89, 192, 30, 91, 243, 145, 66, 156, 50, 93, 0, 155, 126, 94, 228, 154, 152,
                140, 42, 55, 167, 103, 62, 1, 160, 160, 90, 16, 237, 192, 196, 7, 62, 104, 88, 162,
                12, 134, 127, 217, 249, 27, 36, 173, 236, 154, 161, 14, 207, 15, 23, 205, 138, 69,
                140, 138, 113, 160, 69, 244, 213, 151, 111, 234, 109, 192, 53, 106, 62, 228, 45,
                242, 1, 240, 51, 151, 45, 43, 141, 98, 107, 124, 15, 83, 195, 14, 25, 217, 177,
                184, 160, 220, 213, 46, 94, 111, 99, 252, 194, 227, 240, 150, 163, 201, 91, 201,
                241, 46, 203, 224, 190, 58, 184, 116, 89, 179, 192, 240, 141, 182, 9, 61, 251, 160,
                167, 180, 250, 107, 247, 40, 209, 253, 76, 27, 63, 119, 34, 90, 116, 66, 130, 31,
                76, 1, 37, 33, 216, 138, 9, 149, 35, 212, 223, 229, 34, 144, 160, 11, 199, 88, 30,
                13, 166, 89, 91, 225, 84, 117, 122, 97, 57, 160, 144, 147, 185, 92, 31, 115, 183,
                141, 95, 111, 110, 55, 213, 208, 239, 127, 112, 160, 9, 49, 175, 55, 211, 175, 104,
                91, 177, 107, 212, 64, 164, 92, 67, 242, 75, 115, 97, 213, 40, 226, 249, 75, 243,
                213, 78, 87, 211, 71, 212, 133, 160, 25, 254, 139, 180, 249, 66, 19, 234, 203, 171,
                154, 137, 178, 38, 50, 224, 81, 13, 215, 250, 238, 162, 154, 246, 248, 226, 104,
                66, 97, 41, 20, 127, 160, 91, 196, 179, 82, 169, 47, 181, 250, 39, 237, 100, 187,
                94, 25, 164, 245, 239, 36, 185, 73, 208, 170, 185, 245, 181, 84, 120, 250, 248,
                181, 157, 213, 160, 142, 70, 202, 182, 185, 103, 186, 4, 52, 135, 61, 221, 239,
                105, 56, 197, 149, 122, 179, 212, 30, 129, 83, 198, 249, 70, 158, 20, 203, 96, 20,
                130, 160, 67, 67, 132, 171, 15, 141, 255, 0, 127, 230, 114, 19, 63, 227, 121, 115,
                251, 130, 129, 217, 4, 55, 45, 120, 55, 16, 100, 240, 51, 107, 84, 123, 128,
            ],
            vec![
                249, 1, 113, 128, 160, 180, 188, 150, 107, 204, 208, 48, 229, 134, 124, 108, 90,
                236, 124, 148, 108, 15, 143, 83, 239, 251, 42, 27, 223, 103, 244, 46, 150, 75, 126,
                186, 125, 160, 239, 79, 253, 254, 28, 191, 67, 220, 172, 18, 201, 19, 81, 155, 114,
                145, 250, 160, 204, 81, 68, 142, 225, 15, 16, 206, 188, 187, 79, 225, 68, 146, 160,
                222, 56, 48, 17, 112, 109, 214, 3, 191, 99, 117, 141, 132, 134, 131, 178, 114, 12,
                71, 222, 62, 216, 239, 43, 169, 210, 144, 188, 220, 103, 101, 1, 160, 180, 177,
                220, 102, 32, 77, 251, 239, 148, 58, 239, 95, 126, 106, 172, 207, 134, 113, 149, 2,
                246, 125, 136, 147, 213, 37, 172, 148, 125, 73, 187, 229, 160, 124, 202, 189, 132,
                130, 4, 197, 87, 155, 177, 115, 221, 14, 219, 189, 182, 7, 44, 121, 14, 221, 24,
                98, 222, 227, 195, 133, 45, 194, 242, 196, 236, 160, 112, 157, 170, 8, 200, 135,
                85, 169, 11, 34, 228, 213, 94, 68, 252, 138, 51, 142, 178, 30, 23, 55, 240, 50, 42,
                220, 44, 169, 81, 38, 25, 32, 128, 128, 160, 178, 29, 204, 124, 120, 226, 179, 249,
                215, 162, 211, 118, 144, 211, 35, 143, 75, 207, 51, 176, 196, 166, 191, 23, 251,
                224, 157, 205, 87, 84, 160, 49, 160, 254, 214, 234, 164, 168, 253, 229, 234, 132,
                182, 216, 168, 181, 197, 79, 209, 72, 44, 133, 201, 166, 52, 171, 211, 4, 234, 108,
                7, 45, 125, 207, 139, 128, 160, 122, 100, 97, 121, 115, 225, 147, 32, 28, 23, 65,
                210, 0, 120, 98, 81, 3, 96, 231, 113, 136, 198, 51, 46, 247, 6, 5, 78, 204, 165,
                60, 108, 160, 152, 141, 82, 32, 12, 163, 16, 158, 61, 134, 180, 120, 72, 104, 160,
                28, 174, 224, 182, 29, 78, 66, 193, 60, 159, 29, 207, 155, 151, 233, 47, 221, 160,
                74, 63, 133, 70, 5, 79, 234, 51, 255, 7, 194, 236, 135, 198, 246, 65, 188, 126, 52,
                96, 96, 196, 10, 115, 20, 8, 154, 17, 107, 127, 82, 59, 128, 128,
            ],
            vec![
                249, 2, 17, 160, 96, 119, 26, 127, 144, 102, 30, 40, 37, 242, 34, 134, 59, 168,
                132, 233, 42, 145, 93, 172, 178, 187, 194, 234, 151, 57, 202, 216, 246, 231, 236,
                84, 160, 219, 194, 219, 90, 34, 177, 71, 145, 99, 254, 15, 22, 69, 63, 15, 87, 2,
                240, 247, 42, 235, 214, 48, 71, 255, 115, 21, 207, 149, 60, 186, 91, 160, 206, 58,
                30, 242, 221, 18, 202, 88, 232, 130, 107, 157, 107, 58, 204, 230, 111, 204, 52,
                109, 118, 108, 59, 143, 35, 33, 179, 239, 151, 99, 114, 165, 160, 251, 251, 150,
                195, 199, 66, 92, 240, 170, 164, 68, 211, 201, 56, 236, 35, 189, 241, 127, 48, 161,
                125, 113, 168, 162, 141, 196, 136, 165, 157, 131, 138, 160, 41, 180, 117, 235, 213,
                27, 199, 34, 134, 176, 74, 210, 137, 159, 216, 146, 13, 240, 72, 34, 67, 143, 224,
                113, 125, 234, 242, 207, 189, 243, 230, 104, 160, 224, 147, 120, 208, 75, 100, 145,
                213, 89, 192, 30, 91, 243, 145, 66, 156, 50, 93, 0, 155, 126, 94, 228, 154, 152,
                140, 42, 55, 167, 103, 62, 1, 160, 160, 90, 16, 237, 192, 196, 7, 62, 104, 88, 162,
                12, 134, 127, 217, 249, 27, 36, 173, 236, 154, 161, 14, 207, 15, 23, 205, 138, 69,
                140, 138, 113, 160, 69, 244, 213, 151, 111, 234, 109, 192, 53, 106, 62, 228, 45,
                242, 1, 240, 51, 151, 45, 43, 141, 98, 107, 124, 15, 83, 195, 14, 25, 217, 177,
                184, 160, 220, 213, 46, 94, 111, 99, 252, 194, 227, 240, 150, 163, 201, 91, 201,
                241, 46, 203, 224, 190, 58, 184, 116, 89, 179, 192, 240, 141, 182, 9, 61, 251, 160,
                167, 180, 250, 107, 247, 40, 209, 253, 76, 27, 63, 119, 34, 90, 116, 66, 130, 31,
                76, 1, 37, 33, 216, 138, 9, 149, 35, 212, 223, 229, 34, 144, 160, 11, 199, 88, 30,
                13, 166, 89, 91, 225, 84, 117, 122, 97, 57, 160, 144, 147, 185, 92, 31, 115, 183,
                141, 95, 111, 110, 55, 213, 208, 239, 127, 112, 160, 9, 49, 175, 55, 211, 175, 104,
                91, 177, 107, 212, 64, 164, 92, 67, 242, 75, 115, 97, 213, 40, 226, 249, 75, 243,
                213, 78, 87, 211, 71, 212, 133, 160, 25, 254, 139, 180, 249, 66, 19, 234, 203, 171,
                154, 137, 178, 38, 50, 224, 81, 13, 215, 250, 238, 162, 154, 246, 248, 226, 104,
                66, 97, 41, 20, 127, 160, 91, 196, 179, 82, 169, 47, 181, 250, 39, 237, 100, 187,
                94, 25, 164, 245, 239, 36, 185, 73, 208, 170, 185, 245, 181, 84, 120, 250, 248,
                181, 157, 213, 160, 142, 70, 202, 182, 185, 103, 186, 4, 52, 135, 61, 221, 239,
                105, 56, 197, 149, 122, 179, 212, 30, 129, 83, 198, 249, 70, 158, 20, 203, 96, 20,
                130, 160, 67, 67, 132, 171, 15, 141, 255, 0, 127, 230, 114, 19, 63, 227, 121, 115,
                251, 130, 129, 217, 4, 55, 45, 120, 55, 16, 100, 240, 51, 107, 84, 123, 128,
            ],
            vec![
                249, 1, 17, 160, 180, 1, 55, 39, 180, 25, 195, 103, 247, 247, 172, 170, 177, 105,
                74, 167, 35, 146, 123, 97, 97, 134, 80, 67, 211, 244, 37, 225, 186, 172, 222, 87,
                160, 129, 148, 213, 118, 228, 106, 187, 36, 211, 117, 245, 240, 151, 138, 76, 252,
                88, 196, 46, 48, 16, 142, 12, 67, 227, 145, 100, 190, 109, 145, 224, 6, 160, 235,
                105, 210, 193, 127, 199, 100, 53, 37, 58, 65, 101, 125, 177, 243, 59, 83, 27, 112,
                205, 240, 109, 65, 250, 15, 212, 69, 190, 100, 95, 87, 60, 160, 60, 150, 210, 200,
                0, 208, 7, 166, 40, 62, 61, 120, 207, 100, 188, 55, 223, 27, 36, 136, 215, 188,
                100, 100, 25, 149, 126, 92, 216, 39, 40, 50, 128, 128, 128, 160, 95, 164, 192, 70,
                75, 66, 125, 173, 16, 56, 225, 65, 30, 116, 9, 219, 47, 47, 70, 212, 248, 215, 53,
                7, 113, 228, 90, 124, 45, 251, 183, 106, 128, 128, 128, 160, 219, 207, 190, 83, 57,
                222, 229, 58, 211, 107, 73, 104, 128, 186, 127, 61, 15, 242, 226, 207, 250, 11, 23,
                246, 93, 212, 203, 80, 216, 230, 250, 228, 160, 70, 209, 217, 155, 239, 131, 205,
                90, 35, 233, 141, 113, 146, 19, 111, 163, 180, 244, 11, 98, 45, 237, 221, 254, 136,
                67, 209, 247, 33, 157, 11, 10, 160, 200, 146, 17, 127, 6, 200, 149, 175, 101, 37,
                27, 111, 71, 48, 16, 157, 30, 218, 94, 77, 103, 75, 42, 130, 191, 221, 179, 2, 37,
                201, 84, 201, 128, 128, 128,
            ],
            vec![
                248, 81, 128, 128, 128, 128, 128, 128, 128, 128, 128, 128, 160, 177, 26, 199, 54,
                227, 168, 219, 27, 24, 81, 151, 199, 102, 107, 79, 147, 176, 252, 128, 79, 133, 7,
                122, 44, 34, 189, 49, 97, 52, 177, 237, 101, 160, 37, 253, 40, 126, 101, 234, 30,
                100, 163, 124, 252, 236, 251, 230, 139, 155, 135, 218, 253, 64, 210, 128, 80, 59,
                253, 90, 183, 244, 75, 27, 178, 18, 128, 128, 128, 128, 128,
            ],
            vec![
                248, 104, 159, 59, 141, 220, 168, 206, 207, 226, 117, 218, 30, 161, 195, 110, 73,
                69, 54, 245, 129, 214, 77, 223, 12, 79, 46, 109, 174, 156, 125, 137, 20, 39, 184,
                70, 248, 68, 1, 128, 160, 6, 195, 234, 204, 226, 209, 156, 156, 158, 174, 254, 14,
                204, 147, 212, 112, 247, 125, 143, 243, 238, 67, 77, 26, 115, 96, 143, 44, 165,
                154, 145, 29, 160, 197, 210, 70, 1, 134, 247, 35, 60, 146, 126, 125, 178, 220, 199,
                3, 192, 229, 0, 182, 83, 202, 130, 39, 59, 123, 250, 216, 4, 93, 133, 164, 112,
            ],
        ];

        assert!(verify_range(root, &first_key, &keys, &values, &proof).is_ok())
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
            trie.remove(values[0].clone()).unwrap();
            // Generate proofs
            let proof = trie.get_proof(&values[0]).unwrap();
            // Verify the range proof
            assert!(verify_range(root, &keys[0], &keys, &values, &proof).is_err());
        }
    }
}
