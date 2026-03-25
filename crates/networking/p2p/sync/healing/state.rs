//! This module contains the logic for state healing
//! State healing begins after we already downloaded the whole state trie and rebuilt it locally
//! It's purpose is to fix inconsistencies with the canonical state trie by downloading all the trie nodes that we don't have starting from the root node
//! The reason for these inconsistencies is that state download can spawn across multiple sync cycles each with a different pivot,
//! meaning that the resulting trie is made up of fragments of different state tries and is not consistent with any block's state trie
//! For each node downloaded, will add it to the trie's state and check if we have its children stored, if we don't we will download each missing child
//! Note that during this process the state trie for the pivot block and any prior pivot block will not be in a consistent state
//! This process will stop once it has fixed all trie inconsistencies or when the pivot becomes stale, in which case it can be resumed on the next cycle
//! All healed accounts will also have their bytecodes and storages healed by the corresponding processes

use std::{
    cmp::min,
    collections::{BTreeMap, HashMap},
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use crate::snap::mpt_stubs::{Nibbles, Node, TrieDB, TrieError};
use ethrex_common::{H256, constants::EMPTY_KECCACK_HASH, types::AccountState};
use ethrex_crypto::NativeCrypto;
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_storage::Store;
use tracing::{debug, trace};

use crate::{
    metrics::{CurrentStepValue, METRICS},
    peer_handler::{PeerHandler, RequestMetadata},
    rlpx::p2p::SUPPORTED_SNAP_CAPABILITIES,
    snap::{
        SnapError,
        constants::{NODE_BATCH_SIZE, SHOW_PROGRESS_INTERVAL_DURATION},
        request_state_trienodes,
    },
    sync::{AccountStorageRoots, SyncError, code_collector::CodeHashCollector},
    utils::current_unix_time,
};

use super::types::{HealingQueueEntry, StateHealingQueue};

pub async fn heal_state_trie_wrap(
    _state_root: H256,
    _store: Store,
    _peers: &PeerHandler,
    _staleness_timestamp: u64,
    _global_leafs_healed: &mut u64,
    _storage_accounts: &mut AccountStorageRoots,
    _code_hash_collector: &mut CodeHashCollector,
) -> Result<bool, SyncError> {
    // MPT state trie healing not supported on binary trie branch
    Ok(true)
}

/// Heals the trie given its state_root by fetching any missing nodes in it via p2p
/// Returns true if healing was fully completed or false if we need to resume healing on the next sync cycle
/// This method also stores modified storage roots in the db for heal_storage_trie
/// Note: downloaders only gets updated when heal_state_trie, once per snap cycle
#[allow(clippy::too_many_arguments)]
async fn heal_state_trie(
    _state_root: H256,
    _store: Store,
    _peers: PeerHandler,
    _staleness_timestamp: u64,
    _global_leafs_healed: &mut u64,
    _healing_queue: StateHealingQueue,
    _storage_accounts: &mut AccountStorageRoots,
    _code_hash_collector: &mut CodeHashCollector,
) -> Result<bool, SyncError> {
    // MPT state trie healing not supported on binary trie branch
    Ok(true)
}

/// Receives a set of state trie paths, fetches their respective nodes, stores them,
/// and returns their children paths and the paths that couldn't be fetched so they can be returned to the queue
fn heal_state_batch(
    batch: Vec<RequestMetadata>,
    _nodes: Vec<Node>,
    _store: Store,
    _healing_queue: &mut StateHealingQueue,
    _nodes_to_write: &mut Vec<(Nibbles, Node)>,
) -> Result<Vec<RequestMetadata>, SyncError> {
    // MPT state trie healing not supported on binary trie branch
    Ok(batch)
}

fn commit_node(
    node: Node,
    path: &Nibbles,
    parent_path: &Nibbles,
    healing_queue: &mut StateHealingQueue,
    nodes_to_write: &mut Vec<(Nibbles, Node)>,
) -> Result<(), SyncError> {
    nodes_to_write.push((path.clone(), node));

    if parent_path == path {
        return Ok(()); // Case where we're saving the root
    }

    let mut healing_queue_entry = healing_queue.remove(parent_path).ok_or_else(|| {
        SyncError::HealingQueueInconsistency(format!("{parent_path:?}"), format!("{path:?}"))
    })?;

    healing_queue_entry.pending_children_count -= 1;
    if healing_queue_entry.pending_children_count == 0 {
        commit_node(
            healing_queue_entry.node,
            parent_path,
            &healing_queue_entry.parent_path,
            healing_queue,
            nodes_to_write,
        )?;
    } else {
        healing_queue.insert(parent_path.clone(), healing_queue_entry);
    }
    Ok(())
}

/// Returns the partial paths to the node's children if they are not already part of the trie state
pub fn node_pending_children(
    node: &Node,
    path: &Nibbles,
    trie_state: &dyn TrieDB,
) -> Result<(usize, Vec<RequestMetadata>), TrieError> {
    let mut paths: Vec<RequestMetadata> = Vec::new();
    let mut pending_children_count: usize = 0;
    match &node {
        Node::Branch(node) => {
            for (index, child) in node.choices.iter().enumerate() {
                let child_path = path.clone().append_new(index as u8);
                if !child.is_valid() {
                    continue;
                }
                let validity = child
                    .get_node_checked(trie_state, child_path.clone())
                    .inspect_err(|_| {
                        debug!("Malformed data when doing get child of a branch node")
                    })?
                    .is_some();
                if validity {
                    continue;
                }

                pending_children_count += 1;
                paths.extend(vec![RequestMetadata {
                    hash: child.compute_hash(&NativeCrypto).finalize(&NativeCrypto),
                    path: child_path,
                    parent_path: path.clone(),
                }]);
            }
        }
        Node::Extension(node) => {
            let child_path = path.concat(&node.prefix);
            if !node.child.is_valid() {
                return Ok((0, vec![]));
            }
            let validity = node
                .child
                .get_node_checked(trie_state, child_path.clone())
                .inspect_err(|_| debug!("Malformed data when doing get child of a branch node"))?
                .is_some();
            if validity {
                return Ok((0, vec![]));
            }
            pending_children_count += 1;

            paths.extend(vec![RequestMetadata {
                hash: node
                    .child
                    .compute_hash(&NativeCrypto)
                    .finalize(&NativeCrypto),
                path: child_path,
                parent_path: path.clone(),
            }]);
        }
        _ => {}
    }
    Ok((pending_children_count, paths))
}
