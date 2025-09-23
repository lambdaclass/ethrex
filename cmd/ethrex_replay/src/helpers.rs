use std::collections::{BTreeMap, HashSet};

use ethrex_rlp::decode::RLPDecode;
use ethrex_trie::{Node, NodeHash, NodeRef};

pub fn get_referenced_hashes(
    nodes: &BTreeMap<NodeHash, Vec<u8>>,
) -> eyre::Result<HashSet<NodeHash>> {
    let mut referenced_hashes: HashSet<NodeHash> = HashSet::new(); // All hashes referenced in the trie (by Branch or Ext nodes).

    for (_node_hash, node_rlp) in nodes.iter() {
        let node = Node::decode(node_rlp)?;
        match node {
            Node::Branch(node) => {
                for choice in &node.choices {
                    let NodeRef::Hash(hash) = *choice else {
                        unreachable!()
                    };

                    referenced_hashes.insert(hash);
                }
            }
            Node::Extension(node) => {
                let NodeRef::Hash(hash) = node.child else {
                    unreachable!()
                };

                referenced_hashes.insert(hash);
            }
            Node::Leaf(_node) => {}
        }
    }

    Ok(referenced_hashes)
}
