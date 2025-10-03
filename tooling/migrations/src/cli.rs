use std::path::{Path, PathBuf};

use clap::{Parser as ClapParser, Subcommand as ClapSubcommand};
use ethrex_blockchain::{Blockchain, BlockchainOptions, BlockchainType};
use ethrex_common::types::Block;
use ethrex_storage_rollup::SQLStore;
use libsql::params::IntoParams;

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
    #[command(
        name = "rollup-store.migrate",
        visible_alias = "rsm",
        about = "Migrate rollup store's SQL tables"
    )]
    RollupStoreMigrate {
        #[arg()]
        /// Version to upgrade/downgrade to
        version: Option<u64>,
        #[arg(long)]
        /// Path to the network's datadir
        datadir: PathBuf,
    },
}

impl Subcommand {
    pub async fn run(self) {
        match self {
            Self::Libmdbx2Rocksdb {
                genesis_path,
                old_storage_path,
                new_storage_path,
            } => {
                migrate_libmdbx_to_rocksdb(genesis_path, old_storage_path, new_storage_path).await;
            }
            Self::RollupStoreMigrate { version, datadir } => {
                migrate_rollup_store(version, datadir).await;
            }
        };
    }
}

async fn migrate_libmdbx_to_rocksdb(
    genesis_path: PathBuf,
    old_storage_path: PathBuf,
    new_storage_path: PathBuf,
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
        new_storage_path.as_path(),
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
        println!("Rocksdb store is already up to date");
        return;
    }

    println!("Migrating from block {last_known_block} to {last_block_number}");

    let blockchain_opts = BlockchainOptions {
        r#type: BlockchainType::L2,
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
    for (header, body) in blocks {
        let header = migrate_block_header(header);
        let body = migrate_block_body(body);
        let block_number = header.number;
        let block = Block::new(header, body);

        blockchain
            .add_block(&block)
            .await
            .unwrap_or_else(|e| panic!("Cannot add block {block_number} to rocksdb store: {e}"));
        added_blocks.push((block.header.number, block.hash()));
    }

    let last_block = old_store
        .get_block_header(last_block_number)
        .ok()
        .flatten()
        .expect("Cannot get last block from libmdbx store");
    new_store
        .forkchoice_update(
            Some(added_blocks),
            last_block.number,
            last_block.hash(),
            None,
            None,
        )
        .await
        .expect("Cannot apply forkchoice update");
}

async fn migrate_rollup_store(new_version: Option<u64>, datadir: PathBuf) {
    let store =
        SQLStore::new(&datadir.join("rollup_store"), false).expect("Failed to initiate store");

    let current_version = store.get_version().await.unwrap_or(0);

    let new_version = new_version.unwrap_or(ethrex_storage_rollup::MIGRATION_VERSION);

    if new_version == current_version {
        println!("Database is already in desired version");
        return;
    }

    let script_name = if new_version > current_version {
        "migrate.sql"
    } else {
        "revert.sql"
    };

    let mut migrations: Vec<Vec<String>> =
        Vec::with_capacity((new_version - current_version) as usize);
    for version in current_version + 1..=new_version {
        let path = Path::new("./rollup_store_migrations")
            .join(format!("v{version}"))
            .join(script_name);
        let sql_script = std::fs::read_to_string(path)
            .expect(&format!("Error reading migration for version {version}"));

        let instructions: Vec<String> = sql_script
            .split(";")
            .filter_map(|instruction| {
                if instruction.trim().is_empty() {
                    return None;
                }
                Some(instruction.trim().to_string())
            })
            .collect();

        migrations.push(instructions);
    }

    let empty_params = ().into_params().expect("Unexpected error");
    if new_version < current_version {
        migrations.reverse();
    }

    store
        .execute_in_tx(
            migrations
                .iter()
                .flatten()
                .map(|migration| (migration.as_str(), empty_params.clone()))
                .collect(),
            None,
        )
        .await
        .expect(&format!("Error migrating to version {new_version}"))
}
