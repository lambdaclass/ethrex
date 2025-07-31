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
    time::{Duration, Instant},
};

use ethrex_common::{H256, constants::EMPTY_KECCACK_HASH, types::AccountState};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_storage::Store;
use ethrex_trie::{EMPTY_TRIE_HASH, Nibbles, Node, NodeHash};
use prometheus::core::AtomicU64;
use tokio::sync::mpsc::{Sender, channel};
use tracing::{debug, error, info};

use crate::{
    kademlia::PeerChannels,
    peer_handler::{PeerHandler, RequestStateTrieNodesError},
    rlpx::p2p::SUPPORTED_SNAP_CAPABILITIES,
    sync::node_missing_children,
    utils::current_unix_time,
};

/// The minimum amount of blocks from the head that we want to full sync during a snap sync
const MIN_FULL_BLOCKS: usize = 64;
/// Max size of bach to start a bytecode fetch request in queues
const BYTECODE_BATCH_SIZE: usize = 70;
/// Max size of a bach to start a storage fetch request in queues
const STORAGE_BATCH_SIZE: usize = 300;
/// Max size of a bach to start a node fetch request in queues
const NODE_BATCH_SIZE: usize = 900;
/// Maximum amount of concurrent paralell fetches for a queue
const MAX_PARALLEL_FETCHES: usize = 10;
/// Maximum amount of messages in a channel
const MAX_CHANNEL_MESSAGES: usize = 500;
/// Maximum amount of messages to read from a channel at once
const MAX_CHANNEL_READS: usize = 200;
/// Pace at which progress is shown via info tracing
const SHOW_PROGRESS_INTERVAL_DURATION: Duration = Duration::from_secs(2);
/// Amount of blocks to execute in a single batch during FullSync
const EXECUTE_BATCH_SIZE_DEFAULT: usize = 1024;

use super::SyncError;

/// Heals the trie given its state_root by fetching any missing nodes in it via p2p
/// Returns true if healing was fully completed or false if we need to resume healing on the next sync cycle
/// This method also stores modified storage roots in the db for heal_storage_trie
pub(crate) async fn heal_state_trie(
    state_root: H256,
    store: Store,
    peers: PeerHandler,
    staleness_timestamp: u64,
) -> Result<bool, SyncError> {
    let mut paths = store.get_state_heal_paths().await?.unwrap_or_default();
    // Spawn a bytecode fetcher for this block
    // let (bytecode_sender, bytecode_receiver) = channel::<Vec<H256>>(MAX_CHANNEL_MESSAGES);
    // let bytecode_fetcher_handle = tokio::spawn(bytecode_fetcher(
    //     bytecode_receiver,
    //     peers.clone(),
    //     store.clone(),
    // ));
    // Add the current state trie root to the pending paths
    paths.push(Nibbles::default());
    let mut last_update = Instant::now();
    let mut inflight_tasks: u64 = 0;
    let mut is_stale = false;
    let mut longest_path_seen = 0;

    // channel to send the tasks to the peers
    let (task_sender, mut task_receiver) = tokio::sync::mpsc::channel::<(
        Result<Vec<Node>, RequestStateTrieNodesError>,
        Vec<Nibbles>,
    )>(1000);

    // channel to send the tasks to the peers
    let (returned_paths_sender, mut returned_paths_receiver) =
        tokio::sync::mpsc::channel::<Vec<Nibbles>>(1000);

    // let mut downloaders: BTreeMap<H256, bool> = BTreeMap::from_iter(
    //     peers_table
    //         .iter()
    //         .map(|(peer_id, _peer_data)| (*peer_id, true)),
    // );
    // Contains both nodes and their corresponding paths to heal
    let mut nodes_to_heal = Vec::new();
    loop {
        if last_update.elapsed() >= SHOW_PROGRESS_INTERVAL_DURATION {
            last_update = Instant::now();
            if is_stale {
                info!(
                    "State Healing stopping due to staleness, inflight_tasks: {inflight_tasks}, Maximum depth reached on loop {longest_path_seen}. Paths to go {}",
                    paths.len()
                );
            } else {
                info!(
                    "State Healing in Progress, inflight_tasks: {inflight_tasks}, Maximum depth reached on loop {longest_path_seen}. Paths to go {}",
                    paths.len()
                );
            }
        }

        // Attempt to receive a response from one of the peers
        // TODO: this match response should score the appropiate peers
        if let Ok((response, batch)) = task_receiver.try_recv() {
            inflight_tasks -= 1;
            match response {
                // If the peers responded with nodes, add them to the nodes_to_heal vector
                Ok(nodes) => {
                    nodes_to_heal.push((nodes, batch));
                }
                // If the peers failed to respond, reschedule the task by adding the batch to the paths vector
                Err(_) => {
                    paths.extend(batch);
                }
            }
        }

        // Attempt to receive paths returned by the healing tasks, and add them to the paths vector
        if let Ok(mut returned_paths) = returned_paths_receiver.try_recv() {
            inflight_tasks -= 1;
            longest_path_seen = usize::max(
                returned_paths
                    .iter()
                    .map(|nibbles_vec| nibbles_vec.len())
                    .max()
                    .unwrap_or_default(),
                longest_path_seen,
            );
            // We try to extend returned_paths so that the new nodes are the first to be downloaded.
            // This is so we do depth first search, which should make the algorithm more efficient if we need
            // to move the pivot. This is for testing.
            returned_paths.extend(paths);
            paths = returned_paths;
        }

        // TODO: add scoring
        let (peer_id, mut peer_channel) = peers
            .get_peer_channel_with_retry(&SUPPORTED_SNAP_CAPABILITIES)
            .await
            .unwrap();

        if !is_stale {
            let batch: Vec<Nibbles> = paths.drain(0..min(paths.len(), NODE_BATCH_SIZE)).collect();
            if !batch.is_empty() {
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
                    let _ = tx.send((response, batch)).await.inspect_err(|err| {
                        error!("Failed to send state trie nodes response. Error: {err}")
                    });
                });
            }
        }

        let store_cloned = store.clone();
        let tx = returned_paths_sender.clone();
        // If there is at least one "batch" of nodes to heal, heal it
        if let Some((nodes, batch)) = nodes_to_heal.pop() {
            inflight_tasks += 1;
            // TODO: consider adding a semaphore to limit the concurrent tasks that access the db
            tokio::spawn(async move {
                if let Ok(return_paths) = heal_state_batch(batch, nodes, store_cloned).await {
                    let _ = tx
                        .send(return_paths)
                        .await
                        .inspect_err(|err| error!("Failed to send returned paths. Error: {err}"));
                }
            });
        }

        // End loop if we have no more paths to fetch nor nodes to heal and no inflight tasks
        if paths.is_empty() && nodes_to_heal.is_empty() && inflight_tasks == 0 {
            info!("Nothing more to heal found");
            break;
        }

        // Process the results of each batch
        // for res in state_tasks.join_all().await {
        //     let return_paths = res?;
        //     // stale |= is_stale;
        //     paths.extend(return_paths);
        // }

        // We check with a clock if we are stale
        if !is_stale && current_unix_time() > staleness_timestamp {
            info!("state healing is stale");
            is_stale = true;
        }

        if is_stale && inflight_tasks == 0 {
            break;
        }
    }
    info!("State Healing stopped, signaling storage healer");
    // Save paths for the next cycle. If there are no paths left, clear it in case pivot becomes stale during storage
    info!("Caching {} paths for the next cycle", paths.len());
    store.set_state_heal_paths(paths.clone()).await?;
    // Send empty batch to signal that no more batches are incoming
    // bytecode_sender.send(vec![]).await?;
    // bytecode_fetcher_handle.await??;
    Ok(paths.is_empty())
}

/// Receives a set of state trie paths, fetches their respective nodes, stores them,
/// and returns their children paths and the paths that couldn't be fetched so they can be returned to the queue
async fn heal_state_batch(
    mut batch: Vec<Nibbles>,
    nodes: Vec<Node>,
    store: Store,
) -> Result<Vec<Nibbles>, SyncError> {
    let mut hashed_addresses = vec![];
    let mut code_hashes = vec![];
    // For each node:
    // - Add its children to the queue (if we don't have them already)
    // - If it is a leaf, request its bytecode & storage
    // - If it is a leaf, add its path & value to the trie
    {
        let trie: ethrex_trie::Trie = store.open_state_trie(*EMPTY_TRIE_HASH)?;
        for node in nodes.iter() {
            let path = batch.remove(0);
            batch.extend(node_missing_children(node, &path, trie.db())?);
            if let Node::Leaf(node) = &node {
                // Fetch bytecode & storage
                let account = AccountState::decode(&node.value)?;
                // By now we should have the full path = account hash
                let path = &path.concat(node.partial.clone()).to_bytes();
                if path.len() != 32 {
                    // Something went wrong
                    return Err(SyncError::CorruptPath);
                }
                let account_hash = H256::from_slice(path);
                if account.storage_root != *EMPTY_TRIE_HASH
                    && !store.contains_storage_node(account_hash, account.storage_root)?
                {
                    hashed_addresses.push(account_hash);
                }
                if account.code_hash != *EMPTY_KECCACK_HASH
                    && store.get_account_code(account.code_hash)?.is_none()
                {
                    code_hashes.push(account.code_hash);
                }
            }
        }
        // Write nodes to trie
        trie.db().put_batch(
            nodes
                .into_iter()
                .filter_map(|node| match node.compute_hash() {
                    hash @ NodeHash::Hashed(_) => Some((hash, node.encode_to_vec())),
                    NodeHash::Inline(_) => None,
                })
                .collect(),
        )?;
    }
    // Send storage & bytecode requests
    if !hashed_addresses.is_empty() {
        store
            .set_storage_heal_paths(
                hashed_addresses
                    .into_iter()
                    .map(|hash| (hash, vec![Nibbles::default()]))
                    .collect(),
            )
            .await?;
    }
    if !code_hashes.is_empty() {
        //bytecode_sender.send(code_hashes).await?;
    }
    Ok(batch)
}
