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

    pub fn advance(&mut self, key: PathRLP) -> Result<(), TrieError> {
        debug_assert!(!self.stack.is_empty());
        let Some((root_path, root_ref)) = self.stack.pop() else {
            return Ok(());
        };

        fn first_ge(
            db: &dyn TrieDB,
            stacked_nibbles: Nibbles,
            mut nibbles: Nibbles,
            node: NodeRef,
            new_stack: &mut Vec<(Nibbles, NodeRef)>,
        ) -> Result<(), TrieError> {
            let next_node = node.get_node(db).ok().flatten().expect("must exist");
            match &next_node {
                Node::Branch(branch_node) => {
                    // Add all children to the stack (in reverse order so we process first child frist)
                    let choice = nibbles.next_choice().expect("not empty");
                    let child = &branch_node.choices[choice];
                    if child.is_valid() {
                        first_ge(
                            db,
                            stacked_nibbles.append_new(choice as u8),
                            nibbles,
                            child.clone(),
                            new_stack,
                        )?;
                    }
                    for i in choice + 1..16 {
                        let child = &branch_node.choices[i];
                        if child.is_valid() {
                            new_stack.push((stacked_nibbles.append_new(i as u8), child.clone()));
                        }
                    }
                    Ok(())
                }
                Node::Extension(extension_node) => {
                    // Update path
                    let prefix = &extension_node.prefix;
                    match nibbles.compare_prefix(prefix) {
                        Ordering::Greater => Ok(()),
                        Ordering::Less => {
                            let mut new_stacked = stacked_nibbles.clone();
                            new_stacked.extend(&extension_node.prefix);
                            new_stack.push((new_stacked, extension_node.child.clone()));
                            Ok(())
                        }
                        Ordering::Equal => {
                            nibbles = nibbles.offset(prefix.len());
                            let mut new_stacked = stacked_nibbles.clone();
                            new_stacked.extend(&extension_node.prefix);
                            first_ge(
                                db,
                                new_stacked,
                                nibbles.clone(),
                                extension_node.child.clone(),
                                new_stack,
                            )
                        }
                    }
                }
                Node::Leaf(leaf) => {
                    let prefix = &leaf.partial;
                    match nibbles.compare_prefix(prefix) {
                        Ordering::Greater => Ok(()),
                        _ => {
                            new_stack.push((stacked_nibbles.clone(), node.clone()));
                            Ok(())
                        }
                    }
                }
            }
        }

        // Fetch the last node in the stack
        let nibbles = Nibbles::from_bytes(&key);
        first_ge(
            self.db.as_ref(),
            root_path,
            nibbles,
            root_ref,
            &mut self.stack,
        )?;
        self.stack.reverse();
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
