//! The payload builder applies withdrawals before the post-block request system calls,
//! matching block import and EELS `apply_body`. The order is observable only when a
//! withdrawal credits a request predeploy whose own system call spends that balance,
//! which no real predeploy does; here each predeploy's code forwards its whole balance
//! to a sink, as in the EEST case
//! `test_bal_withdrawals_and_dequeues_net_balance_at_last_index`. A builder that ran
//! the system calls first would forward nothing and commit to a state root every
//! import path rejects.

use std::{fs::File, io::BufReader, path::PathBuf, sync::Arc};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    types::{
        Block, DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER, Genesis, GenesisAccount,
        Withdrawal, block_access_list::BlockAccessList,
    },
};
use ethrex_storage::{EngineType, Store};
use ethrex_vm::system_contracts::{
    BUILDER_DEPOSIT_CONTRACT_ADDRESS, BUILDER_EXIT_CONTRACT_ADDRESS,
    CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS, WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
};

/// The predeploys called after the transactions on an Amsterdam block.
const POST_BLOCK_PREDEPLOYS: [Address; 4] = [
    WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS.address,
    CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS.address,
    BUILDER_DEPOSIT_CONTRACT_ADDRESS.address,
    BUILDER_EXIT_CONTRACT_ADDRESS.address,
];
const SINK: Address = H160([0xab; 20]);
const SINK_PRE_BALANCE: u64 = 1;
const WITHDRAWAL_GWEI: u64 = 1;
const GWEI: u64 = 1_000_000_000;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// `CALL(GAS, SINK, SELFBALANCE, 0, 0, 0, 0)`, result popped: sends the predeploy's
/// whole balance to `SINK`.
fn forward_balance_code() -> Bytes {
    let mut code = vec![0x5f, 0x5f, 0x5f, 0x5f, 0x47, 0x73];
    code.extend_from_slice(SINK.as_bytes());
    code.extend_from_slice(&[0x5a, 0xf1, 0x50, 0x00]);
    Bytes::from(code)
}

async fn setup_store() -> Store {
    let file = File::open(workspace_root().join("fixtures/genesis/l1-bal.json"))
        .expect("open l1-bal genesis");
    let mut genesis: Genesis =
        serde_json::from_reader(BufReader::new(file)).expect("parse l1-bal genesis");
    for predeploy in POST_BLOCK_PREDEPLOYS {
        let account = genesis
            .alloc
            .get_mut(&predeploy)
            .expect("l1-bal genesis allocates every request predeploy");
        account.code = forward_balance_code();
        account.storage.clear();
    }
    genesis.alloc.insert(
        SINK,
        GenesisAccount {
            code: Bytes::new(),
            storage: Default::default(),
            balance: U256::from(SINK_PRE_BALANCE),
            nonce: 0,
        },
    );
    let mut store = Store::new("store.db", EngineType::InMemory).expect("build in-memory store");
    store
        .add_initial_state(genesis)
        .await
        .expect("add genesis state");
    store
}

/// Builds an empty Amsterdam block on genesis that withdraws to every post-block
/// predeploy, with the BAL the producer recorded for it.
async fn build_forwarding_block() -> (Block, BlockAccessList) {
    let store = setup_store().await;
    let genesis_header = store.get_block_header(0).unwrap().unwrap();
    let withdrawals = POST_BLOCK_PREDEPLOYS
        .iter()
        .enumerate()
        .map(|(i, &address)| Withdrawal {
            index: i as u64,
            validator_index: i as u64,
            address,
            amount: WITHDRAWAL_GWEI,
        })
        .collect();
    let args = BuildPayloadArgs {
        parent: genesis_header.hash(),
        timestamp: genesis_header.timestamp + 12,
        fee_recipient: H160::zero(),
        random: H256::zero(),
        withdrawals: Some(withdrawals),
        beacon_root: Some(H256::zero()),
        slot_number: Some(1),
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };
    let payload = create_payload(&args, &store, Bytes::new()).unwrap();
    let result = Blockchain::new(store, BlockchainOptions::default())
        .build_payload(payload)
        .unwrap();
    let bal = result
        .block_access_list
        .expect("amsterdam block must produce a BAL");
    (result.payload, bal)
}

/// Every predeploy forwarded the withdrawal it received, so the predeploys end empty
/// and the sink holds all of it.
fn assert_withdrawals_forwarded(store: &Store, block: &Block) {
    let balance = |address| {
        store
            .get_account_info_by_hash(block.hash(), address)
            .unwrap()
            .map(|info| info.balance)
            .unwrap_or_default()
    };
    for predeploy in POST_BLOCK_PREDEPLOYS {
        assert_eq!(
            balance(predeploy),
            U256::zero(),
            "{predeploy:#x} must forward the withdrawal it received"
        );
    }
    let forwarded = WITHDRAWAL_GWEI * GWEI * POST_BLOCK_PREDEPLOYS.len() as u64;
    assert_eq!(
        balance(SINK),
        U256::from(SINK_PRE_BALANCE + forwarded),
        "the sink must hold every forwarded withdrawal"
    );
}

#[tokio::test]
async fn sequential_import_accepts_built_block_with_withdrawals_before_requests() {
    let (block, _) = build_forwarding_block().await;
    let store = setup_store().await;
    let result =
        Blockchain::new(store.clone(), BlockchainOptions::default()).add_block(block.clone());
    assert!(result.is_ok(), "built block must import, got: {result:?}");
    assert_withdrawals_forwarded(&store, &block);
}

#[tokio::test]
async fn pipeline_import_accepts_built_block_with_withdrawals_before_requests() {
    let (block, _) = build_forwarding_block().await;
    let store = setup_store().await;
    let result = Blockchain::new(store.clone(), BlockchainOptions::default())
        .add_block_pipeline(block.clone(), None);
    assert!(result.is_ok(), "built block must import, got: {result:?}");
    assert_withdrawals_forwarded(&store, &block);
}

#[tokio::test]
async fn parallel_import_accepts_built_block_with_withdrawals_before_requests() {
    let (block, bal) = build_forwarding_block().await;
    let store = setup_store().await;
    let blockchain = Blockchain::new(
        store.clone(),
        BlockchainOptions {
            bal_parallel_exec_enabled: true,
            ..Default::default()
        },
    );
    let result = blockchain.add_block_pipeline_bal(block.clone(), Some(Arc::new(bal)));
    assert!(result.is_ok(), "built block must import, got: {result:?}");
    assert_withdrawals_forwarded(&store, &block);
}
