//! The parallel Amsterdam path rejects a BAL carrying a code change larger than the
//! maximum deployable code size before it uses the BAL to seed execution, and still
//! accepts a block that deploys code of exactly that size.
//!
//! Each transaction that loads an account builds that account's code from the BAL, so
//! the size check is structural and runs up front, next to the BAL index check, rather
//! than with the semantic BAL checks after execution.

use std::{fs::File, io::BufReader, path::PathBuf, sync::Arc};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    H160, H256, U256,
    constants::AMSTERDAM_MAX_CODE_SIZE,
    types::{
        Block, BlockHeader, EIP1559Transaction, ELASTICITY_MULTIPLIER, Genesis, Receipt,
        Transaction, TxKind,
        block_access_list::{AccountChanges, BlockAccessList, CodeChange},
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_storage::{EngineType, Store};
use secp256k1::SecretKey;

const MAX: usize = AMSTERDAM_MAX_CODE_SIZE as usize;
const REJECTION: &str = "above the maximum code size";

/// Test private key from fixtures/keys/private_keys_tests.txt (line 1), a rich account
/// in `l1-bal.json`.
const SENDER_PRIVATE_KEY: &str = "850643a0224065ecce3882673c21f56bcf6eef86274cc21cadff15930b59fc8c";

/// `PUSH3 0x010000 PUSH1 0 RETURN`: returns 65,536 zero bytes (STOP opcodes) as the
/// runtime code, exactly the maximum deployable size.
const MAX_SIZE_INIT_CODE: [u8; 7] = [0x62, 0x01, 0x00, 0x00, 0x60, 0x00, 0xf3];
/// EIP-8037 charges the code deposit as state gas at 1,530 per byte, about 100.3M for
/// 64 KiB. A transaction pays state gas from the part of its limit above the regular
/// cap (`TX_MAX_GAS_LIMIT_AMSTERDAM`, 16.8M), so the limit has to carry both.
const DEPLOY_GAS_LIMIT: u64 = 120_000_000;
/// Deploying 64 KiB does not fit `l1-bal.json`'s 25M block gas limit; use Plataberget's.
const BLOCK_GAS_LIMIT: u64 = 200_000_000;
const MAX_FEE_PER_GAS: u64 = 10_000_000_000;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

async fn setup_store() -> (Store, u64) {
    let file = File::open(workspace_root().join("fixtures/genesis/l1-bal.json"))
        .expect("open l1-bal genesis");
    let mut genesis: Genesis =
        serde_json::from_reader(BufReader::new(file)).expect("parse l1-bal genesis");
    genesis.gas_limit = BLOCK_GAS_LIMIT;
    let chain_id = genesis.config.chain_id;
    let mut store = Store::new("store.db", EngineType::InMemory).expect("build in-memory store");
    store
        .add_initial_state(genesis)
        .await
        .expect("add genesis state");
    (store, chain_id)
}

fn parallel_blockchain(store: Store) -> Blockchain {
    Blockchain::new(
        store,
        BlockchainOptions {
            bal_parallel_exec_enabled: true,
            // The deployment below pays no priority fee; admission policy is not under test.
            min_tip_wei: 0,
            ..Default::default()
        },
    )
}

/// Builds an Amsterdam block on top of `parent` from whatever is in `blockchain`'s
/// mempool, with the BAL and receipts the producer recorded for it.
fn build_block(
    store: &Store,
    blockchain: &Blockchain,
    parent: &BlockHeader,
) -> (Block, BlockAccessList, Vec<Receipt>) {
    let args = BuildPayloadArgs {
        parent: parent.hash(),
        timestamp: parent.timestamp + 12,
        fee_recipient: H160::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        slot_number: Some(1),
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: BLOCK_GAS_LIMIT,
    };
    let payload = create_payload(&args, store, Bytes::new()).unwrap();
    let result = blockchain.build_payload(payload).unwrap();
    let bal = result
        .block_access_list
        .expect("amsterdam block must produce a BAL");
    (result.payload, bal, result.receipts)
}

/// A valid empty block with one extra code change of `MAX + 1` bytes added to its BAL,
/// and the header re-committed to the altered BAL, must be rejected by the size check
/// before anything executes.
#[tokio::test]
async fn parallel_path_rejects_bal_code_change_over_the_limit_before_execution() {
    let (store, _) = setup_store().await;
    let genesis_header = store.get_block_header(0).unwrap().unwrap();
    let (mut block, bal, _) = build_block(
        &store,
        &Blockchain::new(store.clone(), BlockchainOptions::default()),
        &genesis_header,
    );

    // The highest address sorts last, so the BAL stays in canonical order. Index 1 is the
    // post-execution slot of an empty block, the only in-bounds non-system index.
    let mut accounts = bal.accounts().to_vec();
    accounts.push(
        AccountChanges::new(H160::repeat_byte(0xff))
            .with_code_changes(vec![CodeChange::new(1, Bytes::from(vec![0; MAX + 1]))]),
    );
    let altered = BlockAccessList::from_accounts(accounts);
    block.header.block_access_list_hash = Some(altered.compute_hash(&NativeCrypto));
    block.header.hash = Default::default();

    let (import_store, _) = setup_store().await;
    let result =
        parallel_blockchain(import_store).add_block_pipeline_bal(block, Some(Arc::new(altered)));

    let err = format!(
        "{:?}",
        result.expect_err("a BAL code change over the maximum code size must be rejected")
    );
    assert!(
        err.contains(REJECTION),
        "must be rejected by the structural code-size check, got: {err}"
    );
}

/// Code of exactly the maximum size is deployable, so a block that deploys it carries a
/// legitimate `MAX`-byte code change in its BAL and must import on the parallel path.
#[tokio::test]
async fn parallel_path_accepts_a_block_deploying_code_of_the_maximum_size() {
    let (build_store, chain_id) = setup_store().await;
    let build_blockchain = parallel_blockchain(build_store.clone());
    let genesis_header = build_store.get_block_header(0).unwrap().unwrap();

    let signer: Signer =
        LocalSigner::new(SecretKey::from_slice(&hex::decode(SENDER_PRIVATE_KEY).unwrap()).unwrap())
            .into();
    let mut tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id,
        nonce: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        gas_limit: DEPLOY_GAS_LIMIT,
        to: TxKind::Create,
        value: U256::zero(),
        data: Bytes::from_static(&MAX_SIZE_INIT_CODE),
        ..Default::default()
    });
    tx.sign_inplace(&signer).await.unwrap();
    build_blockchain
        .add_transaction_to_pool(tx)
        .await
        .expect("the deployment must enter the pool");

    let (block, bal, receipts) = build_block(&build_store, &build_blockchain, &genesis_header);
    assert_eq!(
        block.body.transactions.len(),
        1,
        "block must include the deployment"
    );
    assert!(
        receipts[0].succeeded,
        "deployment must succeed, got: {receipts:?}"
    );
    // The BAL really carries a code change at the limit, so this exercises the check.
    let deployed_sizes: Vec<usize> = bal
        .accounts()
        .iter()
        .flat_map(|account| account.code_changes.iter().map(|c| c.new_code.len()))
        .collect();
    assert_eq!(
        deployed_sizes,
        vec![MAX],
        "expected one code change of exactly MAX bytes"
    );

    let (import_store, _) = setup_store().await;
    let result =
        parallel_blockchain(import_store).add_block_pipeline_bal(block, Some(Arc::new(bal)));
    assert!(
        result.is_ok(),
        "a block deploying code of the maximum size must import, got: {result:?}"
    );
}
