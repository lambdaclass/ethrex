//! This module contains the logic for state healing
//! State healing begins after we already downloaded the whole state trie and rebuilt it locally
//! It's purpose is to fix inconsistencies with the canonical state trie by downloading all the trie nodes that we don't have starting from the root node
//! The reason for these inconsistencies is that state download can spawn across multiple sync cycles each with a different pivot,
//! meaning that the resulting trie is made up of fragments of different state tries and is not consistent with any block's state trie
//! For each node downloaded, will add it to the trie's state and check if we have its children stored, if we don't we will download each missing child
//! Note that during this process the state trie for the pivot block and any prior pivot block will not be in a consistent state
//! This process will stop once it has fixed all trie inconsistencies or when the pivot becomes stale, in which case it can be resumed on the next cycle
//! All healed accounts will also have their bytecodes and storages healed by the corresponding processes

use std::{cmp::min, time::Instant};

use ethrex_common::{H256, constants::EMPTY_KECCACK_HASH, types::AccountState};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_storage::Store;
use ethrex_trie::{EMPTY_TRIE_HASH, Nibbles, Node, NodeHash};
use tokio::sync::mpsc::{Sender, channel};
use tracing::{debug, info};

use crate::{
    peer_handler::PeerHandler,
    sync::{
        MAX_CHANNEL_MESSAGES, MAX_PARALLEL_FETCHES, NODE_BATCH_SIZE,
        SHOW_PROGRESS_INTERVAL_DURATION, bytecode_fetcher, node_missing_children,
    },
};

use super::SyncError;

/// Heals the trie given its state_root by fetching any missing nodes in it via p2p
/// Returns true if healing was fully completed or false if we need to resume healing on the next sync cycle
pub(crate) async fn heal_state_trie(
    state_root: H256,
    store: Store,
    peers: PeerHandler,
) -> Result<bool, SyncError> {
    let mut paths = store.get_state_heal_paths().await?.unwrap_or_default();
    info!(
        "Starting state healing, pre-existing paths: {}",
        paths.len()
    );
    let healing_start = Instant::now();
    let mut total_healed: usize = 0;
    // Spawn a bytecode fetcher for this block
    let (bytecode_sender, bytecode_receiver) = channel::<Vec<H256>>(MAX_CHANNEL_MESSAGES);
    let bytecode_fetcher_handle = tokio::spawn(bytecode_fetcher(
        bytecode_receiver,
        peers.clone(),
        store.clone(),
    ));
    // Add the current state trie root to the pending paths
    paths.push(Nibbles::default());
    let mut last_update = Instant::now();
    while !paths.is_empty() {
        if last_update.elapsed() >= SHOW_PROGRESS_INTERVAL_DURATION {
            last_update = Instant::now();
            let speed = healing_start.elapsed().as_millis() / total_healed as u128;
            info!(
                "State Healing in Progress, pending paths: {}, healing speed: {}ms/node",
                paths.len(),
                speed
            );
        }
        // Spawn multiple parallel requests
        let mut state_tasks = tokio::task::JoinSet::new();
        for _ in 0..MAX_PARALLEL_FETCHES {
            // Spawn fetcher for the batch
            let batch = paths.drain(0..min(paths.len(), NODE_BATCH_SIZE)).collect();
            state_tasks.spawn(heal_state_batch(
                state_root,
                batch,
                peers.clone(),
                store.clone(),
                bytecode_sender.clone(),
            ));
            // End loop if we have no more paths to fetch
            if paths.is_empty() {
                break;
            }
        }
        // Process the results of each batch
        let mut stale = false;
        for res in state_tasks.join_all().await {
            let (return_paths, is_stale, nodes_healed) = res?;
            total_healed += nodes_healed;
            stale |= is_stale;
            paths.extend(return_paths);
        }
        if stale {
            break;
        }
    }
    debug!("State Healing stopped, signaling storage healer");
    // Save paths for the next cycle
    if !paths.is_empty() {
        debug!("Caching {} paths for the next cycle", paths.len());
        store.set_state_heal_paths(paths.clone()).await?;
    }
    // Send empty batch to signal that no more batches are incoming
    bytecode_sender.send(vec![]).await?;
    bytecode_fetcher_handle.await??;
    Ok(paths.is_empty())
}

/// Receives a set of state trie paths, fetches their respective nodes, stores them,
/// and returns their children paths and the paths that couldn't be fetched so they can be returned to the queue
/// Also returns a boolean indicating if the pivot became stale during the request
async fn heal_state_batch(
    state_root: H256,
    mut batch: Vec<Nibbles>,
    peers: PeerHandler,
    store: Store,
    bytecode_sender: Sender<Vec<H256>>,
) -> Result<(Vec<Nibbles>, bool, usize), SyncError> {
    if let Some(nodes) = peers
        .request_state_trienodes(state_root, batch.clone())
        .await
    {
        let nodes_received = nodes.len();
        debug!("Received {} state nodes", nodes.len());
        let mut hashed_addresses = vec![];
        let mut code_hashes = vec![];
        // For each fetched node:
        // - Add its children to the queue (if we don't have them already)
        // - If it is a leaf, request its bytecode & storage
        // - If it is a leaf, add its path & value to the trie
        {
            let trie = store.open_state_trie(*EMPTY_TRIE_HASH)?;
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
            bytecode_sender.send(code_hashes).await?;
        }
        Ok((batch, false, nodes_received))
    } else {
        Ok((batch, true, 0))
    }
}
