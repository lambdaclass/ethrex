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
use prometheus::core::AtomicU64;
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

pub async fn heal_state_trie_wrap(
    state_root: H256,
    store: Store,
    peers: &PeerHandler,
    staleness_timestamp: u64,
) -> Result<bool, SyncError> {
    let mut healing_done = false;
    info!("Starting state healing");
    while !healing_done {
        healing_done = heal_state_trie(
            state_root,
            store.clone(),
            peers.clone(),
            staleness_timestamp,
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
        path: Nibbles::default(),
    }];
    let mut last_update = Instant::now();
    let mut inflight_tasks: u64 = 0;
    let mut is_stale = false;
    let mut longest_path_seen = 0;
    let mut downloads_success = 0;
    let mut downloads_fail = 0;
    let mut leafs_healed = 0;

    // channel with the task result, including the peer id, the result of the request and what we asked
    let (task_sender, mut task_receiver) = tokio::sync::mpsc::channel::<(
        H256,                                          // peer id
        Result<Vec<Node>, RequestStateTrieNodesError>, // result of the request
        Vec<RequestMetadata>,                          // What we asked for from the peers
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
                    "State Healing stopping due to staleness, snap peers available {}, peers available {}, inflight_tasks: {inflight_tasks}, Maximum depth reached on loop {longest_path_seen}, leafs healed {leafs_healed}, Download success rate {downloads_rate}, Paths to go {}",
                    peers_table.len(),
                    peers_table_2.len(),
                    paths.len()
                );
            } else {
                info!(
                    "State Healing in Progress, snap peers available {}, peers available {}, inflight_tasks: {inflight_tasks}, Maximum depth reached on loop {longest_path_seen}, leafs healed {leafs_healed}, Download success rate {downloads_rate}, Paths to go {}",
                    peers_table.len(),
                    peers_table_2.len(),
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
                    println!(
                        "(SUPERLOG) DOWNLOADED batch of len {} when asked for {} and paths: {:?}",
                        nodes.len(),
                        batch.len(),
                        batch
                            .iter()
                            .map(|request_metadata| &request_metadata.path)
                            .collect::<Vec<&Nibbles>>()
                    );
                    leafs_healed += nodes
                        .iter()
                        .filter(|node| matches!(node, Node::Leaf(leaf_node)))
                        .count();
                    nodes_to_heal.push((nodes, batch));
                    downloads_success += 1;
                    scores.entry(peer_id).and_modify(|score| {
                        if *score < MAX_SCORE {
                            *score += 1;
                        }
                    });
                }
                // If the peers failed to respond, reschedule the task by adding the batch to the paths vector
                Err(err) => {
                    // TODO: Check if it's faster to reach the leafs of the trie
                    // by doing batch.extend(paths);paths = batch
                    // Or with a VecDequeue
                    println!(
                        "(SUPERLOG) RETRYING batch {:?} because of {err:?}",
                        batch
                            .iter()
                            .map(|request_metadata| &request_metadata.path)
                            .collect::<Vec<&Nibbles>>()
                    );
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
                println!(
                    "(SUPERLOG) GATHERING batch {:?}",
                    batch
                        .iter()
                        .map(|request_metadata| &request_metadata.path)
                        .collect::<Vec<&Nibbles>>()
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
                    println!("(SUPERLOG) NOPEERS reinserting batch");
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
                    println!(
                        "(SUPERLOG) RESPONSE batch {:?}",
                        batch
                            .iter()
                            .map(|request_metadata| &request_metadata.path)
                            .collect::<Vec<&Nibbles>>()
                    );
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
            match heal_state_batch(batch, nodes, store.clone()).await {
                Ok(return_paths) => paths.extend(return_paths),
                Err(err) => {
                    error!("We have found a sync error while trying to write to DB a batch: {err}");
                }
            }
        }

        // End loop if we have no more paths to fetch nor nodes to heal and no inflight tasks
        if paths.is_empty() && nodes_to_heal.is_empty() && inflight_tasks == 0 {
            info!("Finished first download, checking the cache of previous downloads");
            let trie = store.open_state_trie(state_root).expect("Store Error");
            let unfilted_paths = store
                .get_state_heal_paths()
                .await
                .expect("Store Error")
                .unwrap_or_default();
            println!(
                "(SUPERLOG) UNFPOPCACHE unfilted_paths {:?}",
                unfilted_paths
                    .iter()
                    .map(|(path, _)| path)
                    .collect::<Vec<_>>()
            );

            paths = unfilted_paths
                .into_iter()
                .filter(|(path, _)| trie.get_node(&path.encode_compact()).is_err())
                .map(|(path, hash)| RequestMetadata { path, hash })
                .collect();
            println!(
                "(SUPERLOG) POPCACHE paths {:?}",
                paths
                    .iter()
                    .map(|request_metadata| &request_metadata.path)
                    .collect::<Vec<&Nibbles>>()
            );
            store
                .set_state_heal_paths(Vec::new())
                .await
                .expect("Store Error");
            if paths.is_empty() {
                info!("Nothing more to heal found");
                break;
            }
        }

        // We check with a clock if we are stale
        if !is_stale && current_unix_time() > staleness_timestamp {
            info!("state healing is stale");
            is_stale = true;
        }

        if is_stale && nodes_to_heal.is_empty() && inflight_tasks == 0 {
            info!("Caching {} paths for the next cycle", paths.len());
            let old_paths: Vec<RequestMetadata> = store
                .get_state_heal_paths()
                .await
                .expect("Store Error")
                .unwrap_or_default()
                .into_iter()
                .map(|(path, hash)| RequestMetadata { path, hash })
                .collect();
            info!("We had {} old paths from a previous cycle", old_paths.len());
            println!(
                "(SUPERLOG) STALED paths {:?}",
                paths
                    .iter()
                    .map(|request_metadata| &request_metadata.path)
                    .collect::<Vec<&Nibbles>>()
            );
            println!(
                "(SUPERLOG) STALE_MERGEWITH olds {:?}",
                old_paths
                    .iter()
                    .map(|request_metadata| &request_metadata.path)
                    .collect::<Vec<&Nibbles>>()
            );
            let mut paths_hashmap: HashMap<Nibbles, RequestMetadata> = HashMap::from_iter(
                old_paths
                    .into_iter()
                    .map(|request_metadata| (request_metadata.path.clone(), request_metadata)),
            );

            for path in paths.clone() {
                paths_hashmap.insert(path.path.clone(), path);
            }
            store
                .set_state_heal_paths(
                    paths_hashmap
                        .values()
                        .map(|request_metadata| {
                            (request_metadata.path.clone(), request_metadata.hash)
                        })
                        .collect(),
                )
                .await?;
            break;
        }
    }
    info!("State Healing stopped, signaling storage healer");
    // Save paths for the next cycle. If there are no paths left, clear it in case pivot becomes stale during storage
    // Send empty batch to signal that no more batches are incoming
    // bytecode_sender.send(vec![]).await?;
    // bytecode_fetcher_handle.await??;
    println!("(SUPERLOG) ENDED paths {:?}", paths.len());
    Ok(paths.is_empty())
}

/// Receives a set of state trie paths, fetches their respective nodes, stores them,
/// and returns their children paths and the paths that couldn't be fetched so they can be returned to the queue
async fn heal_state_batch(
    mut batch: Vec<RequestMetadata>,
    nodes: Vec<Node>,
    store: Store,
) -> Result<Vec<RequestMetadata>, SyncError> {
    // For each node:
    // - Add its children to the queue (if we don't have them already)
    // - If it is a leaf, request its bytecode & storage
    // - If it is a leaf, add its path & value to the trie
    {
        let trie: ethrex_trie::Trie = store.open_state_trie(*EMPTY_TRIE_HASH)?;
        println!(
            "(SUPERLOG) PRE_NMC batch len {}, paths {:?}",
            batch.len(),
            batch
                .iter()
                .map(|request_metadata| &request_metadata.path)
                .collect::<Vec<&Nibbles>>()
        );
        for node in nodes.iter() {
            let path = batch.remove(0);
            batch.extend(node_missing_children(node, &path.path, trie.db())?);
        }
        println!(
            "(SUPERLOG) POST_NMC batch len {}, paths {:?}",
            batch.len(),
            batch
                .iter()
                .map(|request_metadata| &request_metadata.path)
                .collect::<Vec<&Nibbles>>()
        );
        println!("(SUPERLOG) WRITEDB nodes {:?}", nodes.len());
        // Write nodes to trie
        trie.db().put_batch(
            nodes
                .into_iter()
                .map(|node| (node.compute_hash(), node.encode_to_vec()))
                .collect(),
        )?;
    }
    Ok(batch)
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
    parent_path: &Nibbles,
    trie_state: &dyn TrieDB,
) -> Result<Vec<RequestMetadata>, TrieError> {
    let mut paths: Vec<RequestMetadata> = Vec::new();
    match &node {
        Node::Branch(node) => {
            for (index, child) in node.choices.iter().enumerate() {
                if child.is_valid() {
                    match child {
                        NodeRef::Node(_, _) => {
                            error!("Decoding gave us a node")
                        }
                        NodeRef::Hash(NodeHash::Inline(node)) => {
                            error!("We have inflined nodes in our tree {node:?}")
                        }
                        _ => (),
                    }
                }
                if child.is_valid() && child.get_node(trie_state)?.is_none() {
                    paths.push(RequestMetadata {
                        hash: child.compute_hash().finalize(),
                        path: parent_path.append_new(index as u8),
                    });
                }
            }
        }
        Node::Extension(node) => {
            if node.child.is_valid() {
                match node.child {
                    NodeRef::Node(_, _) => {
                        error!("Decoding gave us a node")
                    }
                    NodeRef::Hash(NodeHash::Inline(node)) => {
                        error!("We have inflined nodes in our tree {node:?}")
                    }
                    _ => (),
                }
            }
            if node.child.is_valid() && node.child.get_node(trie_state)?.is_none() {
                paths.push(RequestMetadata {
                    hash: node.child.compute_hash().finalize(),
                    path: parent_path.concat(node.prefix.clone()),
                });
            }
        }
        _ => {}
    }
    println!(
        "(SUPERLOG) MISSINGCHILD paths {:?}",
        paths
            .iter()
            .map(|request_metadata| &request_metadata.path)
            .collect::<Vec<&Nibbles>>()
    );
    Ok(paths)
}
