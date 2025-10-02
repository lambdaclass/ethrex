use std::{
    collections::{BTreeMap, HashSet},
    path::Path,
};

use ethrex_config::networks::Network;
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

/// Get block numbers inside the cache directory for a given network.
pub fn get_block_numbers_in_cache_dir(dir: &Path, network: &Network) -> eyre::Result<Vec<u64>> {
    let mut block_numbers = Vec::new();
    let entries = std::fs::read_dir(dir)?;
    let prefix = format!("cache_{}_", network);

    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            let file_name = entry.file_name();
            let file_name = file_name.to_string_lossy();
            if file_name.starts_with(&prefix) && file_name.ends_with(".json") {
                let number_part = &file_name[prefix.len()..file_name.len() - 5]; // remove ".json"
                if let Ok(number) = number_part.parse::<u64>() {
                    block_numbers.push(number);
                }
            }
        }
    }

    block_numbers.sort_unstable();
    Ok(block_numbers)
}
