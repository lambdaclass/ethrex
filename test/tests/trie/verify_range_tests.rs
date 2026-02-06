#![expect(clippy::unnecessary_to_owned, clippy::useless_vec)]
use ethereum_types::H256;
use ethrex_trie::{Trie, verify_range};
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
    account_addresses.sort_unstable();
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
        H256::from_str("7e56f63c9dd8c6b1708d26079ff5c538a729a11d3398a0c24fe679b2bd5609b5").unwrap();

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
        H256::from_str("2000000000000000000000000000000000000000000000000000000000000000").unwrap();
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
