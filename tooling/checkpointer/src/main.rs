use std::{fs::remove_dir_all, path::Path, sync::Arc};

use ethrex::{cli::remove_db, initializers::regenerate_head_state, utils::default_datadir};
use ethrex_blockchain::{
    Blockchain, BlockchainOptions, BlockchainType,
    fork_choice::apply_fork_choice,
    payload::{BuildPayloadArgs, create_payload},
    validate_block,
};
use ethrex_common::{
    Address, Bytes, H256, U256,
    types::{
        Block, DEFAULT_BUILDER_GAS_CEIL, EIP1559Transaction, ELASTICITY_MULTIPLIER, Genesis,
        Transaction, TxKind, fee_config::FeeConfig,
    },
};
use ethrex_config::networks::Network;
use ethrex_l2::sequencer::block_producer::build_payload;
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_storage::{EngineType, Store};
use ethrex_storage_rollup::{EngineTypeRollup, StoreRollup};
use ethrex_vm::BlockExecutionResult;

#[tokio::main]
async fn main() {
    let datadir = default_datadir();

    println!("Removing existing database...");

    remove_db(&datadir, true);

    let network = Network::LocalDevnet;

    let genesis = network
        .get_genesis()
        .expect("failed to get genesis from local devnet");

    println!("Creating store and adding genesis state...");

    let mut store = {
        let store_inner =
            Store::new(&datadir, EngineType::RocksDB).expect("failed to create store");

        store_inner
            .add_initial_state(genesis.clone())
            .await
            .expect("failed to add genesis state to store");

        store_inner
    };

    let rollup_store = {
        let rollup_store = StoreRollup::new(&datadir, EngineTypeRollup::InMemory)
            .expect("failed to create StoreRollup");
        rollup_store
            .init()
            .await
            .expect("failed to init rollup store");
        rollup_store
    };

    println!("Creating blockchain instance...");

    let blockchain = Arc::new(Blockchain::new(
        store.clone(),
        BlockchainOptions {
            r#type: BlockchainType::L2(FeeConfig::default()),
            ..Default::default()
        },
    ));

    let mut head_block_hash = genesis.get_block().hash();

    let mut head_block_timestamp = genesis.timestamp;

    for n_blocks in 1..=7200 {
        println!("Testing {n_blocks} blocks batch");

        let checkpoint_datadir = datadir.join(format!("checkpoint_{n_blocks}"));

        // If the checkpoint directory already exists, remove it
        if checkpoint_datadir.exists() {
            println!("Removing existing checkpoint directory at {checkpoint_datadir:?}");

            remove_dir_all(&checkpoint_datadir)
                .expect("failed to remove existing checkpoint directory");
        }

        println!("Creating checkpoint at {checkpoint_datadir:?}");

        let (checkpoint_store, checkpoint_blockchain) =
            create_checkpoint(&store, &checkpoint_datadir, genesis.clone()).await;

        println!("Producing {n_blocks} blocks...");

        // This block building represents the sequencer building blocks
        let blocks = build_l2_blocks(
            blockchain.clone(),
            &mut store,
            &rollup_store,
            head_block_hash,
            head_block_timestamp + 12,
            n_blocks,
        )
        .await;

        head_block_hash = blocks.last().expect("no blocks built").hash();

        head_block_timestamp = blocks.last().expect("no blocks built").header.timestamp;

        println!(
            "Checkpoint head: {}, Store head: {}",
            checkpoint_store
                .get_latest_block_number()
                .await
                .expect("failed to get latest block number from checkpoint store"),
            store
                .get_latest_block_number()
                .await
                .expect("failed to get latest block number from main store"),
        );

        println!("Generating witnesses for built blocks...");

        checkpoint_blockchain
            .generate_witness_for_blocks(&blocks)
            .await
            .expect("failed to generate witness for blocks");

        println!("Removing checkpoint directory...");

        // The checkpoint has fulfilled its purpose, so we can remove it to
        // avoid wasting disk space
        // Ideally, we should keep the batch until it is verified in the L1,
        // just in case something goes wrong and we need to regenerate it.
        remove_dir_all(&checkpoint_datadir).expect("failed to remove checkpoint directory");

        println!("Done!");
    }
}

/// Produces L2 blocks populated with a single signed transaction each.
pub async fn build_l2_blocks(
    blockchain: Arc<Blockchain>,
    store: &mut Store,
    rollup_store: &StoreRollup,
    head_block_hash: H256,
    initial_timestamp: u64,
    n_blocks: u64,
) -> Vec<Block> {
    let mut blocks = Vec::new();

    let mut current_parent_hash = head_block_hash;

    let mut current_timestamp = initial_timestamp;

    let mut last_privilege_nonce = None;

    for _ in 0..n_blocks {
        let tx = generate_signed_transaction(store).await;

        blockchain
            .add_transaction_to_pool(tx)
            .await
            .expect("failed to add tx to pool");

        let block = build_empty_l2_block(
            blockchain.clone(),
            store,
            rollup_store,
            current_parent_hash,
            current_timestamp,
            &mut last_privilege_nonce,
        )
        .await;

        current_parent_hash = block.hash();

        current_timestamp += 12; // Assuming an average block time of 12 seconds

        blocks.push(block);
    }

    blocks
}

/// Produces a single empty L2 block.
pub async fn build_empty_l2_block(
    blockchain: Arc<Blockchain>,
    store: &mut Store,
    rollup_store: &StoreRollup,
    head_block_hash: H256,
    timestamp: u64,
    last_privilege_nonce: &mut Option<u64>,
) -> Block {
    let build_payload_args = BuildPayloadArgs {
        parent: head_block_hash,
        timestamp,
        fee_recipient: Address::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        version: 3,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };

    let payload =
        create_payload(&build_payload_args, store, Bytes::new()).expect("failed to create payload");

    let payload_build_result = build_payload(
        blockchain.clone(),
        payload,
        store,
        last_privilege_nonce,
        DEFAULT_BUILDER_GAS_CEIL,
    )
    .await
    .expect("failed to build payload");

    let new_block = payload_build_result.payload;

    let chain_config = store
        .get_chain_config()
        .expect("failed to get chain config");

    validate_block(
        &new_block,
        &store
            .get_block_header_by_hash(new_block.header.parent_hash)
            .expect("failed to get parent block header")
            .expect("parent block not found"),
        &chain_config,
        build_payload_args.elasticity_multiplier,
    )
    .expect("failed to validate block");

    let account_updates = payload_build_result.account_updates;

    let execution_result = BlockExecutionResult {
        receipts: payload_build_result.receipts,
        requests: Vec::new(),
    };

    let account_updates_list = store
        .apply_account_updates_batch(new_block.header.parent_hash, &account_updates)
        .await
        .expect("failed to apply account updates")
        .expect("no account updates returned");

    blockchain
        .store_block(new_block.clone(), account_updates_list, execution_result)
        .await
        .expect("failed to store block");

    rollup_store
        .store_account_updates_by_block_number(new_block.header.number, account_updates)
        .await
        .expect("failed to store account updates in rollup store");

    let new_block_hash = new_block.hash();

    apply_fork_choice(store, new_block_hash, new_block_hash, new_block_hash)
        .await
        .expect("failed to apply fork choice");

    new_block
}

async fn generate_signed_transaction(store: &Store) -> Transaction {
    // 0x941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e is the
    // private key for address 0x4417092b70a3e5f10dc504d0947dd256b965fc62, a
    // pre-funded account in the local devnet genesis.
    let signer = Signer::Local(LocalSigner::new(
        "941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e"
            .parse()
            .expect("invalid private key"),
    ));

    let current_block_number = store
        .get_latest_block_number()
        .await
        .expect("failed to get latest block header");

    Transaction::EIP1559Transaction(EIP1559Transaction {
        nonce: store
            .get_account_info(current_block_number, signer.address())
            .await
            .expect("failed to get account info")
            .expect("account not found")
            .nonce,
        value: U256::one(),
        gas_limit: 250000,
        max_fee_per_gas: u64::MAX,
        max_priority_fee_per_gas: 10,
        chain_id: store
            .get_chain_config()
            .expect("failed to get chain config")
            .chain_id,
        to: TxKind::Call(Address::random()),
        ..Default::default()
    })
    .sign(&signer)
    .await
    .expect("failed to sign transaction")
}

async fn create_checkpoint(
    store: &Store,
    path: &Path,
    genesis: Genesis,
) -> (Store, Arc<Blockchain>) {
    store
        .create_checkpoint(path)
        .await
        .expect("failed to create checkpoint");

    let checkpoint_store = {
        let checkpoint_store_inner =
            Store::new(path, EngineType::RocksDB).expect("failed to create store");

        checkpoint_store_inner
            .add_initial_state(genesis.clone())
            .await
            .expect("failed to add genesis state to store");

        checkpoint_store_inner
    };

    let checkpoint_blockchain = Arc::new(Blockchain::new(
        checkpoint_store.clone(),
        BlockchainOptions {
            r#type: BlockchainType::L2(FeeConfig::default()),
            ..Default::default()
        },
    ));

    let checkpoint_head_block_number = checkpoint_store
        .get_latest_block_number()
        .await
        .expect("failed to get latest block number from checkpoint store");

    let db_head_block_number = store
        .get_latest_block_number()
        .await
        .expect("failed to get latest block number from main store");

    assert_eq!(
        checkpoint_head_block_number, db_head_block_number,
        "checkpoint store head block number does not match main store head block number before regeneration"
    );

    regenerate_head_state(&checkpoint_store, &checkpoint_blockchain)
        .await
        .expect("failed to regenerate head state in checkpoint store");

    let checkpoint_latest_block_number = checkpoint_store
        .get_latest_block_number()
        .await
        .expect("failed to get latest block number from checkpoint store");

    let db_latest_block_number = store
        .get_latest_block_number()
        .await
        .expect("failed to get latest block number from main store");

    let checkpoint_latest_block = checkpoint_store
        .get_block_by_number(checkpoint_latest_block_number)
        .await
        .expect("failed to get latest block from checkpoint store")
        .expect("latest block not found in checkpoint store");

    let db_latest_block = store
        .get_block_by_number(db_latest_block_number)
        .await
        .expect("failed to get latest block from main store")
        .expect("latest block not found in main store");

    // Final sanity check
    assert!(
        checkpoint_store
            .has_state_root(checkpoint_latest_block.header.state_root)
            .expect("failed to check state root in checkpoint store"),
        "checkpoint store state is not regenerated properly"
    );
    assert_eq!(
        checkpoint_latest_block_number, db_latest_block_number,
        "latest block numbers do not match after populating checkpoint store"
    );
    assert_eq!(
        checkpoint_latest_block.hash(),
        db_latest_block.hash(),
        "latest block hashes do not match after populating checkpoint store"
    );

    (checkpoint_store, checkpoint_blockchain)
}
