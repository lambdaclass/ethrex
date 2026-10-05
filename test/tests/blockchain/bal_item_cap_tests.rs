//! A supplied BAL over the EIP-7928 item cap (`gas_limit / BAL_ITEM_COST`) must not
//! drive the block pipeline. Everything the pipeline builds from a supplied BAL (trie
//! updates, prefetches, the warmer, the parallel executor's indices) scales with the
//! BAL's size rather than with gas, so such a block runs on the sequential path, which
//! rebuilds the BAL from execution and rejects the block by its hash.
//!
//! The BALs below pad a valid block's BAL with accounts that were never accessed and
//! commit the header to the padded BAL, so the only thing that differs between the two
//! cases is the item count. The parallel path reports the unaccessed account; the
//! sequential path reports the hash mismatch. Which error comes back shows which path
//! the BAL drove.

use std::{fs::File, io::BufReader, path::PathBuf, sync::Arc};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    error::{ChainError, InvalidBlockError},
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    H160, H256,
    constants::BAL_ITEM_COST,
    types::{
        Block, DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER, Genesis,
        block_access_list::{AccountChanges, BlockAccessList},
    },
};
use ethrex_crypto::NativeCrypto;
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

/// A fully valid empty Amsterdam block on top of genesis and the BAL its producer recorded.
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
        slot_number: Some(1),
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

/// Pads `bal` with never-accessed accounts up to `items` items, in ascending address order,
/// and commits `block`'s header to the result.
fn pad_bal(block: &mut Block, bal: &BlockAccessList, items: u64) -> Arc<BlockAccessList> {
    let mut accounts = bal.accounts().to_vec();
    let mut next: u64 = 1 << 40;
    let mut count = bal.item_count();
    while count < items {
        accounts.push(AccountChanges::new(H160::from_low_u64_be(next)));
        next += 1;
        count += 1;
    }
    accounts.sort_by_key(|account| account.address);
    let padded = BlockAccessList::from_accounts(accounts);
    assert_eq!(padded.item_count(), items);
    block.header.block_access_list_hash = Some(padded.compute_hash(&NativeCrypto));
    Arc::new(padded)
}

async fn import_parallel(block: Block, bal: Arc<BlockAccessList>) -> Result<(), ChainError> {
    let bc = Blockchain::new(
        setup_store().await,
        BlockchainOptions {
            bal_parallel_exec_enabled: true,
            ..Default::default()
        },
    );
    bc.add_block_pipeline_bal(block, Some(bal)).map(|_| ())
}

#[tokio::test]
async fn bal_at_the_item_cap_drives_the_parallel_path() {
    let (mut block, bal) = build_valid_amsterdam_block(&setup_store().await).await;
    let max_items = block.header.gas_limit / BAL_ITEM_COST;
    let padded = pad_bal(&mut block, &bal, max_items);

    let result = import_parallel(block, padded).await;
    let error = format!("{result:?}");
    assert!(
        error.contains("never accessed during block execution"),
        "a BAL at the cap must drive parallel execution, which reports the unaccessed \
         account, got: {error}"
    );
}

#[tokio::test]
async fn bal_over_the_item_cap_runs_the_sequential_path() {
    let (mut block, bal) = build_valid_amsterdam_block(&setup_store().await).await;
    let max_items = block.header.gas_limit / BAL_ITEM_COST;
    let padded = pad_bal(&mut block, &bal, max_items + 1);

    let result = import_parallel(block, padded).await;
    assert!(
        matches!(
            result,
            Err(ChainError::InvalidBlock(
                InvalidBlockError::BlockAccessListHashMismatch
            ))
        ),
        "a BAL over the cap must not drive execution: the sequential path rebuilds the BAL \
         and rejects the block by its hash, got: {result:?}"
    );
}
