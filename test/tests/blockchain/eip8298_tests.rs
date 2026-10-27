//! EIP-8298 `SETCODEFROM` through block production and import.
//!
//! One block deploys a contract, has another account adopt its code, and then has
//! the deployer replace its own code through `SETCODEFROM`. The adopter is then the
//! only account referencing the deployed bytecode, which appears in the block's
//! access list only as the deployer's earlier code change: every import path must
//! still store that bytecode for the adopter. The parallel path, which validates
//! the supplied access list against execution instead of rebuilding it, must also
//! reject an adoption recorded as bytecode.

use std::{collections::BTreeMap, fs::File, io::BufReader, path::PathBuf, sync::Arc};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    error::ChainError,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    evm::calculate_create_address,
    types::{
        Block, DEFAULT_BUILDER_GAS_CEIL, EIP1559Transaction, ELASTICITY_MULTIPLIER, Genesis,
        GenesisAccount, Receipt, Transaction, TxKind,
        block_access_list::{BlockAccessList, CodeChange},
    },
    utils::keccak,
};
use ethrex_crypto::NativeCrypto;
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_storage::{EngineType, Store};
use ethrex_vm::EvmError;
use secp256k1::SecretKey;

/// Rich account of `l1-hegota.json` (fixtures/keys/private_keys_tests.txt, line 1).
const SENDER_PRIVATE_KEY: &str = "850643a0224065ecce3882673c21f56bcf6eef86274cc21cadff15930b59fc8c";
const MAX_FEE_PER_GAS: u64 = 10_000_000_000;
const GAS_LIMIT: u64 = 1_000_000;

/// `SSTORE(0, SETCODEFROM(CALLDATALOAD(0)))`: adopts the code of the address in its
/// calldata and stores the result.
const ADOPTER: Address = H160([0x82; 20]);
const ADOPTER_CODE: &[u8] = &[0x60, 0x00, 0x35, 0xA7, 0x60, 0x00, 0x55, 0x00];
/// `SSTORE(1, 0x42)`: the code the deployed contract adopts later in the block.
const TEMPLATE: Address = H160([0x83; 20]);
const TEMPLATE_CODE: &[u8] = &[0x60, 0x42, 0x60, 0x01, 0x55, 0x00];
/// `POP(SETCODEFROM(CALLDATALOAD(0)))`: the runtime code the block deploys.
const DEPLOYED_CODE: &[u8] = &[0x60, 0x00, 0x35, 0xA7, 0x50, 0x00];

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn secret_key() -> SecretKey {
    SecretKey::from_slice(&hex::decode(SENDER_PRIVATE_KEY).unwrap()).unwrap()
}

/// `l1-hegota.json` (every fork through Hegota active at genesis) with the adopter
/// and the template installed.
async fn setup_store() -> (Store, u64) {
    let file = File::open(workspace_root().join("fixtures/genesis/l1-hegota.json"))
        .expect("open l1-hegota genesis");
    let mut genesis: Genesis =
        serde_json::from_reader(BufReader::new(file)).expect("parse l1-hegota genesis");
    for (address, code) in [(ADOPTER, ADOPTER_CODE), (TEMPLATE, TEMPLATE_CODE)] {
        genesis.alloc.insert(
            address,
            GenesisAccount {
                code: Bytes::from_static(code),
                storage: BTreeMap::new(),
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
            min_tip_wei: 0,
            ..Default::default()
        },
    )
}

fn word(address: Address) -> Bytes {
    let mut word = [0u8; 32];
    word[12..].copy_from_slice(address.as_bytes());
    Bytes::copy_from_slice(&word)
}

/// Initcode returning `DEPLOYED_CODE`.
fn deploy_initcode() -> Bytes {
    let mut initcode = vec![0x7F]; // PUSH32 the code, left-aligned
    let mut padded = [0u8; 32];
    padded[..DEPLOYED_CODE.len()].copy_from_slice(DEPLOYED_CODE);
    initcode.extend_from_slice(&padded);
    // MSTORE(0), RETURN(0, len)
    initcode.extend_from_slice(&[
        0x60,
        0x00,
        0x52,
        0x60,
        DEPLOYED_CODE.len() as u8,
        0x60,
        0x00,
    ]);
    initcode.push(0xF3);
    Bytes::from(initcode)
}

async fn signed_tx(chain_id: u64, nonce: u64, to: TxKind, data: Bytes) -> Transaction {
    let signer: Signer = LocalSigner::new(secret_key()).into();
    let mut tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id,
        nonce,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        gas_limit: GAS_LIMIT,
        to,
        data,
        ..Default::default()
    });
    tx.sign_inplace(&signer).await.unwrap();
    tx
}

/// Builds the block: tx 1 deploys `DEPLOYED_CODE`, tx 2 has the adopter adopt it and
/// tx 3 has the deployed contract adopt the template's code.
async fn build_block() -> (Block, BlockAccessList, Vec<Receipt>, Address) {
    let (store, chain_id) = setup_store().await;
    let blockchain = blockchain(store.clone(), true);
    let sender = LocalSigner::new(secret_key()).address;
    let deployed = calculate_create_address(sender, 0);
    let txs = [
        signed_tx(chain_id, 0, TxKind::Create, deploy_initcode()).await,
        signed_tx(chain_id, 1, TxKind::Call(ADOPTER), word(deployed)).await,
        signed_tx(chain_id, 2, TxKind::Call(deployed), word(TEMPLATE)).await,
    ];
    for tx in txs {
        blockchain
            .add_transaction_to_pool(tx)
            .await
            .expect("tx should enter the pool");
    }

    let parent = store.get_block_header(0).unwrap().unwrap();
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
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };
    let payload = create_payload(&args, &store, Bytes::new()).unwrap();
    let result = blockchain.build_payload(payload).unwrap();
    let bal = result
        .block_access_list
        .expect("a Hegota block must produce a BAL");
    (result.payload, bal, result.receipts, deployed)
}

/// After importing the block, the adopter runs the deployed code and the store holds
/// it, although the contract that deployed it moved on to the template's code.
fn assert_imported_state(store: &Store, block: &Block, deployed: Address) {
    let state_root = block.header.state_root;
    let adopter = store
        .get_account_state_by_root(state_root, ADOPTER)
        .unwrap()
        .expect("adopter exists");
    assert_eq!(adopter.code_hash, keccak(DEPLOYED_CODE));
    let deployed = store
        .get_account_state_by_root(state_root, deployed)
        .unwrap()
        .expect("deployed contract exists");
    assert_eq!(deployed.code_hash, keccak(TEMPLATE_CODE));
    let code = store
        .get_account_code(keccak(DEPLOYED_CODE))
        .unwrap()
        .expect("the adopted bytecode must be stored");
    assert_eq!(code.code(), DEPLOYED_CODE);
}

#[tokio::test]
async fn adoption_of_code_deployed_earlier_in_the_block_imports_on_every_path() {
    let (block, bal, receipts, deployed) = build_block().await;
    assert_eq!(block.body.transactions.len(), 3);
    assert!(
        receipts.iter().all(|r| r.succeeded),
        "every tx must succeed: {receipts:?}"
    );

    let changes = |address: Address| {
        bal.accounts()
            .iter()
            .find(|account| account.address == address)
            .map(|account| account.code_changes.clone())
            .unwrap_or_default()
    };
    assert_eq!(
        changes(deployed),
        vec![
            CodeChange::new(1, Bytes::from_static(DEPLOYED_CODE)),
            CodeChange::adoption(3, keccak(TEMPLATE_CODE)),
        ]
    );
    assert_eq!(
        changes(ADOPTER),
        vec![CodeChange::adoption(2, keccak(DEPLOYED_CODE))]
    );

    // Parallel: per-tx databases seeded from the BAL, merkleized from the BAL.
    let (store, _) = setup_store().await;
    blockchain(store.clone(), true)
        .add_block_pipeline(block.clone(), Some(Arc::new(bal.clone())))
        .expect("parallel import");
    assert_imported_state(&store, &block, deployed);

    // Sequential pipeline, rebuilding the BAL.
    let (store, _) = setup_store().await;
    blockchain(store.clone(), false)
        .add_block_pipeline(block.clone(), Some(Arc::new(bal)))
        .expect("sequential pipeline import");
    assert_imported_state(&store, &block, deployed);

    // Non-pipelined import.
    let (store, _) = setup_store().await;
    blockchain(store.clone(), true)
        .add_block(block.clone())
        .expect("plain import");
    assert_imported_state(&store, &block, deployed);
}

#[tokio::test]
async fn parallel_path_rejects_an_adoption_recorded_as_bytecode() {
    let (block, bal, _, _) = build_block().await;

    // Same code hash, so the state root stands; only the form, and with it the
    // BAL hash the header commits to, differs.
    let mut accounts = bal.accounts().to_vec();
    let adopter = accounts
        .iter_mut()
        .find(|account| account.address == ADOPTER)
        .expect("adopter in BAL");
    adopter.code_changes = vec![CodeChange::new(2, Bytes::from_static(DEPLOYED_CODE))];
    let mutated_bal = Arc::new(BlockAccessList::from_accounts(accounts));
    let mut mutated_block = block;
    mutated_block.header.block_access_list_hash = Some(mutated_bal.compute_hash(&NativeCrypto));

    let (store, _) = setup_store().await;
    let result = blockchain(store, true).add_block_pipeline_bal(mutated_block, Some(mutated_bal));
    match result {
        Err(ChainError::EvmError(EvmError::Custom(msg))) => assert!(
            msg.contains("recorded as bytecode, but the code was adopted"),
            "unexpected rejection: {msg}"
        ),
        other => panic!("expected the BAL form check to reject the block, got: {other:?}"),
    }
}
