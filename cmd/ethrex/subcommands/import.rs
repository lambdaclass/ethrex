use std::fs::{self, metadata};

use crate::create_store_at_path;
use clap::ArgMatches;
use ethrex_blockchain::Blockchain;
use ethrex_common::types::{Block, Genesis};
use ethrex_rlp::decode::RLPDecode;

use ethrex_vm::backends::EVM;
use tracing::info;

use crate::decode;

use super::removedb;

pub fn import_blocks_from_path(
    matches: &ArgMatches,
    data_dir: String,
    evm: &EVM,
    genesis: Genesis,
) {
    let remove_db = *matches.get_one::<bool>("removedb").unwrap_or(&false);
    let path = matches
        .get_one::<String>("path")
        .expect("No path provided to import blocks");
    if remove_db {
        removedb::remove_db_file(&data_dir);
    }

    let store = create_store_at_path(&data_dir);

    store
        .add_initial_state(genesis)
        .expect("Failed to create genesis block");

    let blockchain = Blockchain::new(evm.clone(), store.clone());

    let path_metadata = metadata(path).expect("Failed to read path");
    let blocks = if path_metadata.is_dir() {
        let mut blocks = vec![];
        let dir_reader = fs::read_dir(path).expect("Failed to read blocks directory");
        for file_res in dir_reader {
            let file = file_res.expect("Failed to open file in directory");
            let path = file.path();
            let s = path
                .to_str()
                .expect("Path could not be converted into string");
            blocks.push(read_block_file(s));
        }
        blocks
    } else {
        info!("Importing blocks from chain file: {}", path);
        read_chain_file(path)
    };
    blockchain.import_blocks(&blocks);
}

pub fn read_chain_file(chain_rlp_path: &str) -> Vec<Block> {
    let chain_file = std::fs::File::open(chain_rlp_path).expect("Failed to open chain rlp file");
    decode::chain_file(chain_file).expect("Failed to decode chain rlp file")
}

pub fn read_block_file(block_file_path: &str) -> Block {
    let encoded_block = std::fs::read(block_file_path)
        .unwrap_or_else(|_| panic!("Failed to read block file with path {}", block_file_path));
    Block::decode(&encoded_block)
        .unwrap_or_else(|_| panic!("Failed to decode block file {}", block_file_path))
}
