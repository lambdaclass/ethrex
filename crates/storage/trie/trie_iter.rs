use crate::{nibbles::Nibbles, node::Node, node_hash::NodeHash, PathRLP, Trie, ValueRLP};

pub struct TrieIterator {
    trie: Trie,
    // The stack contains the current traversed path and the next node to be traversed
    stack: Vec<(Nibbles, NodeHash)>,
}

impl TrieIterator {
    pub(crate) fn new(trie: Trie) -> Self {
        let stack = if let Some(root) = &trie.root {
            vec![(Nibbles::default(), root.clone())]
        } else {
            vec![]
        };
        Self { trie, stack }
    }
}

impl Iterator for TrieIterator {
    type Item = (Nibbles, Node);

    fn next(&mut self) -> Option<Self::Item> {
        if self.stack.is_empty() {
            return None;
        };
        // Fetch the last node in the stack
        let (mut path, next_node_hash) = self.stack.pop()?;
        let next_node = self.trie.state.get_node(next_node_hash).ok()??;
        match &next_node {
            Node::Branch(branch_node) => {
                // Add all children to the stack (in reverse order so we process first child frist)
                for (choice, child) in branch_node.choices.iter().enumerate().rev() {
                    if child.is_valid() {
                        let mut child_path = path.clone();
                        child_path.append(choice as u8);
                        self.stack.push((child_path, child.clone()))
                    }
                }
            }
            Node::Extension(extension_node) => {
                // Update path
                path.extend(&extension_node.prefix);
                // Add child to the stack
                self.stack
                    .push((path.clone(), extension_node.child.clone()));
            }
            Node::Leaf(leaf) => {
                path.extend(&leaf.partial);
            }
        }
        Some((path, next_node))
    }
}

impl TrieIterator {
    // TODO: construct path from nibbles
    pub fn content(self) -> impl Iterator<Item = (PathRLP, ValueRLP)> {
        self.filter_map(|(p, n)| match n {
            Node::Branch(branch_node) => {
                (!branch_node.value.is_empty()).then_some((p.to_bytes(), branch_node.value))
            }
            Node::Extension(_) => None,
            Node::Leaf(leaf_node) => Some((p.to_bytes(), leaf_node.value)),
        })
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use proptest::{
        collection::{btree_map, vec},
        prelude::any,
        proptest,
    };

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
}
