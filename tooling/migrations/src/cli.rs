use std::path::PathBuf;

use clap::{Parser as ClapParser, Subcommand as ClapSubcommand};
use ethrex_blockchain::Blockchain;
use ethrex_common::types::Block;

use crate::utils::{migrate_block_body, migrate_block_header};

#[allow(clippy::upper_case_acronyms)]
#[derive(ClapParser)]
#[command(
    name = "migrations",
    author = "Lambdaclass",
    about = "ethrex migration tools"
)]
pub struct CLI {
    #[command(subcommand)]
    pub command: Subcommand,
}

#[allow(clippy::large_enum_variant)]
#[derive(ClapSubcommand)]
pub enum Subcommand {
    #[command(
        name = "libmdbx2rocksdb",
        visible_alias = "l2r",
        about = "Migrate a libmdbx database to rocksdb"
    )]
    Libmdbx2Rocksdb {
        #[arg(long = "genesis")]
        genesis_path: PathBuf,
        #[arg(long = "store.old")]
        old_storage_path: PathBuf,
        #[arg(long = "store.new")]
        new_storage_path: PathBuf,
    },
}

impl Subcommand {
    pub async fn run(&self) {
        match self {
            Self::Libmdbx2Rocksdb {
                genesis_path,
                old_storage_path,
                new_storage_path,
            } => migrate_libmdbx_to_rocksdb(genesis_path, old_storage_path, new_storage_path).await,
        }
    }
}

async fn migrate_libmdbx_to_rocksdb(
    genesis_path: &PathBuf,
    old_storage_path: &PathBuf,
    new_storage_path: &PathBuf,
) {
    let old_store = ethrex_storage_libmdbx::Store::new(
        old_storage_path,
        ethrex_storage_libmdbx::EngineType::Libmdbx,
    )
    .unwrap();
    old_store.load_initial_state().await.unwrap();
    let new_store = ethrex_storage::Store::new_from_genesis(
        new_storage_path.as_path(),
        ethrex_storage::EngineType::RocksDB,
        genesis_path.to_str().unwrap(),
    )
    .await
    .unwrap();

    let last_block_number = old_store.get_latest_block_number().await.unwrap();
    let last_known_block = new_store.get_latest_block_number().await.unwrap();

    let blockchain = Blockchain::new(
        new_store.clone(),
        ethrex_blockchain::BlockchainType::L2,
        false,
    );

    println!("Last known number: {last_known_block}");
    println!("Last block number: {last_block_number}");

    let block_bodies = old_store
        .get_block_bodies(last_known_block + 1, last_block_number)
        .await
        .unwrap();

    let block_headers = (last_known_block + 1..=last_block_number)
        .into_iter()
        .map(|i| old_store.get_block_header(i).unwrap().unwrap());

    let blocks = block_headers.zip(block_bodies);
    for (header, body) in blocks {
        let header = migrate_block_header(header);
        let body = migrate_block_body(body);
        blockchain
            .add_block(&Block::new(header, body))
            .await
            .unwrap();
    }

    let last_block = old_store
        .get_block_header(last_block_number)
        .unwrap()
        .unwrap();
    new_store
        .forkchoice_update(None, last_block.number, last_block.hash(), None, None)
        .await
        .unwrap();
}
