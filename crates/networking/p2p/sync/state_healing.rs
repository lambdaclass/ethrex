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
    time::{Duration, Instant},
};

use ethrex_common::{H256, constants::EMPTY_KECCACK_HASH, types::AccountState};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_storage::Store;
use ethrex_trie::{EMPTY_TRIE_HASH, Nibbles, Node, NodeHash, NodeRef, TrieDB, TrieError};
use tokio::sync::mpsc::{Sender, channel};
use tracing::{debug, error, info};

use crate::{
    kademlia::PeerChannels,
    peer_handler::{PeerHandler, RequestMetadata, RequestStateTrieNodesError},
    rlpx::p2p::SUPPORTED_SNAP_CAPABILITIES,
    utils::current_unix_time,
};

/// The minimum amount of blocks from the head that we want to full sync during a snap sync
const MIN_FULL_BLOCKS: usize = 64;
/// Max size of bach to start a bytecode fetch request in queues
const BYTECODE_BATCH_SIZE: usize = 70;
/// Max size of a bach to start a storage fetch request in queues
pub const STORAGE_BATCH_SIZE: usize = 300;
/// Max size of a bach to start a node fetch request in queues
pub const NODE_BATCH_SIZE: usize = 500;
/// Maximum amount of concurrent paralell fetches for a queue
const MAX_PARALLEL_FETCHES: usize = 10;
/// Maximum amount of messages in a channel
const MAX_CHANNEL_MESSAGES: usize = 500;
/// Maximum amount of messages to read from a channel at once
const MAX_CHANNEL_READS: usize = 200;
/// Pace at which progress is shown via info tracing
pub const SHOW_PROGRESS_INTERVAL_DURATION: Duration = Duration::from_secs(2);
/// Amount of blocks to execute in a single batch during FullSync
const EXECUTE_BATCH_SIZE_DEFAULT: usize = 1024;
const MAX_SCORE: i64 = 10;

use super::SyncError;

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
    membatch: &mut HashMap<Nibbles, MembatchEntryValue>,
) -> Result<bool, SyncError> {
    let mut healing_done = false;
    info!("Starting state healing");
    while !healing_done {
        healing_done = heal_state_trie(
            state_root,
            store.clone(),
            peers.clone(),
            staleness_timestamp,
            global_leafs_healed,
            membatch,
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
    membatch: &mut HashMap<Nibbles, MembatchEntryValue>,
) -> Result<bool, SyncError> {
    // TODO:
    // Spawn a bytecode fetcher for this block
    // let (bytecode_sender, bytecode_receiver) = channel::<Vec<H256>>(MAX_CHANNEL_MESSAGES);
    // let bytecode_fetcher_handle = tokio::spawn(bytecode_fetcher(
    //     bytecode_receiver,
    //     peers.clone(),
    //     store.clone(),
    // ));
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
    let mut nodes_to_write: Vec<Node> = Vec::new();
    let mut db_joinset = tokio::task::JoinSet::new();

    // channel to send the tasks to the peers
    let (task_sender, mut task_receiver) = tokio::sync::mpsc::channel::<(
        H256,
        Result<Vec<Node>, RequestStateTrieNodesError>,
        Vec<RequestMetadata>,
    )>(1000);

    let peers_table = peers
        .peer_table
        .get_peer_channels(&SUPPORTED_SNAP_CAPABILITIES)
        .await;
    let mut downloaders: HashMap<H256, bool> = HashMap::from_iter(
        peers_table
            .iter()
            .map(|(peer_id, _peer_data)| (*peer_id, true)),
    );
    let mut scores: HashMap<H256, i64> =
        HashMap::from_iter(peers_table.iter().map(|(peer_id, _)| (*peer_id, 0)));

    // Contains both nodes and their corresponding paths to heal
    let mut nodes_to_heal = Vec::new();
    loop {
        let peers_table = peers
            .peer_table
            .get_peer_channels(&SUPPORTED_SNAP_CAPABILITIES)
            .await;
        let peers_table_2 = peers.peer_table.get_peer_channels(&[]).await;

        if last_update.elapsed() >= SHOW_PROGRESS_INTERVAL_DURATION {
            last_update = Instant::now();
            let downloads_rate =
                downloads_success as f64 / (downloads_success + downloads_fail) as f64;

            if is_stale {
                info!(
                    "State Healing stopping due to staleness, snap peers available {}, peers available {}, inflight_tasks: {inflight_tasks}, Maximum depth reached on loop {longest_path_seen}, leafs healed {leafs_healed}, global leafs healed {}, Download success rate {downloads_rate}, Paths to go {}",
                    peers_table.len(),
                    peers_table_2.len(),
                    global_leafs_healed,
                    paths.len()
                );
            } else {
                info!(
                    "State Healing in Progress, snap peers available {}, peers available {}, inflight_tasks: {inflight_tasks}, Maximum depth reached on loop {longest_path_seen}, leafs healed {leafs_healed}, global leafs healed {}, Download success rate {downloads_rate}, Paths to go {}",
                    peers_table.len(),
                    peers_table_2.len(),
                    global_leafs_healed,
                    paths.len()
                );
            }
            downloads_success = 0;
            downloads_fail = 0;

            for (peer_id, _) in peers_table {
                downloaders.entry(peer_id).or_insert(true);
                scores.entry(peer_id).or_insert(0);
            }
        }

        // Attempt to receive a response from one of the peers
        // TODO: this match response should score the appropiate peers
        if let Ok((peer_id, response, batch)) = task_receiver.try_recv() {
            inflight_tasks -= 1;
            // Mark the peer as available
            downloaders
                .entry(peer_id)
                .and_modify(|is_free| *is_free = true);
            match response {
                // If the peers responded with nodes, add them to the nodes_to_heal vector
                Ok(nodes) => {
                    leafs_healed += nodes
                        .iter()
                        .filter(|node| matches!(node, Node::Leaf(leaf_node)))
                        .count();
                    *global_leafs_healed += nodes
                        .iter()
                        .filter(|node| matches!(node, Node::Leaf(leaf_node)))
                        .count() as u64;
                    nodes_to_heal.push((nodes, batch));
                    downloads_success += 1;
                    scores.entry(peer_id).and_modify(|score| {
                        if *score < MAX_SCORE {
                            *score += 1;
                        }
                    });
                }
                // If the peers failed to respond, reschedule the task by adding the batch to the paths vector
                Err(_) => {
                    // TODO: Check if it's faster to reach the leafs of the trie
                    // by doing batch.extend(paths);paths = batch
                    // Or with a VecDequeue
                    paths.extend(batch);
                    downloads_fail += 1;
                    scores.entry(peer_id).and_modify(|score| *score -= 1);
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
                let Some((peer_id, mut peer_channel)) =
                    get_peer_with_highest_score_and_mark_it_as_occupied(
                        &peers,
                        &mut downloaders,
                        &scores,
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
            let return_paths =
                heal_state_batch(batch, nodes, store.clone(), membatch, &mut nodes_to_write)
                    .await
                    .inspect_err(|err| {
                        error!(
                            "We have found a sync error while trying to write to DB a batch: {err}"
                        )
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
            db_joinset.join_all();
            break;
        }

        // We check with a clock if we are stale
        if !is_stale && current_unix_time() > staleness_timestamp {
            info!("state healing is stale");
            is_stale = true;
        }

        if is_stale && nodes_to_heal.is_empty() && inflight_tasks == 0 {
            info!("Finisehd inflight tasks");
            db_joinset.join_all();
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
            node_missing_children(&node, &path.path, &membatch, trie.db())?;
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
            membatch.insert(
                path.path.clone(),
                MembatchEntryValue {
                    node,
                    children_not_in_storage_count: missing_children_count,
                    parent_path: path.parent_path.clone(),
                },
            );
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

    membatch.retain(|_, entry| {
        entry.children_not_in_storage_count -= 1;
        entry.children_not_in_storage_count == 0
    });
}

async fn get_peer_with_highest_score_and_mark_it_as_occupied(
    peers: &PeerHandler,
    downloaders: &mut HashMap<H256, bool>,
    scores: &HashMap<H256, i64>,
) -> Option<(H256, PeerChannels)> {
    // Filter the free downloaders
    let free_downloaders: Vec<H256> = downloaders
        .iter()
        .filter(|(_peer_id, is_free)| **is_free)
        .map(|(peer_id, _is_free)| peer_id.clone())
        .collect();

    // Get the peer with the highest score
    let mut peer_with_highest_score = free_downloaders.get(0)?;
    let mut highest_score = i64::MIN;
    for peer_id in &free_downloaders {
        let Some(score) = scores.get(&peer_id) else {
            continue;
        };
        if *score > highest_score {
            highest_score = *score;
            peer_with_highest_score = &peer_id;
        }
    }
    let Some(peer_channel) = peers
        .peer_table
        .get_peer_channel(*peer_with_highest_score)
        .await
    else {
        downloaders.remove(peer_with_highest_score);
        return None;
    };

    // Mark it as occupied
    downloaders
        .entry(*peer_with_highest_score)
        .and_modify(|is_free| *is_free = false);

    Some((*peer_with_highest_score, peer_channel))
}

/// Returns the partial paths to the node's children if they are not already part of the trie state
pub fn node_missing_children(
    node: &Node,
    path: &Nibbles,
    membatch: &HashMap<Nibbles, MembatchEntryValue>,
    trie_state: &dyn TrieDB,
) -> Result<(u64, Vec<RequestMetadata>), TrieError> {
    let mut paths: Vec<RequestMetadata> = Vec::new();
    let mut missing_children_count = 0_u64;
    match &node {
        Node::Branch(node) => {
            for (index, child) in node.choices.iter().enumerate() {
                if child.is_valid() && child.get_node(trie_state)?.is_none() {
                    missing_children_count += 1;
                    paths.extend(membatch_node_missing_children(
                        RequestMetadata {
                            hash: child.compute_hash().finalize(),
                            path: path.clone().append_new(index as u8),
                            parent_path: path.clone(),
                        },
                        membatch,
                        trie_state,
                    )?);
                }
            }
        }
        Node::Extension(node) => {
            if node.child.is_valid() && node.child.get_node(trie_state)?.is_none() {
                paths.extend(membatch_node_missing_children(
                    RequestMetadata {
                        hash: node.child.compute_hash().finalize(),
                        path: path.concat(node.prefix.clone()),
                        parent_path: path.clone(),
                    },
                    membatch,
                    trie_state,
                )?);
            }
        }
        _ => {}
    }
    Ok((missing_children_count, paths))
}

// This function searches for the nodes we have to download that are childs from the membatch
fn membatch_node_missing_children(
    node_request: RequestMetadata,
    membatch: &HashMap<Nibbles, MembatchEntryValue>,
    trie_state: &dyn TrieDB,
) -> Result<Vec<RequestMetadata>, TrieError> {
    if let Some(membatch_entry) = membatch.get(&node_request.path) {
        if membatch_entry.node.compute_hash().finalize() == node_request.hash {
            node_missing_children(
                &membatch_entry.node,
                &node_request.path,
                membatch,
                trie_state,
            )
            .map(|(counts, paths)| paths)
        } else {
            Ok(vec![node_request])
        }
    } else {
        Ok(vec![node_request])
    }
}
