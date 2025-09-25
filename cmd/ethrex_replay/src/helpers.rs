use std::collections::{BTreeMap, HashSet};

use ethrex_rlp::decode::RLPDecode;
use ethrex_trie::{Node, NodeHash, NodeRef};

/// Given a mapping of nodes with their corresponding hash get all hashes referenced by branch and extension nodes.
pub fn get_referenced_hashes(
    nodes: &BTreeMap<NodeHash, Vec<u8>>,
) -> eyre::Result<HashSet<NodeHash>> {
    let mut referenced_hashes: HashSet<NodeHash> = HashSet::new();

    for (_node_hash, node_rlp) in nodes.iter() {
        let node = Node::decode(node_rlp)?;
        match node {
            Node::Branch(node) => {
                for choice in &node.choices {
                    if let NodeRef::Hash(hash) = *choice {
                        referenced_hashes.insert(hash);
                    } else {
                        return Err(eyre::eyre!("Branch node contains non-hash reference"));
                    }
                }
            }
            Node::Extension(node) => {
                if let NodeRef::Hash(hash) = node.child {
                    referenced_hashes.insert(hash);
                } else {
                    return Err(eyre::eyre!("Extension node contains non-hash reference"));
                }
            }
            Node::Leaf(_node) => {}
        }
    }

    Ok(referenced_hashes)
}
