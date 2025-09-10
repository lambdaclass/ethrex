use std::cmp::Ordering;

use crate::{
    PathRLP, Trie, TrieDB, TrieError, ValueRLP,
    nibbles::Nibbles,
    node::{Node, NodeRef},
};

pub struct TrieIterator {
    db: Box<dyn TrieDB>,
    // The stack contains the current traversed path and the next node to be traversed.
    // It proactively stacks all children of a branch after consuming it to reduce accesses to the database.
    // The stack is really used as a convoluted FIFO, so elements are added in the reverse order they will be popped.
    // This avoids extra copies caused by taking elements from the front.
    stack: Vec<(Nibbles, NodeRef)>,
}

impl TrieIterator {
    pub(crate) fn new(trie: Trie) -> Self {
        let mut stack = Vec::new();
        if trie.root.is_valid() {
            stack.push((Nibbles::default(), trie.root));
        }
        Self { db: trie.db, stack }
    }

    /// Position the iterator to the first leaf >= key.
    /// Manually push the correct nodes to the stack so iteration doesn't rewind back
    /// to left children of a traversed branch node.
    pub fn advance(&mut self, key: PathRLP) -> Result<(), TrieError> {
        let Some((root_path, root_ref)) = self.stack.pop() else {
            return Ok(());
        };

        let mut node_ref = root_ref;
        let mut prefix_nibbles = root_path;
        let mut target_nibbles = Nibbles::from_bytes(&key);
        let db = self.db.as_ref();

        loop {
            if target_nibbles.is_empty() || target_nibbles.at(0) == 16 || !node_ref.is_valid() {
                break;
            }
            let Some(next_node) = node_ref.get_node(db).ok().flatten() else {
                self.stack.clear();
                break;
            };
            match &next_node {
                Node::Branch(branch_node) => {
                    // We need to skip the branch node itself to avoid iterating over left
                    // uncles of our key. Instead, we add explicitly the children that can't
                    // lead to our leaf but are greater lexicographically, and then explore,
                    // if valid, the child that leads to a prefix of `key`.
                    let Some(choice) = target_nibbles.next_choice() else {
                        self.stack.clear();
                        break;
                    };
                    let child = &branch_node.choices[choice];
                    // Because we can't add the branch, we need to add the valid greater children.
                    for i in (choice + 1..16).rev() {
                        let child = &branch_node.choices[i];
                        if child.is_valid() {
                            self.stack
                                .push((prefix_nibbles.append_new(i as u8), child.clone()));
                        }
                    }
                    // If a prefix of `key` exists under this branch, we recur to the child node, skipping
                    // the branch itself to avoid iterating lesser keys.
                    node_ref = child.clone();
                    prefix_nibbles = prefix_nibbles.append_new(choice as u8);
                }
                Node::Extension(extension_node) => {
                    // Update path
                    let prefix = &extension_node.prefix;
                    match target_nibbles.compare_prefix(prefix) {
                        Ordering::Greater => {
                            // Path is lesser than `key`, ignore it and finish iteration.
                            break;
                        }
                        Ordering::Less => {
                            // Path is greater than `key`, we need to iterate it.
                            self.stack.push((prefix_nibbles.clone(), node_ref));
                            break;
                        }
                        Ordering::Equal => {
                            // This is a prefix of `key`, we'll need to check the child,
                            // but not iterate the node itself again.
                            target_nibbles = target_nibbles.offset(prefix.len());
                            prefix_nibbles.extend(&extension_node.prefix);
                            node_ref = extension_node.child.clone();
                        }
                    }
                }
                Node::Leaf(leaf) => {
                    let prefix = &leaf.partial;
                    match target_nibbles.compare_prefix(prefix) {
                        Ordering::Greater => {} // Leaf is less than `key`, ignore it.
                        _ => {
                            // Leaf is greater than or equal to `key`, so it's in range for
                            // iteration.
                            self.stack.push((prefix_nibbles.clone(), node_ref.clone()));
                        }
                    }
                    // We reached the bottom, so every other path has to be covered.
                    break;
                }
            }
        }

        Ok(())
    }
}

impl Iterator for TrieIterator {
    type Item = (Nibbles, Node);

    fn next(&mut self) -> Option<Self::Item> {
        if self.stack.is_empty() {
            return None;
        };
        // Fetch the last node in the stack
        let (mut path, next_node_ref) = self.stack.pop()?;
        let next_node = next_node_ref.get_node(self.db.as_ref()).ok().flatten()?;
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
}
