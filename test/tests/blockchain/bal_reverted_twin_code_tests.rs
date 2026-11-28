//! A contract deployed earlier in an Amsterdam block, whose code bytes a reverted frame
//! later in the block also deployed.
//!
//! On the parallel BAL path every transaction gets a fresh database, and the block access
//! list provides the code deployed earlier in the block, which the store does not hold yet.
//! Here a frame deploys the same bytes before the earlier contract is first loaded, so the
//! frame's insert and the block access list share one entry of the by-hash code cache. The
//! frame then reverts, and the earlier contract's code must still be readable afterwards,
//! on both paths.

use std::{fs::File, io::BufReader, path::PathBuf, sync::Arc};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    evm::calculate_create_address,
    types::{
        Block, BlockHeader, EIP1559Transaction, ELASTICITY_MULTIPLIER, Genesis, GenesisAccount,
        Receipt, Transaction, TxKind, block_access_list::BlockAccessList,
    },
};
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_levm::utils::calculate_create2_address;
use ethrex_storage::{EngineType, Store};
use secp256k1::SecretKey;

const SENDER_PRIVATE_KEY: &str = "850643a0224065ecce3882673c21f56bcf6eef86274cc21cadff15930b59fc8c";
const BLOCK_GAS_LIMIT: u64 = 60_000_000;
const TX_GAS_LIMIT: u64 = 2_000_000;
const MAX_FEE_PER_GAS: u64 = 10_000_000_000;

/// The contract that calls the reverting deployer, then reads the earlier contract.
const PARENT: Address = H160([0xc0; 20]);
/// The contract that deploys the same code, calls the earlier contract, and reverts.
const REVERTING_DEPLOYER: Address = H160([0xd0; 20]);

/// Runtime code that is new in the test block: PUSH4 0xdeadbeef, PUSH1 0, MSTORE,
/// PUSH1 32, PUSH1 0, RETURN.
const RUNTIME: [u8; 13] = [
    0x63, 0xde, 0xad, 0xbe, 0xef, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3,
];

/// Init code returning [`RUNTIME`]: PUSH13 runtime, PUSH1 0, MSTORE, PUSH1 13, PUSH1 19,
/// RETURN (MSTORE right-aligns the 13 bytes at offset 19).
fn init_code() -> Vec<u8> {
    let mut code = vec![0x6c];
    code.extend_from_slice(&RUNTIME);
    code.extend_from_slice(&[0x60, 0x00, 0x52, 0x60, 0x0d, 0x60, 0x13, 0xf3]);
    code
}

/// CALL `target` with no value, no arguments and all remaining gas, and drop the result.
fn call(target: Address) -> Vec<u8> {
    let mut code = vec![0x5f, 0x5f, 0x5f, 0x5f, 0x5f, 0x73];
    code.extend_from_slice(target.as_bytes());
    code.extend_from_slice(&[0x5a, 0xf1, 0x50]);
    code
}

/// The reverting deployer: copies its calldata (the init code), CREATE2s it with salt 0,
/// calls `earlier` (its first load in the transaction), then reverts with the address the
/// CREATE2 returned.
fn reverting_deployer_code(earlier: Address) -> Vec<u8> {
    let mut code = vec![
        0x36, 0x5f, 0x5f, 0x37, // CALLDATACOPY(0, 0, CALLDATASIZE)
        0x5f, 0x36, 0x5f, 0x5f, 0xf5, // CREATE2(0, 0, CALLDATASIZE, salt 0)
        0x5f, 0x52, // MSTORE(0, created address)
    ];
    code.extend(call(earlier));
    code.extend_from_slice(&[0x60, 0x20, 0x5f, 0xfd]); // REVERT(0, 32)
    code
}

/// The parent: forwards its calldata to the reverting deployer and stores the address its
/// CREATE2 returned in slot 2, then stores EXTCODESIZE(`earlier`) in slot 0, calls `earlier`
/// and stores the size of its return data in slot 1.
fn parent_code(earlier: Address) -> Vec<u8> {
    let mut code = vec![
        0x36, 0x5f, 0x5f, 0x37, // CALLDATACOPY(0, 0, CALLDATASIZE)
        0x5f, 0x5f, 0x36, 0x5f, 0x5f, 0x73, // CALL(gas, deployer, 0, 0, CALLDATASIZE, 0, 0)
    ];
    code.extend_from_slice(REVERTING_DEPLOYER.as_bytes());
    code.extend_from_slice(&[0x5a, 0xf1, 0x50]);
    // RETURNDATACOPY(0, 0, 32), SSTORE(2, MLOAD(0))
    code.extend_from_slice(&[0x60, 0x20, 0x5f, 0x5f, 0x3e, 0x5f, 0x51, 0x60, 0x02, 0x55]);
    code.push(0x73); // PUSH20 earlier
    code.extend_from_slice(earlier.as_bytes());
    code.extend_from_slice(&[0x3b, 0x5f, 0x55]); // EXTCODESIZE, SSTORE(0, size)
    code.extend(call(earlier));
    code.extend_from_slice(&[0x3d, 0x60, 0x01, 0x55]); // RETURNDATASIZE, SSTORE(1, size)
    code.push(0x00); // STOP
    code
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn signer() -> Signer {
    LocalSigner::new(SecretKey::from_slice(&hex::decode(SENDER_PRIVATE_KEY).unwrap()).unwrap())
        .into()
}

/// The earlier contract: the first transaction of the block deploys it from the sender's
/// nonce 0.
fn earlier_contract() -> Address {
    calculate_create_address(signer().address(), 0)
}

async fn setup_store() -> (Store, u64) {
    let file = File::open(workspace_root().join("fixtures/genesis/l1-bal.json"))
        .expect("open l1-bal genesis");
    let mut genesis: Genesis =
        serde_json::from_reader(BufReader::new(file)).expect("parse l1-bal genesis");
    genesis.gas_limit = BLOCK_GAS_LIMIT;
    let earlier = earlier_contract();
    for (address, code) in [
        (PARENT, parent_code(earlier)),
        (REVERTING_DEPLOYER, reverting_deployer_code(earlier)),
    ] {
        genesis.alloc.insert(
            address,
            GenesisAccount {
                code: Bytes::from(code),
                storage: Default::default(),
                balance: U256::zero(),
                nonce: 1,
            },
        );
    }
    let chain_id = genesis.config.chain_id;
    let mut store = Store::new("store.db", EngineType::InMemory).expect("build in-memory store");
    store
        .add_initial_state(genesis)
        .await
        .expect("add genesis state");
    (store, chain_id)
}

fn blockchain(store: Store, bal_parallel_exec_enabled: bool) -> Blockchain {
    Blockchain::new(
        store,
        BlockchainOptions {
            bal_parallel_exec_enabled,
            // The transactions below pay no priority fee; admission policy is not under test.
            min_tip_wei: 0,
            ..Default::default()
        },
    )
}

async fn transaction(chain_id: u64, nonce: u64, to: TxKind, data: Vec<u8>) -> Transaction {
    let mut tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id,
        nonce,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        gas_limit: TX_GAS_LIMIT,
        to,
        value: U256::zero(),
        data: Bytes::from(data),
        ..Default::default()
    });
    tx.sign_inplace(&signer()).await.unwrap();
    tx
}

/// Builds, on a store of its own, the block: transaction 0 deploys the earlier contract,
/// transaction 1 calls the parent with the same init code as calldata.
async fn build_block() -> (Block, BlockAccessList, Vec<Receipt>) {
    let (store, chain_id) = setup_store().await;
    let builder = blockchain(store.clone(), false);
    for tx in [
        transaction(chain_id, 0, TxKind::Create, init_code()).await,
        transaction(chain_id, 1, TxKind::Call(PARENT), init_code()).await,
    ] {
        builder
            .add_transaction_to_pool(tx)
            .await
            .expect("transaction enters the pool");
    }
    let parent: BlockHeader = store.get_block_header(0).unwrap().unwrap();
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
    let payload = create_payload(&args, &store, Bytes::new()).unwrap();
    let result = builder.build_payload(payload).unwrap();
    let bal = result
        .block_access_list
        .expect("an Amsterdam block carries a block access list");
    (result.payload, bal, result.receipts)
}

/// Imports the block on a fresh store through the given path and checks what the parent read
/// of the earlier contract after the reverted frame. The slot values are pinned here rather
/// than taken from the builder, so both paths are checked against the expected reads.
async fn import_and_check_reads(bal_parallel_exec_enabled: bool) {
    let (block, bal, receipts) = build_block().await;
    assert_eq!(
        block.body.transactions.len(),
        2,
        "both transactions included"
    );
    assert!(
        receipts.iter().all(|receipt| receipt.succeeded),
        "both transactions succeed, got {receipts:?}"
    );
    let earlier = earlier_contract();
    assert!(
        bal.accounts()
            .iter()
            .any(|account| account.address == earlier && !account.code_changes.is_empty()),
        "the block access list carries the earlier contract's deployment"
    );

    let (store, _) = setup_store().await;
    let state_root = block.header.state_root;
    blockchain(store.clone(), bal_parallel_exec_enabled)
        .add_block_pipeline_bal(block, Some(Arc::new(bal)))
        .expect("the block imports");
    let slot = |key: u64| {
        store
            .get_storage_at_root(state_root, PARENT, H256::from_low_u64_be(key))
            .unwrap()
    };
    assert_eq!(
        slot(0),
        Some(U256::from(RUNTIME.len())),
        "EXTCODESIZE of the earlier contract"
    );
    assert_eq!(
        slot(1),
        Some(U256::from(32)),
        "the earlier contract's code ran and returned one word"
    );
    let twin = calculate_create2_address(REVERTING_DEPLOYER, &init_code(), U256::zero())
        .expect("CREATE2 address");
    assert_eq!(
        slot(2),
        Some(U256::from_big_endian(twin.as_bytes())),
        "the reverted frame's CREATE2 deployed the twin"
    );
    assert_eq!(
        store.get_account_state_by_root(state_root, twin).unwrap(),
        None,
        "the reverted frame's deployment does not persist"
    );
}

#[tokio::test]
async fn parallel_path_keeps_block_access_list_code_through_a_reverted_twin_deployment() {
    import_and_check_reads(true).await;
}

#[tokio::test]
async fn sequential_path_reads_the_same_code() {
    import_and_check_reads(false).await;
}
