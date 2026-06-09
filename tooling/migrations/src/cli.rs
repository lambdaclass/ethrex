use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use clap::{Parser as ClapParser, Subcommand as ClapSubcommand};
use ethrex_blockchain::{Blockchain, BlockchainOptions, BlockchainType, L2Config};
use ethrex_common::types::Block;
use tracing::info;

use crate::utils::{migrate_block_body, migrate_block_header};

/// Minimum interval between migration progress log lines.
const PROGRESS_LOG_INTERVAL: Duration = Duration::from_secs(10);

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

#[derive(ClapSubcommand)]
pub enum Subcommand {
    #[command(
        name = "libmdbx2rocksdb",
        visible_alias = "l2r",
        about = "Migrate a libmdbx database to rocksdb"
    )]
    Libmdbx2Rocksdb {
        #[arg(long = "genesis")]
        /// Path to the genesis file for the old database
        genesis_path: PathBuf,
        #[arg(long = "store.old")]
        /// Path to the target Libmbdx database to migrate
        old_storage_path: PathBuf,
        #[arg(long = "store.new")]
        /// Path for the new RocksDB database
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
    genesis_path: &Path,
    old_storage_path: &Path,
    new_storage_path: &Path,
) {
    let old_store = ethrex_storage_libmdbx::Store::new(
        old_storage_path.to_str().expect("Invalid old storage path"),
        ethrex_storage_libmdbx::EngineType::Libmdbx,
    )
    .expect("Cannot open libmdbx store");
    old_store
        .load_initial_state()
        .await
        .expect("Cannot load libmdbx store state");

    let new_store = ethrex_storage::Store::new_from_genesis(
        new_storage_path,
        ethrex_storage::EngineType::RocksDB,
        genesis_path
            .to_str()
            .expect("Cannot convert genesis path to str"),
    )
    .await
    .expect("Cannot create rocksdb store");

    let last_block_number = old_store
        .get_latest_block_number()
        .await
        .expect("Cannot get latest block from libmdbx store");
    let last_known_block = new_store
        .get_latest_block_number()
        .await
        .expect("Cannot get latest known block from rocksdb store");

    if last_known_block >= last_block_number {
        info!("RocksDB store is already up to date (latest block {last_known_block})");
        return;
    }

    let total_blocks = last_block_number - last_known_block;
    info!(
        "Migrating {total_blocks} blocks ({} to {last_block_number}) from libmdbx to RocksDB",
        last_known_block + 1
    );

    let blockchain_opts = BlockchainOptions {
        // TODO: we may want to migrate using a specified fee config
        r#type: BlockchainType::L2(L2Config::default()),
        ..Default::default()
    };
    let blockchain = Blockchain::new(new_store.clone(), blockchain_opts);

    let block_bodies = old_store
        .get_block_bodies(last_known_block + 1, last_block_number)
        .await
        .expect("Cannot get bodies from libmdbx store");

    let block_headers = (last_known_block + 1..=last_block_number).map(|i| {
        old_store
            .get_block_header(i)
            .ok()
            .flatten()
            .expect("Cannot get block headers from libmdbx store")
    });

    let blocks = block_headers.zip(block_bodies);
    let mut added_blocks = Vec::new();
    let start = Instant::now();
    let mut last_progress_log = Instant::now();
    for (header, body) in blocks {
        let header = migrate_block_header(header);
        let body = migrate_block_body(body);
        let block_number = header.number;
        let block = Block::new(header, body);

        let block_hash = block.hash();
        blockchain
            .add_block_pipeline(block, None)
            .unwrap_or_else(|e| panic!("Cannot add block {block_number} to rocksdb store: {e}"));
        added_blocks.push((block_number, block_hash));

        if last_progress_log.elapsed() >= PROGRESS_LOG_INTERVAL {
            let migrated = added_blocks.len() as u64;
            let rate = migrated as f64 / start.elapsed().as_secs_f64();
            info!(
                "Migrated {migrated}/{total_blocks} blocks ({:.1}%), currently at block {block_number} ({rate:.0} blocks/s)",
                migrated as f64 * 100.0 / total_blocks as f64
            );
            last_progress_log = Instant::now();
        }
    }

    let migrated_blocks = added_blocks.len();
    let last_block = old_store
        .get_block_header(last_block_number)
        .ok()
        .flatten()
        .expect("Cannot get last block from libmdbx store");
    new_store
        .forkchoice_update(
            added_blocks,
            last_block.number,
            last_block.hash(),
            None,
            None,
        )
        .await
        .expect("Cannot apply forkchoice update");

    info!(
        "Migration complete: {migrated_blocks} blocks migrated in {:.1}s, head is now block {last_block_number}",
        start.elapsed().as_secs_f64()
    );
}
