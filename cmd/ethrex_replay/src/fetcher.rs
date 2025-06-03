use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use ethrex_common::types::{code_hash, AccountState, ChainConfig};
use ethrex_rlp::decode::RLPDecode;
use ethrex_rpc::types::block_execution_witness::ExecutionWitnessResult;
use ethrex_storage::hash_address;
use ethrex_trie::{Node, Trie};
use ethrex_vm::{ProverDB, ProverDBError};
use eyre::Context;

use crate::{
    cache::{load_cache, write_cache, Cache},
    rpc::{get_block, get_latest_block_number, get_witness, get_witness_range},
};

pub async fn or_latest(maybe_number: Option<usize>, rpc_url: &str) -> eyre::Result<usize> {
    Ok(match maybe_number {
        Some(v) => v,
        None => get_latest_block_number(rpc_url).await?,
    })
}

pub async fn get_blockdata(
    rpc_url: &str,
    chain_config: ChainConfig,
    block_number: usize,
) -> eyre::Result<Cache> {
    if let Ok(cache) = load_cache(block_number) {
        return Ok(cache);
    }
    let block = get_block(rpc_url, block_number)
        .await
        .wrap_err("failed to fetch block")?;

    let parent_block_header = get_block(rpc_url, block_number - 1)
        .await
        .wrap_err("failed to fetch block")?
        .header;

    println!("populating rpc db cache");
    let witness = get_witness(rpc_url, block_number)
        .await
        .wrap_err("Failed to get execution witness")?;

    let db = to_exec_db_from_witness(chain_config, &witness)
        .wrap_err("Failed to build prover db from execution witness")?;

    let cache = Cache {
        blocks: vec![block],
        parent_block_header,
        witness,
        chain_config,
        db,
    };
    write_cache(&cache).expect("failed to write cache");
    Ok(cache)
}

pub async fn get_rangedata(
    rpc_url: &str,
    chain_config: ChainConfig,
    from: usize,
    to: usize,
) -> eyre::Result<Cache> {
    let mut blocks = Vec::with_capacity(to - from);
    for block_number in from..=to {
        let block = get_block(rpc_url, block_number)
            .await
            .wrap_err("failed to fetch block")?;
        blocks.push(block);
    }

    let parent_block_header = get_block(rpc_url, from - 1)
        .await
        .wrap_err("failed to fetch block")?
        .header;

    let witness = get_witness_range(rpc_url, from, to)
        .await
        .wrap_err("Failed to get execution witness for range")?;

    let db = to_exec_db_from_witness(chain_config, &witness)
        .wrap_err("Failed to build prover db from execution witness")?;

    let cache = Cache {
        blocks,
        parent_block_header,
        witness,
        chain_config,
        db,
    };
    // TODO fix this
    // write_cache(&cache).expect("failed to write cache");

    Ok(cache)
}

pub fn to_exec_db_from_witness(
    chain_config: ChainConfig,
    witness: &ExecutionWitnessResult,
) -> Result<ethrex_vm::ProverDB, ProverDBError> {
    let mut code = HashMap::new();
    for witness_code in &witness.codes {
        code.insert(code_hash(witness_code), witness_code.clone());
    }

    let mut block_hashes = HashMap::new();

    let initial_state_hash = witness
        .block_headers
        .first()
        .expect("no headers?")
        .state_root;

    for header in witness.block_headers.iter() {
        block_hashes.insert(header.number, header.hash());
    }

    let mut initial_node = None;

    for node in witness.state.iter() {
        let x = Node::decode_raw(node).expect("invalid node");
        let hash = x.compute_hash().finalize();
        if hash == initial_state_hash {
            initial_node = Some(node.clone());
            break;
        }
    }

    let state_trie =
        Trie::from_nodes(initial_node.as_ref(), &witness.state).expect("failed to create trie");

    let mut storage_tries = HashMap::new();
    for (addr, nodes) in &witness.storage_tries {
        let hashed_address = hash_address(addr);
        let Some(encoded_state) = state_trie
            .get(&hashed_address)
            .expect("Failed to get from trie")
        else {
            // TODO re-explore this. When testing with hoodi this happened block 521990 an this continue fixed it
            continue;
        };

        let state =
            AccountState::decode(&encoded_state).expect("Failed to get state from encoded state");

        let mut initial_node = None;

        for node in nodes.iter() {
            let x = Node::decode_raw(node).expect("invalid node");
            let hash = x.compute_hash().finalize();
            if hash == state.storage_root {
                initial_node = Some(node);
                break;
            }
        }

        let storage_trie = Trie::from_nodes(initial_node, nodes).unwrap();

        storage_tries.insert(*addr, storage_trie);
    }

    let state_trie = Arc::new(Mutex::new(state_trie));
    let storage_tries = Arc::new(Mutex::new(storage_tries));

    Ok(ProverDB {
        code,
        block_hashes,
        chain_config,
        state_trie,
        storage_tries,
    })
}
