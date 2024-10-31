use crate::{node::Node, node_hash::NodeHash, PathRLP, Trie, ValueRLP};

pub struct TrieIterator {
    trie: Trie,
    stack: Vec<NodeHash>,
}

impl TrieIterator {
    pub(crate) fn new(trie: Trie) -> Self {
        let stack = if let Some(root) = &trie.root {
            vec![root.clone()]
        } else {
            vec![]
        };
        Self { trie, stack }
    }
}

impl Iterator for TrieIterator {
    type Item = Node;

    fn next(&mut self) -> Option<Self::Item> {
        if self.stack.is_empty() {
            return None;
        };
        // Fetch the last node in the stack
        let next_node_hash = self.stack.pop()?;
        let next_node = self.trie.state.get_node(next_node_hash).ok()??;
        match &next_node {
            Node::Branch(branch_node) => {
                // Add all children to the stack (in reverse order so we process first child frist)
                for child in branch_node.choices.iter().rev() {
                    if child.is_valid() {
                        self.stack.push(child.clone())
                    }
                }
            }
            Node::Extension(extension_node) => {
                // Add child to the stack
                self.stack.push(extension_node.child.clone());
            }
            Node::Leaf(_) => {}
        }
        Some(next_node)
    }
}

impl TrieIterator {
    pub fn content(self) -> impl Iterator<Item = (PathRLP, ValueRLP)> {
        self.filter_map(|n| match n {
            Node::Branch(branch_node) => {
                (!branch_node.path.is_empty()).then_some((branch_node.path, branch_node.value))
            }
            Node::Extension(_) => None,
            Node::Leaf(leaf_node) => Some((leaf_node.path, leaf_node.value)),
        })
    }
}
