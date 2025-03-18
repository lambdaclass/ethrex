use std::fs::{self, metadata};

use clap::ArgMatches;

use ethrex_common::types::Block;
use ethrex_vm::backends::EvmEngine;
use tracing::info;

use crate::{
    initializers::{init_blockchain, init_store},
    utils,
};

use super::removedb;

fn get_import_blocks(path: &str) -> Vec<Block> {
    let path_metadata = metadata(path).expect("Failed to read path");
    if path_metadata.is_dir() {
        let mut blocks = vec![];
        let dir_reader = fs::read_dir(path).expect("Failed to read blocks directory");
        for file_res in dir_reader {
            let file = file_res.expect("Failed to open file in directory");
            let path = file.path();
            let s = path
                .to_str()
                .expect("Path could not be converted into string");
            blocks.push(utils::read_block_file(s));
        }
        blocks
    } else {
        info!("Importing blocks from chain file: {}", path);
        utils::read_chain_file(path)
    }
}

pub fn import_blocks_from_path(
    matches: &ArgMatches,
    data_dir: String,
    evm: EvmEngine,
    network: &str,
) {
    let remove_db = *matches.get_one::<bool>("removedb").unwrap_or(&false);
    let should_batch = *matches.get_one::<bool>("batch").unwrap_or(&false);

    let path = matches
        .get_one::<String>("path")
        .expect("No path provided to import blocks");
    if remove_db {
        removedb::remove_db(&data_dir);
    }

    let store = init_store(&data_dir, network);
    let blockchain = init_blockchain(evm, store.clone());

    let blocks = get_import_blocks(path);

    if should_batch {
        blockchain.import_blocks_in_batch(&blocks);
        store
            .mark_chain_as_canonical(&blocks)
            .expect("Chain could not be marked as canonical in the db");
    } else {
        blockchain.import_blocks(&blocks);
    }
}

pub fn import_blocks_from_datadir(data_dir: String, evm: EvmEngine, network: &str, path: &str) {
    let store = init_store(&data_dir, network);
    let blockchain = init_blockchain(evm, store);
    let blocks = get_import_blocks(path);

    blockchain.import_blocks(&blocks);
}
