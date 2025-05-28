use std::{
    fs::File,
    io::{BufReader, BufWriter},
};

use ethrex_common::types::{Block, BlockHeader};
use ethrex_vm::ProverDB;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct Cache {
    pub blocks: Vec<Block>,
    pub parent_block_header: BlockHeader,
    pub db: ProverDB,
}

impl Cache {
    pub fn merge(mut self, other: Self) -> Self {
        self.blocks.extend(other.blocks);
        let ProverDB {
            accounts,
            code,
            storage,
            block_hashes,
            chain_config: _chain_config,
            state_proofs,
            storage_proofs,
        } = other.db;
        for (account, account_info) in accounts {
            self.db.accounts.entry(account).or_insert(account_info);
        }
        for (codehash, code) in code {
            self.db.code.entry(codehash).or_insert(code);
        }
        for (account, other_storage) in storage {
            let storage = self.db.storage.entry(account).or_default();
            for (key, value) in other_storage {
                storage.entry(key).or_insert(value);
            }
        }
        self.db.block_hashes.extend(block_hashes);
        
        // Keep initial root, join and dedup.
        self.db.state_proofs.1.extend(state_proofs.1);
        self.db.state_proofs.1.sort_unstable();
        self.db.state_proofs.1.dedup();

        for (address, proof) in storage_proofs {
            let existing = self.db.storage_proofs.entry(address).or_default();
            // Keep storage root if existing
            existing.0 = existing.0.clone().or(proof.0);
            // Add nodes (nodes from intermediary blocks )
            existing.1.extend(proof.1);
            existing.1.sort_unstable();
            existing.1.dedup();
        }
        self
    }
}

pub fn load_cache(block_number: usize) -> eyre::Result<Cache> {
    let file_name = format!("cache_{}.json", block_number);
    let file = BufReader::new(File::open(file_name)?);
    Ok(serde_json::from_reader(file)?)
}

pub fn write_cache(cache: &Cache) -> eyre::Result<()> {
    if cache.blocks.len() != 1 {
        return Err(eyre::Error::msg("trying to save a multi-block cache"));
    }
    let file_name = format!("cache_{}.json", cache.blocks[0].header.number);
    let file = BufWriter::new(File::create(file_name)?);
    Ok(serde_json::to_writer(file, cache)?)
}
