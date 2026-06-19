//! Regression test for the `bal-hash-parallel-skip` finding: the default
//! parallel block-import path must reject a block whose header
//! `block_access_list_hash` does not match `keccak(rlp(BAL))`, just like the
//! sequential path and every spec-conformant client (EIP-7928 block validity).
//!
//! The parallel Amsterdam path uses the header BAL to drive execution and never
//! rebuilds it, so before the fix the commitment check (gated on a rebuilt BAL)
//! never fired: a block with a content-valid BAL but a forged commitment was
//! accepted on the parallel path while the sequential path rejected it.
//!
//! The differential below builds a fully-valid Amsterdam block, forges only its
//! header `block_access_list_hash`, and imports it down both paths with the
//! canonical BAL supplied (what the P2P-sync caller hands to the pipeline).
//! Only `bal_parallel_exec_enabled` is flipped between the two imports.

use std::{fs::File, io::BufReader, path::PathBuf, sync::Arc};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    error::{ChainError, InvalidBlockError},
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    H160, H256,
    types::{
        Block, DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER, Genesis,
        block_access_list::BlockAccessList,
    },
};
use ethrex_storage::{EngineType, Store};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

async fn setup_store() -> Store {
    let file = File::open(workspace_root().join("fixtures/genesis/l1-bal.json"))
        .expect("open l1-bal genesis");
    let genesis: Genesis =
        serde_json::from_reader(BufReader::new(file)).expect("parse l1-bal genesis");
    let mut store = Store::new("store.db", EngineType::InMemory).expect("build in-memory store");
    store
        .add_initial_state(genesis)
        .await
        .expect("add genesis state");
    store
}

/// Produce a fully-valid empty Amsterdam block on top of genesis and the
/// canonical BAL the producer recorded for it.
async fn build_valid_amsterdam_block(store: &Store) -> (Block, BlockAccessList) {
    let bc = Blockchain::new(store.clone(), BlockchainOptions::default());
    let genesis_header = store.get_block_header(0).unwrap().unwrap();
    let args = BuildPayloadArgs {
        parent: genesis_header.hash(),
        timestamp: genesis_header.timestamp + 12,
        fee_recipient: H160::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        slot_number: None,
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };
    let payload = create_payload(&args, store, Bytes::new()).unwrap();
    let result = bc.build_payload(payload).unwrap();
    let bal = result
        .block_access_list
        .expect("amsterdam block must produce a BAL");
    (result.payload, bal)
}

#[tokio::test]
async fn parallel_path_rejects_invalid_block_access_list_hash() {
    let build_store = setup_store().await;
    let (mut block, bal) = build_valid_amsterdam_block(&build_store).await;
    let bal = Arc::new(bal);

    // The empty block still records a non-empty BAL via the EIP-4788 block-start
    // system call, so the fork-activation guard is not vacuous.
    assert!(
        !bal.accounts().is_empty(),
        "BAL should be non-empty (4788 system call)"
    );

    let canonical_hash = bal.compute_hash();
    let forged = H256([0xde; 32]);
    assert_ne!(canonical_hash, forged);

    // Positive control: the UNFORGED block must still import on the parallel
    // path. Guards against a fix that rejects everything.
    let store_ok = setup_store().await;
    let bc_ok = Blockchain::new(
        store_ok,
        BlockchainOptions {
            bal_parallel_exec_enabled: true,
            ..Default::default()
        },
    );
    let valid = bc_ok.add_block_pipeline_bal(block.clone(), Some(bal.clone()));
    assert!(
        valid.is_ok(),
        "parallel path must accept a block with a correct commitment, got: {valid:?}"
    );

    // Forge ONLY the header commitment; keep the canonical BAL.
    block.header.block_access_list_hash = Some(forged);

    // PARALLEL (default): import with the canonical BAL supplied.
    let store_par = setup_store().await;
    let bc_par = Blockchain::new(
        store_par,
        BlockchainOptions {
            bal_parallel_exec_enabled: true,
            ..Default::default()
        },
    );
    let par = bc_par.add_block_pipeline_bal(block.clone(), Some(bal.clone()));

    // SEQUENTIAL: same block, same BAL, only the parallel flag flipped.
    let store_seq = setup_store().await;
    let bc_seq = Blockchain::new(
        store_seq,
        BlockchainOptions {
            bal_parallel_exec_enabled: false,
            ..Default::default()
        },
    );
    let seq = bc_seq.add_block_pipeline_bal(block.clone(), Some(bal.clone()));

    // Both paths must reject a forged commitment with the same error.
    assert!(
        matches!(
            par,
            Err(ChainError::InvalidBlock(
                InvalidBlockError::BlockAccessListHashMismatch
            ))
        ),
        "parallel path must reject forged block_access_list_hash, got: {par:?}"
    );
    assert!(
        matches!(
            seq,
            Err(ChainError::InvalidBlock(
                InvalidBlockError::BlockAccessListHashMismatch
            ))
        ),
        "sequential path must reject forged block_access_list_hash, got: {seq:?}"
    );
}
