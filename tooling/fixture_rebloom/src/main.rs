//! One-off maintenance tool that recomputes the header `logs_bloom` for an
//! ethrex-generated block fixture and rewrites it as a consensus-valid chain.
//!
//! Fixtures produced by older ethrex versions (and by `ethrex export` from a DB
//! whose stored headers predate header-bloom population) carry a zero
//! `logs_bloom` even for blocks that emit logs. PR #6766 wired
//! `validate_receipts_root_and_logs_bloom` into every import path, so importing
//! such a fixture now fails with `LogsBloomMismatch`.
//!
//! This tool re-executes each block against a fresh in-memory state built from
//! the given genesis, computes the correct aggregate bloom from the executed
//! receipts, writes it into the header, and re-links every block's
//! `parent_hash` to the corrected predecessor hash — changing a bloom changes
//! the header hash, so the chain must be re-linked from the first non-empty
//! block onward. The output is a fully valid chain that passes every import
//! check (each block is also re-added through the normal validating path, so a
//! successful run proves the rewritten chain is importable).
//!
//! Usage:
//!   cargo run --release -p fixture_rebloom -- <genesis.json> <input.rlp> <output.rlp>

use std::fs::File;
use std::io::{BufReader, Write};
use std::path::Path;

use ethrex::initializers::{init_blockchain, init_store};
use ethrex::utils::read_chain_file;
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_blockchain::{BlockchainOptions, BlockchainType, find_parent_header};
use ethrex_common::H256;
use ethrex_common::types::{Genesis, compute_receipts_root_and_logs_bloom};
use ethrex_crypto::NativeCrypto;
use ethrex_rlp::encode::RLPEncode;
use eyre::WrapErr;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eyre::bail!("usage: fixture_rebloom <genesis.json> <input.rlp> <output.rlp>");
    }
    let genesis_path = &args[1];
    let input_path = &args[2];
    let output_path = &args[3];

    let genesis: Genesis = serde_json::from_reader(BufReader::new(
        File::open(genesis_path).wrap_err("failed to open genesis file")?,
    ))
    .wrap_err("failed to parse genesis file")?;

    // Fresh in-memory chain seeded with the genesis state.
    let store = init_store(Path::new("memory"), genesis)
        .await
        .wrap_err("failed to init in-memory store")?;
    let blockchain = init_blockchain(
        store.clone(),
        BlockchainOptions {
            r#type: BlockchainType::L1,
            ..Default::default()
        },
    );

    let blocks = read_chain_file(input_path);
    println!("read {} blocks from {input_path}", blocks.len());

    let mut out = File::create(output_path).wrap_err("failed to create output file")?;
    let mut buf = Vec::new();

    // The first block's parent is genesis (whose bloom is empty and correct, so
    // its hash is unchanged); from there each block is re-linked to the
    // corrected predecessor hash.
    let mut corrected_parent: Option<H256> = None;
    let mut rebloomed = 0usize;

    for (index, mut block) in blocks.into_iter().enumerate() {
        let number = block.header.number;

        if let Some(parent) = corrected_parent {
            block.header.parent_hash = parent;
        }

        // Execute against the current state to obtain the receipts WITHOUT
        // running post-execution validation: the stored bloom is wrong, so the
        // normal import path would reject the block before returning receipts.
        let parent_header = find_parent_header(&block.header, &store)
            .wrap_err_with(|| format!("parent not found for block {number}"))?;
        let vm_db = StoreVmDatabase::new(store.clone(), parent_header)?;
        let mut vm = blockchain.new_evm(vm_db)?;
        let (execution_result, _bal) = vm
            .execute_block(&block)
            .wrap_err_with(|| format!("execution failed for block {number}"))?;
        let (_receipts_root, logs_bloom) =
            compute_receipts_root_and_logs_bloom(&execution_result.receipts, &NativeCrypto);

        if block.header.logs_bloom != logs_bloom {
            rebloomed += 1;
        }
        block.header.logs_bloom = logs_bloom;
        // The cached hash (if computed during execution above) was derived from
        // the stale bloom; clear it so `hash()` recomputes from the corrected
        // header.
        block.header.hash.take();
        let corrected_hash = block.header.hash();

        // Re-add through the normal validating path. This re-executes and runs
        // the full validation suite (including the bloom check), so success
        // proves the rewritten block is consensus-valid and makes its state
        // available as the parent of the next block.
        blockchain
            .add_block(block.clone())
            .wrap_err_with(|| format!("add_block failed for block {number}"))?;

        block.encode(&mut buf);
        out.write_all(&buf).wrap_err("failed to write block")?;
        buf.clear();

        corrected_parent = Some(corrected_hash);

        if (index + 1) % 100 == 0 {
            println!("processed {} blocks", index + 1);
        }
    }

    out.flush()?;
    println!("done: rewrote logs_bloom on {rebloomed} block header(s) -> {output_path}");
    Ok(())
}
