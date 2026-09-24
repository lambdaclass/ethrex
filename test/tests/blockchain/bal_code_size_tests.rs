//! The parallel Amsterdam path rejects a BAL carrying a code change larger than the
//! maximum deployable code size before it uses the BAL to seed execution.
//!
//! Each transaction that loads an account builds that account's code from the BAL, so
//! the size check is structural and runs up front, next to the BAL index check, rather
//! than with the semantic BAL checks after execution.

use std::{fs::File, io::BufReader, path::PathBuf, sync::Arc};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    error::ChainError,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    H160, H256,
    constants::AMSTERDAM_MAX_CODE_SIZE,
    types::{
        Block, DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER, Genesis,
        block_access_list::{AccountChanges, BlockAccessList, CodeChange},
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_storage::{EngineType, Store};

const MAX: usize = AMSTERDAM_MAX_CODE_SIZE as usize;
const REJECTION: &str = "above the maximum code size";

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

async fn build_valid_amsterdam_block(store: &Store) -> (Block, BlockAccessList) {
    let blockchain = Blockchain::new(store.clone(), BlockchainOptions::default());
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
    let result = blockchain.build_payload(payload).unwrap();
    let bal = result
        .block_access_list
        .expect("amsterdam block must produce a BAL");
    (result.payload, bal)
}

/// Imports the valid empty block with one extra code change of `code_size` bytes added to
/// its BAL, the header re-committed to the altered BAL, down the parallel path.
async fn import_with_code_change(code_size: usize) -> Result<(), ChainError> {
    let (mut block, bal) = build_valid_amsterdam_block(&setup_store().await).await;

    // The highest address sorts last, so the BAL stays in canonical order. Index 1 is the
    // post-execution slot of an empty block, the only in-bounds non-system index.
    let mut accounts = bal.accounts().to_vec();
    accounts.push(
        AccountChanges::new(H160::repeat_byte(0xff))
            .with_code_changes(vec![CodeChange::new(1, Bytes::from(vec![0; code_size]))]),
    );
    let altered = BlockAccessList::from_accounts(accounts);
    block.header.block_access_list_hash = Some(altered.compute_hash(&NativeCrypto));
    block.header.hash = Default::default();

    let blockchain = Blockchain::new(
        setup_store().await,
        BlockchainOptions {
            bal_parallel_exec_enabled: true,
            ..Default::default()
        },
    );
    blockchain
        .add_block_pipeline_bal(block, Some(Arc::new(altered)))
        .map(|_| ())
}

#[tokio::test]
async fn parallel_path_rejects_bal_code_change_over_the_limit_before_execution() {
    let err = import_with_code_change(MAX + 1)
        .await
        .expect_err("a BAL code change over the maximum code size must be rejected");
    let err = format!("{err:?}");
    assert!(
        err.contains(REJECTION),
        "must be rejected by the structural code-size check, got: {err}"
    );
}

/// Code at the limit is deployable, so the structural check lets it through. The block is
/// still invalid (nothing in it deploys that code), but it must fail on the semantic BAL
/// checks after execution, not on the size check.
#[tokio::test]
async fn parallel_path_passes_bal_code_change_at_the_limit_to_semantic_checks() {
    let err = import_with_code_change(MAX)
        .await
        .expect_err("an undeployed code change is still an invalid BAL");
    let err = format!("{err:?}");
    assert!(
        !err.contains(REJECTION),
        "code at the limit must pass the size check and fail later, got: {err}"
    );
}
