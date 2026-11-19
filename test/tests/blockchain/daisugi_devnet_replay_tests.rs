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
use ethrex_rlp::decode::RLPDecode;
use ethrex_storage::{EngineType, Store};

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
