use std::{
    fs::File,
    io::{BufReader, BufWriter, Read},
};

use ethrex_core::types::{Block, BlockHeader};
use ethrex_vm::execution_db::ExecutionDB;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct Cache {
    pub block: Block,
    pub parent_block_header: BlockHeader,
    pub db: ExecutionDB,
}

pub fn load_cache(block_number: usize) -> Result<Cache, String> {
    let file_name = format!("cache_{}.bin", block_number);
    let file = BufReader::new(File::open(file_name).map_err(|err| err.to_string())?);
    serde_json::from_reader(file).map_err(|err| err.to_string())
}

pub fn write_cache(cache: &Cache) -> Result<(), String> {
    let file_name = format!("cache_{}.bin", cache.block.header.number);
    let mut file = BufWriter::new(File::open(file_name).map_err(|err| err.to_string())?);
    serde_json::to_writer(file, cache).map_err(|err| err.to_string())
}
