use ethrex::{cli::import_blocks, decode::chain_file, initializers::load_store};
use ethrex_blockchain::BlockchainOptions;
use ethrex_common::{
    H256,
    types::{Block, BlockHeader, Genesis},
};
use ethrex_rlp::encode::RLPEncode as _;
use std::{fs::File, path::PathBuf};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn chain_and_genesis() -> (Vec<Block>, Genesis) {
    let root = workspace_root();
    let blocks = chain_file(File::open(root.join("fixtures/blockchain/l2-loadtest.rlp")).unwrap())
        .expect("chain fixture decodes");
    let genesis =
        serde_json::from_reader(File::open(root.join("fixtures/genesis/perf-ci.json")).unwrap())
            .expect("genesis fixture parses");
    (blocks, genesis)
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

/// A single file import stops at the first failure.
#[tokio::test]
async fn file_import_fails_on_undecodable_file() {
    let (_, genesis) = chain_and_genesis();
    let blocks_dir = tempfile::tempdir().unwrap();
    let file = blocks_dir.path().join("0001.rlp");
    std::fs::write(&file, b"not rlp").unwrap();
    let datadir = tempfile::tempdir().unwrap();

    let result = import_blocks(
        file.to_str().unwrap(),
        datadir.path(),
        genesis,
        BlockchainOptions::default(),
    )
    .await;
    assert!(result.is_err(), "an undecodable file fails the import");
}
