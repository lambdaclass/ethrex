use ethrex_trie::Trie;
use proptest::{
    collection::{btree_map, vec},
    prelude::any,
    proptest,
};

#[test]
fn trie_iter_content_advanced() {
    let expected_content = vec![
        (vec![0, 9], vec![3, 4]),
        (vec![1, 2], vec![5, 6]),
        (vec![2, 7], vec![7, 8]),
    ];

    let mut trie = Trie::new_temp();
    for (path, value) in expected_content.clone() {
        trie.insert(path, value).unwrap()
    }
    let mut iter = trie.into_iter();
    iter.advance(vec![1, 2]).unwrap();
    let content = iter.content().collect::<Vec<_>>();
    assert_eq!(content, expected_content[1..]);

    let mut trie = Trie::new_temp();
    for (path, value) in expected_content.clone() {
        trie.insert(path, value).unwrap()
    }
    let mut iter = trie.into_iter();
    iter.advance(vec![1, 3]).unwrap();
    let content = iter.content().collect::<Vec<_>>();
    assert_eq!(content, expected_content[2..]);
}

#[test]
fn trie_iter_content() {
    let expected_content = vec![
        (vec![0, 9], vec![3, 4]),
        (vec![1, 2], vec![5, 6]),
        (vec![2, 7], vec![7, 8]),
    ];
    let mut trie = Trie::new_temp();
    for (path, value) in expected_content.clone() {
        trie.insert(path, value).unwrap()
    }
    let content = trie.into_iter().content().collect::<Vec<_>>();
    assert_eq!(content, expected_content);
}

proptest! {

    #[test]
    fn proptest_trie_iter_content(data in btree_map(vec(any::<u8>(), 5..100), vec(any::<u8>(), 5..100), 5..100)) {
        let expected_content = data.clone().into_iter().collect::<Vec<_>>();
        let mut trie = Trie::new_temp();
        for (path, value) in data.into_iter() {
            trie.insert(path, value).unwrap()
        }
        let content = trie.into_iter().content().collect::<Vec<_>>();
        assert_eq!(content, expected_content);
    }
}
