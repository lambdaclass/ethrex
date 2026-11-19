//! Replays daisugi-devnet-0 from genesis through the block import path a
//! `newPayload` takes: header and body checks, EIP-8288's dependency digest and
//! proof, execution of every transaction (frame transactions included) and the
//! state, receipts and gas each header commits to.
//!
//! The blocks come from the network itself. `scripts/daisugi-devnet-0/export_blocks.py`
//! rebuilds them from the public RPC, checking every header and transaction hash,
//! and writes one hex-encoded block per line:
//!
//! ```text
//! python3 -I scripts/daisugi-devnet-0/export_blocks.py /tmp/daisugi-blocks.hex
//! DAISUGI_DEVNET_BLOCKS=/tmp/daisugi-blocks.hex \
//!     cargo test --release -p ethrex-test --features leanvm -- --ignored daisugi_devnet_replay
//! ```
//!
//! Without the `leanvm` feature the replay stops at the first block carrying a
//! proof, which this build cannot verify.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use ethrex_blockchain::Blockchain;
use ethrex_common::types::{Block, Genesis, Transaction};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_storage::{EngineType, Store};
use tokio_util::sync::CancellationToken;

/// The exported blocks, decoded, with the bytes each came from.
fn exported_blocks() -> Vec<(Block, Vec<u8>)> {
    let blocks = std::env::var("DAISUGI_DEVNET_BLOCKS")
        .expect("set DAISUGI_DEVNET_BLOCKS to a file written by export_blocks.py");
    let reader = BufReader::new(std::fs::File::open(&blocks).expect("open the exported blocks"));
    reader
        .lines()
        .map(|line| {
            let bytes = hex::decode(line.expect("read a block line").trim()).expect("hex block");
            (Block::decode(&bytes).expect("decode the block"), bytes)
        })
        .collect()
}

async fn genesis_store() -> Store {
    let genesis_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/genesis/daisugi-devnet-0.json");
    let genesis = Genesis::try_from(genesis_path.as_path()).expect("the devnet genesis loads");
    let mut store = Store::new("store.db", EngineType::InMemory).expect("in-memory store");
    store
        .add_initial_state(genesis)
        .await
        .expect("genesis state");
    store
}

/// Full sync imports the same chain in batches through `add_blocks_in_batch`, and
/// every block it stores reads back as the bytes the network produced, proof
/// included: that is what this node serves to peers and over `debug_getRawBlock`.
#[tokio::test]
#[ignore = "needs blocks exported from the network; set DAISUGI_DEVNET_BLOCKS"]
async fn daisugi_devnet_full_sync_batches_store_the_network_bytes() {
    let blocks = exported_blocks();
    let store = genesis_store().await;
    let blockchain = Blockchain::default_with_store(store.clone());
    for batch in blocks.chunks(512) {
        let (last, _) = batch.last().expect("non-empty batch");
        let (number, hash) = (last.header.number, last.hash());
        blockchain
            .add_blocks_in_batch(
                batch.iter().map(|(block, _)| block.clone()).collect(),
                &[],
                CancellationToken::new(),
            )
            .await
            .unwrap_or_else(|(err, failure)| {
                panic!("batch ending at {number} failed: {err} ({failure:?})")
            });
        store
            .forkchoice_update(vec![], number, hash, None, None)
            .await
            .expect("advance the head");
    }
    let mut proofs = 0u64;
    for (block, bytes) in &blocks {
        let stored = store
            .get_block_by_hash(block.hash())
            .await
            .expect("read the block")
            .expect("the block is stored");
        assert_eq!(
            stored.encode_to_vec(),
            *bytes,
            "block {} reads back differently",
            block.header.number
        );
        proofs += u64::from(
            stored
                .body
                .stark_proof
                .as_ref()
                .is_some_and(|p| !p.is_empty()),
        );
    }
    eprintln!(
        "full sync stored {} blocks, {proofs} with proofs, byte for byte",
        blocks.len()
    );
}

#[tokio::test]
#[ignore = "needs blocks exported from the network; set DAISUGI_DEVNET_BLOCKS"]
async fn daisugi_devnet_replay_from_genesis() {
    let blocks = std::env::var("DAISUGI_DEVNET_BLOCKS")
        .expect("set DAISUGI_DEVNET_BLOCKS to a file written by export_blocks.py");
    let genesis_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/genesis/daisugi-devnet-0.json");
    let genesis = Genesis::try_from(genesis_path.as_path()).expect("the devnet genesis loads");

    let mut store = Store::new("store.db", EngineType::InMemory).expect("in-memory store");
    store
        .add_initial_state(genesis)
        .await
        .expect("genesis state");
    let blockchain = Blockchain::default_with_store(store.clone());

    let (mut imported, mut frame_transactions, mut proofs) = (0u64, 0u64, 0u64);
    let reader = BufReader::new(std::fs::File::open(&blocks).expect("open the exported blocks"));
    for line in reader.lines() {
        let line = line.expect("read a block line");
        let bytes = hex::decode(line.trim()).expect("hex block");
        let block = Block::decode(&bytes).expect("decode the block");
        let (number, hash) = (block.header.number, block.hash());
        frame_transactions += block
            .body
            .transactions
            .iter()
            .filter(|tx| matches!(tx, Transaction::FrameTransaction(_)))
            .count() as u64;
        proofs += u64::from(
            block
                .body
                .stark_proof
                .as_ref()
                .is_some_and(|p| !p.is_empty()),
        );

        blockchain
            .add_block_pipeline(block, None)
            .unwrap_or_else(|err| panic!("block {number} ({hash:#x}) failed to import: {err}"));
        store
            .forkchoice_update(vec![], number, hash, None, None)
            .await
            .expect("advance the head");
        imported += 1;
    }
    assert!(imported > 0, "the export holds no blocks");
    eprintln!(
        "replayed {imported} blocks: {frame_transactions} frame transactions, {proofs} proofs verified"
    );
}
