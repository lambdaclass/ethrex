use std::fs::{self, metadata};

use crate::{genesis_file_path_from_network, read_genesis_file, set_datadir, DEFAULT_DATADIR};
use clap::ArgMatches;
use ethrex_blockchain::Blockchain;
use ethrex_common::types::Block;
use ethrex_rlp::decode::RLPDecode;
use ethrex_storage::{EngineType, Store};
use ethrex_vm::backends::EVM;
use tracing::info;

use crate::decode;

pub fn import_blocks_from_path(matches: &ArgMatches) {
    let path = matches
        .get_one::<String>("path")
        .expect("No path provided to import blocks");
    let data_dir = matches
        .get_one::<String>("datadir")
        .map_or(set_datadir(DEFAULT_DATADIR), |datadir| set_datadir(datadir));
    let evm = matches.get_one::<EVM>("evm").unwrap_or(&EVM::REVM);

    let store = {
        cfg_if::cfg_if! {
            if #[cfg(feature = "redb")] {
                let engine_type = EngineType::RedB;
            } else if #[cfg(feature = "libmdbx")] {
                let engine_type = EngineType::Libmdbx;
            } else {
                let engine_type = EngineType::InMemory;
                error!("No database specified. The feature flag `redb` or `libmdbx` should've been set while building.");
                panic!("Specify the desired database engine.");
            }
        }
        Store::new(&data_dir, engine_type).expect("Failed to create Store")
    };

    let mut network = matches
        .get_one::<String>("network")
        .expect("network is required")
        .clone();
    if let Some(genesis_path) = genesis_file_path_from_network(&network) {
        network = genesis_path;
    }

    let genesis = read_genesis_file(&network);
    store
        .add_initial_state(genesis.clone())
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

fn read_chain_file(chain_rlp_path: &str) -> Vec<Block> {
    let chain_file = std::fs::File::open(chain_rlp_path).expect("Failed to open chain rlp file");
    decode::chain_file(chain_file).expect("Failed to decode chain rlp file")
}

fn read_block_file(block_file_path: &str) -> Block {
    let encoded_block = std::fs::read(block_file_path)
        .unwrap_or_else(|_| panic!("Failed to read block file with path {}", block_file_path));
    Block::decode(&encoded_block)
        .unwrap_or_else(|_| panic!("Failed to decode block file {}", block_file_path))
}
