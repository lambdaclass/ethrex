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
    collections::HashMap,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use ethrex_common::{H256, types::AccountState};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_storage::Store;
use ethrex_trie::{EMPTY_TRIE_HASH, Nibbles, Node, NodeHash, TrieDB, TrieError};
use tracing::{debug, error, info};

use crate::{
    metrics::METRICS,
    peer_handler::{PeerHandler, RequestMetadata, RequestStateTrieNodesError},
    rlpx::p2p::SUPPORTED_SNAP_CAPABILITIES,
    sync::AccountStorageRoots,
    utils::current_unix_time,
};

/// Max size of a bach to start a storage fetch request in queues
pub const STORAGE_BATCH_SIZE: usize = 300;
/// Max size of a bach to start a node fetch request in queues
pub const NODE_BATCH_SIZE: usize = 500;
/// Pace at which progress is shown via info tracing
pub const SHOW_PROGRESS_INTERVAL_DURATION: Duration = Duration::from_secs(2);

use super::SyncError;

#[derive(Debug)]
pub struct MembatchEntryValue {
    node: Node,
    children_not_in_storage_count: u64,
    parent_path: Nibbles,
}

pub async fn heal_state_trie_wrap(
    state_root: H256,
    store: Store,
    peers: &PeerHandler,
    staleness_timestamp: u64,
    global_leafs_healed: &mut u64,
    storage_accounts: &mut AccountStorageRoots,
) -> Result<bool, SyncError> {
    let mut healing_done = false;
    *METRICS.current_step.lock().await = "Healing State".to_string();
    info!("Starting state healing");
    while !healing_done {
        healing_done = heal_state_trie(
            state_root,
            store.clone(),
            peers.clone(),
            staleness_timestamp,
            global_leafs_healed,
            HashMap::new(),
            storage_accounts,
        )
        .await?;
        if current_unix_time() > staleness_timestamp {
            info!("Stopped state healing due to staleness");
            break;
        }
    }
    info!("Stopped state healing");
    Ok(healing_done)
}

/// Heals the trie given its state_root by fetching any missing nodes in it via p2p
/// Returns true if healing was fully completed or false if we need to resume healing on the next sync cycle
/// This method also stores modified storage roots in the db for heal_storage_trie
/// Note: downloaders only gets updated when heal_state_trie, once per snap cycle
async fn heal_state_trie(
    state_root: H256,
    store: Store,
    peers: PeerHandler,
    staleness_timestamp: u64,
    global_leafs_healed: &mut u64,
    mut membatch: HashMap<Nibbles, MembatchEntryValue>,
    storage_accounts: &mut AccountStorageRoots,
) -> Result<bool, SyncError> {
    // Add the current state trie root to the pending paths
    let mut paths: Vec<RequestMetadata> = vec![RequestMetadata {
        hash: state_root,
        path: Nibbles::default(), // We need to be careful, the root parent is a special case
        parent_path: Nibbles::default(),
    }];
    let mut last_update = Instant::now();
    let mut inflight_tasks: u64 = 0;
    let mut is_stale = false;
    let mut longest_path_seen = 0;
    let mut downloads_success = 0;
    let mut downloads_fail = 0;
    let mut leafs_healed = 0;
    let mut empty_try_recv: u64 = 0;
    let mut nodes_to_write: Vec<Node> = Vec::new();
    let mut db_joinset = tokio::task::JoinSet::new();

    // channel to send the tasks to the peers
    let (task_sender, mut task_receiver) = tokio::sync::mpsc::channel::<(
        H256,
        Result<Vec<Node>, RequestStateTrieNodesError>,
        Vec<RequestMetadata>,
    )>(1000);
    // Contains both nodes and their corresponding paths to heal
    let mut nodes_to_heal = Vec::new();
    loop {
        if last_update.elapsed() >= SHOW_PROGRESS_INTERVAL_DURATION {
            let num_peers = peers
                .peer_table
                .get_peer_channels(&SUPPORTED_SNAP_CAPABILITIES)
                .await
                .len();
            last_update = Instant::now();
            let downloads_rate =
                downloads_success as f64 / (downloads_success + downloads_fail) as f64;

            METRICS
                .global_state_trie_leafs_healed
                .fetch_add(*global_leafs_healed, Ordering::Relaxed);
            METRICS
                .healing_empty_try_recv
                .store(empty_try_recv, Ordering::Relaxed);
            if is_stale {
                debug!(
                    "State Healing stopping due to staleness, snap peers available {num_peers}, inflight_tasks: {inflight_tasks}, Maximum depth reached on loop {longest_path_seen}, leafs healed {leafs_healed}, global leafs healed {}, Download success rate {downloads_rate}, Paths to go {}, Membatch size {}",
                    global_leafs_healed,
                    paths.len(),
                    membatch.len()
                );
            } else {
                debug!(
                    "State Healing in Progress, snap peers available {num_peers}, inflight_tasks: {inflight_tasks}, Maximum depth reached on loop {longest_path_seen}, leafs healed {leafs_healed}, global leafs healed {}, Download success rate {downloads_rate}, Paths to go {}, Membatch size {}",
                    global_leafs_healed,
                    paths.len(),
                    membatch.len()
                );
            }
            downloads_success = 0;
            downloads_fail = 0;

            peers
                .peer_scores
                .lock()
                .await
                .update_peers(&peers.peer_table)
                .await;
        }

        // Attempt to receive a response from one of the peers
        // TODO: this match response should score the appropiate peers
        let res = task_receiver.try_recv();
        if res.is_err() {
            empty_try_recv += 1;
        }
        if let Ok((peer_id, response, batch)) = res {
            inflight_tasks -= 1;
            // Mark the peer as available
            peers.peer_scores.lock().await.free_peer(peer_id);
            match response {
                // If the peers responded with nodes, add them to the nodes_to_heal vector
                Ok(nodes) => {
                    for (node, meta) in nodes.iter().zip(batch.iter()) {
                        if let Node::Leaf(node) = node {
                            let account = AccountState::decode(&node.value).expect("decode failed");
                            let account_hash = H256::from_slice(
                                &meta.path.concat(node.partial.clone()).to_bytes(),
                            );
                            if account.storage_root != *EMPTY_TRIE_HASH {
                                storage_accounts.healed_accounts.insert(account_hash);
                            }
                            storage_accounts
                                .accounts_with_storage_root
                                .remove(&account_hash);
                        }
                    }
                    leafs_healed += nodes
                        .iter()
                        .filter(|node| matches!(node, Node::Leaf(_)))
                        .count();
                    *global_leafs_healed += nodes
                        .iter()
                        .filter(|node| matches!(node, Node::Leaf(_)))
                        .count() as u64;
                    nodes_to_heal.push((nodes, batch));
                    downloads_success += 1;
                    peers.peer_scores.lock().await.record_success(peer_id);
                }
                // If the peers failed to respond, reschedule the task by adding the batch to the paths vector
                Err(_) => {
                    // TODO: Check if it's faster to reach the leafs of the trie
                    // by doing batch.extend(paths);paths = batch
                    // Or with a VecDequeue
                    paths.extend(batch);
                    downloads_fail += 1;
                    peers.peer_scores.lock().await.record_failure(peer_id);
                }
            }
        }

        if !is_stale {
            let batch: Vec<RequestMetadata> =
                paths.drain(0..min(paths.len(), NODE_BATCH_SIZE)).collect();
            if !batch.is_empty() {
                longest_path_seen = usize::max(
                    batch
                        .iter()
                        .map(|request_metadata| request_metadata.path.len())
                        .max()
                        .unwrap_or_default(),
                    longest_path_seen,
                );
                let Some((peer_id, mut peer_channel)) = peers
                    .peer_scores
                    .lock()
                    .await
                    .get_peer_channel_with_highest_score_and_mark_as_used(
                        &peers.peer_table,
                        &SUPPORTED_SNAP_CAPABILITIES,
                    )
                    .await
                else {
                    // If there are no peers available, re-add the batch to the paths vector, and continue
                    paths.extend(batch);
                    continue;
                };

                let tx = task_sender.clone();
                inflight_tasks += 1;

                tokio::spawn(async move {
                    // TODO: check errors to determine whether the current block is stale
                    let response = PeerHandler::request_state_trienodes(
                        &mut peer_channel,
                        state_root,
                        batch.clone(),
                    )
                    .await;
                    // TODO: add error handling
                    tx.send((peer_id, response, batch))
                        .await
                        .inspect_err(|err| {
                            error!("Failed to send state trie nodes response. Error: {err}")
                        })
                });
                tokio::task::yield_now().await;
            }
        }

        // If there is at least one "batch" of nodes to heal, heal it
        if let Some((nodes, batch)) = nodes_to_heal.pop() {
            let return_paths = heal_state_batch(
                batch,
                nodes,
                store.clone(),
                &mut membatch,
                &mut nodes_to_write,
            )
            .await
            .inspect_err(|err| {
                error!("We have found a sync error while trying to write to DB a batch: {err}")
            })?;
            paths.extend(return_paths);
        }

        let is_done = paths.is_empty() && nodes_to_heal.is_empty() && inflight_tasks == 0;

        if nodes_to_write.len() > 100_000 || is_done || is_stale {
            let to_write = nodes_to_write;
            nodes_to_write = Vec::new();
            let store = store.clone();
            if db_joinset.len() > 3 {
                db_joinset.join_next().await;
            }
            db_joinset.spawn_blocking(|| {
                spawned_rt::tasks::block_on(async move {
                    // TODO: replace put batch with the async version
                    let trie_db = store
                        .open_state_trie(*EMPTY_TRIE_HASH)
                        .expect("Store should open");
                    let db = trie_db.db();
                    db.put_batch(
                        to_write
                            .into_iter()
                            .filter_map(|node| match node.compute_hash() {
                                hash @ NodeHash::Hashed(_) => Some((hash, node.encode_to_vec())),
                                NodeHash::Inline(_) => None,
                            })
                            .collect(),
                    )
                    .expect("The put batch on the store failed");
                })
            });
        }

        // End loop if we have no more paths to fetch nor nodes to heal and no inflight tasks
        if is_done {
            info!("Nothing more to heal found");
            db_joinset.join_all().await;
            break;
        }

        // We check with a clock if we are stale
        if !is_stale && current_unix_time() > staleness_timestamp {
            info!("state healing is stale");
            is_stale = true;
        }

        if is_stale && nodes_to_heal.is_empty() && inflight_tasks == 0 {
            info!("Finisehd inflight tasks");
            db_joinset.join_all().await;
            break;
        }
    }
    info!("State Healing stopped, signaling storage healer");
    // Save paths for the next cycle. If there are no paths left, clear it in case pivot becomes stale during storage
    // Send empty batch to signal that no more batches are incoming
    // bytecode_sender.send(vec![]).await?;
    // bytecode_fetcher_handle.await??;
    Ok(paths.is_empty())
}

/// Receives a set of state trie paths, fetches their respective nodes, stores them,
/// and returns their children paths and the paths that couldn't be fetched so they can be returned to the queue
async fn heal_state_batch(
    mut batch: Vec<RequestMetadata>,
    nodes: Vec<Node>,
    store: Store,
    membatch: &mut HashMap<Nibbles, MembatchEntryValue>,
    nodes_to_write: &mut Vec<Node>, // TODO: change tuple to struct
) -> Result<Vec<RequestMetadata>, SyncError> {
    let trie = store.open_state_trie(*EMPTY_TRIE_HASH)?;
    for node in nodes.into_iter() {
        let path = batch.remove(0);
        let (missing_children_count, missing_children) =
            node_missing_children(&node, &path.path, trie.db())?;
        batch.extend(missing_children);
        if missing_children_count == 0 {
            commit_node(
                node,
                &path.path,
                &path.parent_path,
                membatch,
                nodes_to_write,
            );
        } else {
            let entry = MembatchEntryValue {
                node: node.clone(),
                children_not_in_storage_count: missing_children_count,
                parent_path: path.parent_path.clone(),
            };
            membatch.insert(path.path.clone(), entry);
        }
    }
    Ok(batch)
}

fn commit_node(
    node: Node,
    path: &Nibbles,
    parent_path: &Nibbles,
    membatch: &mut HashMap<Nibbles, MembatchEntryValue>,
    nodes_to_write: &mut Vec<Node>,
) {
    nodes_to_write.push(node);

    if parent_path == path {
        return; // Case where we're saving the root
    }

    let mut membatch_entry = membatch.remove(parent_path).unwrap_or_else(|| {
        panic!("The parent should exist. Parent: {parent_path:?}, path: {path:?}")
    });

    membatch_entry.children_not_in_storage_count -= 1;
    if membatch_entry.children_not_in_storage_count == 0 {
        commit_node(
            membatch_entry.node,
            parent_path,
            &membatch_entry.parent_path,
            membatch,
            nodes_to_write,
        );
    } else {
        membatch.insert(parent_path.clone(), membatch_entry);
    }
}

/// Returns the partial paths to the node's children if they are not already part of the trie state
pub fn node_missing_children(
    node: &Node,
    path: &Nibbles,
    trie_state: &dyn TrieDB,
) -> Result<(u64, Vec<RequestMetadata>), TrieError> {
    let mut paths: Vec<RequestMetadata> = Vec::new();
    let mut missing_children_count = 0_u64;
    match &node {
        Node::Branch(node) => {
            for (index, child) in node.choices.iter().enumerate() {
                if child.is_valid() && child.get_node(trie_state)?.is_none() {
                    missing_children_count += 1;
                    paths.extend(vec![RequestMetadata {
                        hash: child.compute_hash().finalize(),
                        path: path.clone().append_new(index as u8),
                        parent_path: path.clone(),
                    }]);
                }
            }
        }
        Node::Extension(node) => {
            if node.child.is_valid() && node.child.get_node(trie_state)?.is_none() {
                missing_children_count += 1;

                paths.extend(vec![RequestMetadata {
                    hash: node.child.compute_hash().finalize(),
                    path: path.concat(node.prefix.clone()),
                    parent_path: path.clone(),
                }]);
            }
        }
        _ => {}
    }
    Ok((missing_children_count, paths))
}
