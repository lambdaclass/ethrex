use bytes::Bytes;
use ethrex::{
    cli::{import_blocks, import_blocks_bench},
    decode::chain_file,
    initializers::{init_store, load_store},
};
use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    error::ChainError,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    H160, H256,
    types::{Block, BlockHeader, DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER, Genesis},
};
use ethrex_rlp::encode::RLPEncode as _;
use ethrex_storage::{EngineType, Store};
use std::{
    fs::File,
    path::{Path, PathBuf},
};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn genesis_fixture(name: &str) -> Genesis {
    let path = workspace_root().join("fixtures/genesis").join(name);
    serde_json::from_reader(File::open(path).unwrap()).expect("genesis fixture parses")
}

fn chain_and_genesis() -> (Vec<Block>, Genesis) {
    let root = workspace_root();
    let blocks = chain_file(File::open(root.join("fixtures/blockchain/l2-loadtest.rlp")).unwrap())
        .expect("chain fixture decodes");
    (blocks, genesis_fixture("perf-ci.json"))
}

/// A chain of `len` empty blocks on `genesis`, longer than the fixture chain, for the
/// tests that need a file whose first blocks are imported in a batch.
async fn built_chain(genesis: &Genesis, len: usize) -> Vec<Block> {
    let mut store = Store::new("store.db", EngineType::InMemory).unwrap();
    store.add_initial_state(genesis.clone()).await.unwrap();
    let blockchain = Blockchain::default_with_store(store.clone());
    let mut parent = store.get_block_header(0).unwrap().unwrap();
    let mut blocks = Vec::with_capacity(len);
    for _ in 0..len {
        let args = BuildPayloadArgs {
            parent: parent.hash(),
            timestamp: parent.timestamp + 12,
            fee_recipient: H160::zero(),
            random: H256::zero(),
            withdrawals: Some(Vec::new()),
            beacon_root: Some(H256::zero()),
            slot_number: None,
            version: 1,
            elasticity_multiplier: ELASTICITY_MULTIPLIER,
            gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
        };
        let payload = create_payload(&args, &store, Bytes::new()).unwrap();
        let block = blockchain.build_payload(payload).unwrap().payload;
        blockchain.add_block(block.clone()).unwrap();
        store
            .forkchoice_update(vec![], block.header.number, block.hash(), None, None)
            .await
            .unwrap();
        parent = block.header.clone();
        blocks.push(block);
    }
    blocks
}

/// `block` with a state root that its execution cannot produce. It decodes, but the
/// import rejects it after executing it.
fn with_bad_state_root(block: &Block) -> Block {
    let header = BlockHeader {
        hash: Default::default(),
        state_root: H256::repeat_byte(0xab),
        ..block.header.clone()
    };
    Block::new(header, block.body.clone())
}

/// `block` with a transactions root that its body does not have. The import rejects it
/// before executing it.
fn with_bad_body(block: &Block) -> Block {
    let header = BlockHeader {
        hash: Default::default(),
        transactions_root: H256::repeat_byte(0xcd),
        ..block.header.clone()
    };
    Block::new(header, block.body.clone())
}

/// `block` on a parent that no chain has.
fn with_unknown_parent(block: &Block) -> Block {
    let header = BlockHeader {
        hash: Default::default(),
        parent_hash: H256::repeat_byte(0xef),
        ..block.header.clone()
    };
    Block::new(header, block.body.clone())
}

/// Writes `blocks` to the chain file `name` in `dir`.
fn write_chain_file(dir: &Path, name: &str, blocks: &[Block]) {
    let bytes: Vec<u8> = blocks
        .iter()
        .flat_map(|block| block.encode_to_vec())
        .collect();
    std::fs::write(dir.join(name), bytes).unwrap();
}

async fn import(path: &Path, datadir: &Path, genesis: Genesis) -> Result<(), ChainError> {
    import_blocks(
        path.to_str().unwrap(),
        datadir,
        genesis,
        BlockchainOptions::default(),
    )
    .await
}

/// Asserts that `blocks` are the canonical chain of the store in `datadir`, with the
/// last of them as its head.
async fn assert_canonical_chain(datadir: &Path, blocks: &[Block]) {
    let store = load_store(datadir).await.unwrap();
    let head = blocks.last().expect("a chain to check");
    assert_eq!(store.get_latest_block_number().unwrap(), head.header.number);
    for block in blocks {
        assert_eq!(
            store
                .get_canonical_block_hash(block.header.number)
                .await
                .unwrap(),
            Some(block.hash()),
            "block {} is canonical",
            block.header.number
        );
    }
}

/// A directory import applies the valid files that follow a file it cannot decode and
/// a block it rejects, and still reports failure once every file has been tried.
#[tokio::test]
async fn directory_import_skips_files_that_fail() {
    let (blocks, genesis) = chain_and_genesis();
    let blocks_dir = tempfile::tempdir().unwrap();
    let files: [(&str, Vec<u8>); 5] = [
        ("0001.rlp", blocks[0].encode_to_vec()),
        ("0002.rlp", b"not rlp".to_vec()),
        ("0003.rlp", with_bad_state_root(&blocks[1]).encode_to_vec()),
        ("0004.rlp", blocks[1].encode_to_vec()),
        ("0005.rlp", blocks[2].encode_to_vec()),
    ];
    for (name, bytes) in files {
        std::fs::write(blocks_dir.path().join(name), bytes).unwrap();
    }
    let datadir = tempfile::tempdir().unwrap();

    let err = import_blocks(
        blocks_dir.path().to_str().unwrap(),
        datadir.path(),
        genesis,
        BlockchainOptions::default(),
    )
    .await
    .expect_err("a directory with failing files reports failure");
    assert_eq!(err.to_string(), "2 of 5 files failed to import");

    let store = load_store(datadir.path()).await.unwrap();
    let head = store.get_latest_block_number().unwrap();
    assert_eq!(head, blocks[2].header.number);
    assert_eq!(
        store.get_canonical_block_hash(head).await.unwrap(),
        Some(blocks[2].hash())
    );
}

/// The blocks a file stored before one of its blocks failed become canonical when a
/// later file builds on them, so the canonical chain has no gap below that file.
#[tokio::test]
async fn directory_import_keeps_the_blocks_before_a_failed_block_canonical() {
    let (blocks, genesis) = chain_and_genesis();
    let blocks_dir = tempfile::tempdir().unwrap();
    let mut failing = blocks[..3].to_vec();
    failing.push(with_bad_state_root(&blocks[3]));
    write_chain_file(blocks_dir.path(), "0001.rlp", &failing);
    write_chain_file(blocks_dir.path(), "0002.rlp", &blocks[3..5]);
    let datadir = tempfile::tempdir().unwrap();

    let err = import(blocks_dir.path(), datadir.path(), genesis)
        .await
        .expect_err("a directory with a failing file reports failure");
    assert_eq!(err.to_string(), "1 of 2 files failed to import");

    assert_canonical_chain(datadir.path(), &blocks[..5]).await;
}

/// The same holds when the block that fails is among the first blocks of a long file,
/// which are imported in a batch.
#[tokio::test]
async fn directory_import_keeps_the_blocks_before_a_failed_block_of_a_batch_canonical() {
    let genesis = genesis_fixture("execution-api.json");
    let blocks = built_chain(&genesis, 140).await;
    let blocks_dir = tempfile::tempdir().unwrap();
    let mut failing = blocks.clone();
    failing[4] = with_bad_state_root(&blocks[4]);
    write_chain_file(blocks_dir.path(), "0001.rlp", &failing);
    write_chain_file(blocks_dir.path(), "0002.rlp", &blocks[4..]);
    let datadir = tempfile::tempdir().unwrap();

    let err = import(blocks_dir.path(), datadir.path(), genesis)
        .await
        .expect_err("a directory with a failing file reports failure");
    assert_eq!(err.to_string(), "1 of 2 files failed to import");

    assert_canonical_chain(datadir.path(), &blocks).await;
}

/// A block that is rejected before it is executed, here for its body, does not drop
/// the blocks of the batch that were queued ahead of it: they are stored, and a later
/// file builds on them.
#[tokio::test]
async fn directory_import_stores_the_blocks_queued_before_a_block_it_rejects() {
    let genesis = genesis_fixture("execution-api.json");
    let blocks = built_chain(&genesis, 140).await;
    let blocks_dir = tempfile::tempdir().unwrap();
    let mut failing = blocks.clone();
    failing[4] = with_bad_body(&blocks[4]);
    write_chain_file(blocks_dir.path(), "0001.rlp", &failing);
    write_chain_file(blocks_dir.path(), "0002.rlp", &blocks[4..]);
    let datadir = tempfile::tempdir().unwrap();

    let err = import(blocks_dir.path(), datadir.path(), genesis)
        .await
        .expect_err("a directory with a failing file reports failure");
    assert_eq!(err.to_string(), "1 of 2 files failed to import");

    assert_canonical_chain(datadir.path(), &blocks).await;
}

/// A file that fails leaves the head where it was, also when its first blocks were
/// already in the store.
#[tokio::test]
async fn directory_import_leaves_the_head_when_a_failed_file_stored_nothing() {
    let (blocks, genesis) = chain_and_genesis();
    let blocks_dir = tempfile::tempdir().unwrap();
    write_chain_file(blocks_dir.path(), "0001.rlp", &blocks[..3]);
    write_chain_file(
        blocks_dir.path(),
        "0002.rlp",
        &[blocks[0].clone(), with_bad_state_root(&blocks[1])],
    );
    let datadir = tempfile::tempdir().unwrap();

    let err = import(blocks_dir.path(), datadir.path(), genesis)
        .await
        .expect_err("a directory with a failing file reports failure");
    assert_eq!(err.to_string(), "1 of 2 files failed to import");

    assert_canonical_chain(datadir.path(), &blocks[..3]).await;
}

/// A database failure, unlike a block the import rejects, stops a directory import:
/// every remaining file would fail the same way.
#[tokio::test]
async fn directory_import_stops_on_a_database_failure() {
    let (blocks, genesis) = chain_and_genesis();
    let datadir = tempfile::tempdir().unwrap();
    {
        // The first four blocks without their execution: the store knows the blocks
        // but not the state of block 4, the first one to change it.
        let store = init_store(datadir.path(), genesis.clone()).await.unwrap();
        store.add_blocks(blocks[..4].to_vec()).await.unwrap();
    }
    let blocks_dir = tempfile::tempdir().unwrap();
    write_chain_file(blocks_dir.path(), "0005.rlp", &blocks[4..5]);
    write_chain_file(blocks_dir.path(), "0006.rlp", &blocks[5..6]);

    let err = import(blocks_dir.path(), datadir.path(), genesis)
        .await
        .expect_err("a missing parent state fails the import");
    assert!(
        err.to_string().contains("state root missing for block 4"),
        "unexpected error: {err}"
    );

    // The import stopped there: the next file was not tried.
    let store = load_store(datadir.path()).await.unwrap();
    assert_eq!(
        store.get_block_number(blocks[5].hash()).await.unwrap(),
        None
    );
}

/// The files of another network all fail, since no block of theirs builds on the
/// genesis block, and none of their blocks is kept in the store as a pending block.
#[tokio::test]
async fn directory_import_of_another_network_keeps_no_pending_blocks() {
    let (blocks, mut genesis) = chain_and_genesis();
    genesis.nonce += 1;
    let blocks_dir = tempfile::tempdir().unwrap();
    for (name, block) in [("0001.rlp", 0), ("0002.rlp", 1), ("0003.rlp", 2)] {
        write_chain_file(blocks_dir.path(), name, &blocks[block..=block]);
    }
    let datadir = tempfile::tempdir().unwrap();

    let err = import(blocks_dir.path(), datadir.path(), genesis)
        .await
        .expect_err("a chain of another network fails the import");
    assert_eq!(err.to_string(), "3 of 3 files failed to import");

    let store = load_store(datadir.path()).await.unwrap();
    assert_eq!(store.get_latest_block_number().unwrap(), 0);
    for block in &blocks[..3] {
        assert!(
            store
                .get_pending_block(block.hash())
                .await
                .unwrap()
                .is_none(),
            "block {} was not kept as pending",
            block.header.number
        );
    }
}

/// A first block on an unknown parent is one block the import rejects, not a sign that
/// every file is of another network: the files after it still import.
#[tokio::test]
async fn directory_import_continues_past_a_first_block_on_an_unknown_parent() {
    let (blocks, genesis) = chain_and_genesis();
    let blocks_dir = tempfile::tempdir().unwrap();
    let rejected = with_unknown_parent(&blocks[0]);
    write_chain_file(
        blocks_dir.path(),
        "0001.rlp",
        std::slice::from_ref(&rejected),
    );
    write_chain_file(blocks_dir.path(), "0002.rlp", &blocks[..1]);
    write_chain_file(blocks_dir.path(), "0003.rlp", &blocks[1..2]);
    let datadir = tempfile::tempdir().unwrap();

    let err = import(blocks_dir.path(), datadir.path(), genesis)
        .await
        .expect_err("a directory with a failing file reports failure");
    assert_eq!(err.to_string(), "1 of 3 files failed to import");

    assert_canonical_chain(datadir.path(), &blocks[..2]).await;
    let store = load_store(datadir.path()).await.unwrap();
    assert!(
        store
            .get_pending_block(rejected.hash())
            .await
            .unwrap()
            .is_none(),
        "the rejected block was not kept as pending"
    );
}

/// A directory import reads the chain files only, not the hidden files and the
/// subdirectories that sit next to them.
#[tokio::test]
async fn directory_import_reads_only_the_chain_files() {
    let (blocks, genesis) = chain_and_genesis();
    let blocks_dir = tempfile::tempdir().unwrap();
    write_chain_file(blocks_dir.path(), "0001.rlp", &blocks[..1]);
    std::fs::write(blocks_dir.path().join(".DS_Store"), b"not rlp").unwrap();
    std::fs::create_dir(blocks_dir.path().join("0002.rlp")).unwrap();
    let datadir = tempfile::tempdir().unwrap();

    import(blocks_dir.path(), datadir.path(), genesis)
        .await
        .expect("the only chain file imports");

    assert_canonical_chain(datadir.path(), &blocks[..1]).await;
}

/// A single file import stops at the first failure and leaves the head where it was.
/// The blocks before the failure stay in the store.
#[tokio::test]
async fn file_import_stops_at_a_block_that_fails() {
    let (blocks, genesis) = chain_and_genesis();
    let blocks_dir = tempfile::tempdir().unwrap();
    write_chain_file(
        blocks_dir.path(),
        "chain.rlp",
        &[
            blocks[0].clone(),
            blocks[1].clone(),
            with_bad_state_root(&blocks[2]),
            blocks[2].clone(),
        ],
    );
    let datadir = tempfile::tempdir().unwrap();

    import(
        &blocks_dir.path().join("chain.rlp"),
        datadir.path(),
        genesis,
    )
    .await
    .expect_err("a file with a block that fails reports failure");

    let store = load_store(datadir.path()).await.unwrap();
    assert_eq!(store.get_latest_block_number().unwrap(), 0);
    for block in &blocks[..2] {
        assert_eq!(
            store.get_block_number(block.hash()).await.unwrap(),
            Some(block.header.number),
            "block {} is in the store",
            block.header.number
        );
    }
}

/// A single file import fails on a file it cannot decode.
#[tokio::test]
async fn file_import_fails_on_undecodable_file() {
    let (_, genesis) = chain_and_genesis();
    let blocks_dir = tempfile::tempdir().unwrap();
    let file = blocks_dir.path().join("0001.rlp");
    std::fs::write(&file, b"not rlp").unwrap();
    let datadir = tempfile::tempdir().unwrap();

    let err = import(&file, datadir.path(), genesis)
        .await
        .expect_err("an undecodable file fails the import");
    assert!(
        err.to_string().starts_with("failed to read chain file"),
        "unexpected error: {err}"
    );
}

/// `import-bench` reads the files of a directory in the same order as `import`.
#[tokio::test]
async fn import_bench_reads_numbered_files_in_numeric_order() {
    let (blocks, genesis) = chain_and_genesis();
    let blocks_dir = tempfile::tempdir().unwrap();
    write_chain_file(blocks_dir.path(), "2.rlp", &blocks[..1]);
    write_chain_file(blocks_dir.path(), "10.rlp", &blocks[1..2]);
    let datadir = tempfile::tempdir().unwrap();

    import_blocks_bench(
        blocks_dir.path().to_str().unwrap(),
        datadir.path(),
        genesis,
        BlockchainOptions::default(),
        None,
        None,
    )
    .await
    .expect("the files import in numeric order");

    assert_canonical_chain(datadir.path(), &blocks[..2]).await;
}
