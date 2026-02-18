use std::{fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    evm::calculate_create_address,
    types::{
        Block, BlockHeader, EIP1559Transaction, GenesisAccount, Transaction, TxKind,
        DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER,
    },
};
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_storage::{EngineType, Store};
use secp256k1::SecretKey;
use tokio_util::sync::CancellationToken;

/// Test private key from fixtures/keys/private_keys_tests.txt.
const TEST_PRIVATE_KEY: &str = "850643a0224065ecce3882673c21f56bcf6eef86274cc21cadff15930b59fc8c";
/// Comfortably high max fee — well above any genesis base fee.
const TEST_MAX_FEE_PER_GAS: u64 = 10_000_000_000;
const TEST_GAS_LIMIT: u64 = 100_000;

fn test_secret_key() -> SecretKey {
    SecretKey::from_slice(&hex::decode(TEST_PRIVATE_KEY).unwrap()).unwrap()
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Load the execution-api genesis, inject `sender` with a large balance,
/// and return an in-memory store together with the chain id.
async fn setup_store(sender: Address) -> (Store, u64) {
    let file = File::open(workspace_root().join("fixtures/genesis/execution-api.json"))
        .expect("Failed to open genesis file");
    let reader = BufReader::new(file);
    let mut genesis: ethrex_common::types::Genesis =
        serde_json::from_reader(reader).expect("Failed to deserialize genesis file");

    let chain_id = genesis.config.chain_id;

    // Give the sender a large balance so it can fund the transactions.
    genesis.alloc.insert(
        sender,
        GenesisAccount {
            balance: U256::from(10).pow(U256::from(20)), // 100 ETH
            code: Bytes::new(),
            storage: Default::default(),
            nonce: 0,
        },
    );

    let mut store =
        Store::new("store.db", EngineType::InMemory).expect("Failed to build DB for testing");

    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to add genesis state");

    (store, chain_id)
}

/// Build a block on top of `parent_header` using the payload builder,
/// including whatever transactions are currently in the mempool.
async fn build_block(store: &Store, blockchain: &Blockchain, parent_header: &BlockHeader) -> Block {
    // Use fixed values instead of random ones for deterministic, reproducible tests.
    let args = BuildPayloadArgs {
        parent: parent_header.hash(),
        timestamp: parent_header.timestamp + 12,
        fee_recipient: H160::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        slot_number: None,
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };

    let block = create_payload(&args, store, Bytes::new()).unwrap();
    let result = blockchain.build_payload(block).unwrap();
    result.payload
}

/// Build init code: `PUSH20 <beneficiary> SELFDESTRUCT`
///
/// Under EIP-6780 (post-Cancun) the contract is truly destroyed because
/// SELFDESTRUCT is executed in the same transaction that created it.
fn selfdestruct_init_code(beneficiary: Address) -> Bytes {
    let mut code = Vec::with_capacity(22);
    code.push(0x73); // PUSH20
    code.extend_from_slice(beneficiary.as_bytes());
    code.push(0xFF); // SELFDESTRUCT
    Bytes::from(code)
}

/// Derive the sender address from a secp256k1 private key.
fn sender_from_key(sk: &SecretKey) -> Address {
    LocalSigner::new(*sk).address
}

/// Build and sign a self-destructing contract deployment transaction.
async fn create_selfdestruct_deploy_tx(
    chain_id: u64,
    nonce: u64,
    beneficiary: Address,
    signer: &Signer,
) -> Transaction {
    let mut tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id,
        nonce,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: TEST_MAX_FEE_PER_GAS,
        gas_limit: TEST_GAS_LIMIT,
        to: TxKind::Create,
        value: U256::zero(),
        data: selfdestruct_init_code(beneficiary),
        ..Default::default()
    });
    tx.sign_inplace(signer).await.unwrap();
    tx
}

/// Regression test for the spurious empty-account bug in batch execution.
///
/// The scenario exercises the `DestroyedModified` account state transition:
///
/// - **Block 1**: Deploy a contract (with value=0) that immediately
///   self-destructs. Under EIP-6780, it is truly destroyed (created +
///   selfdestruct in same tx). After end-of-tx cleanup the account is reset
///   to default values with `status = Destroyed`.
///
/// - **Block 2**: Send a 0-value tx to the destroyed contract address.
///   `transfer_value` calls `increase_account_balance(to, 0)` which calls
///   `get_account_mut` → `mark_modified` → `Destroyed` → `DestroyedModified`.
///   Crucially, the account info (balance, nonce, code_hash) stays at its
///   initial default values, so `acc_info_updated = false` and `info = None`.
///
/// When executed in batch, `initial_state_account.has_storage = false` because
/// the account didn't exist before the batch. Without the fix,
/// `removed_storage = true` would cause the skip-guard in
/// `get_state_transitions` to NOT skip the account, emitting an
/// `AccountUpdate { info: None, removed_storage: true }`.
/// `apply_account_updates_from_trie_batch` would then insert a default
/// (empty) `AccountState` into the state trie — a spurious leaf that doesn't
/// exist in the single-block path, corrupting the state root.
#[tokio::test]
async fn batch_selfdestruct_created_account_no_spurious_state() {
    // 1. Set up a test private key and derive the sender address.
    let sk = test_secret_key();
    let sender = sender_from_key(&sk);
    let signer: Signer = LocalSigner::new(sk).into();

    // 2. Create store A and blockchain A (for single-block validation).
    let (store_a, chain_id) = setup_store(sender).await;
    let blockchain_a = Blockchain::default_with_store(store_a.clone());

    let genesis_header = store_a.get_block_header(0).unwrap().unwrap();

    // The contract will be deployed at the CREATE address for (sender, nonce=0).
    let contract_address = calculate_create_address(sender, 0);

    // Use a fixed beneficiary for the selfdestruct.
    let beneficiary = Address::from_low_u64_be(0xBEEF);

    // 3. Build and sign tx1: deploy the self-destructing contract with value=0.
    //    After selfdestruct cleanup the account has balance=0, nonce=0, status=Destroyed.
    let tx1 = create_selfdestruct_deploy_tx(chain_id, 0, beneficiary, &signer).await;

    blockchain_a
        .add_transaction_to_pool(tx1)
        .await
        .expect("tx1 should enter pool");

    // 4. Build block 1 and validate it via single-block path.
    let block1 = build_block(&store_a, &blockchain_a, &genesis_header).await;
    blockchain_a
        .add_block(block1.clone())
        .expect("block1 should be valid (single-block)");
    store_a
        .forkchoice_update(vec![], 1, block1.hash(), None, None)
        .await
        .unwrap();

    // 5. Build and sign tx2: send 0-value tx to the destroyed contract.
    //    In the batch path this triggers Destroyed → DestroyedModified via
    //    get_account_mut, but balance/nonce/code stay at default values,
    //    leaving acc_info_updated = false.
    let mut tx2 = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id,
        nonce: 1,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: TEST_MAX_FEE_PER_GAS,
        gas_limit: TEST_GAS_LIMIT,
        to: TxKind::Call(contract_address),
        value: U256::zero(),
        data: Bytes::new(),
        ..Default::default()
    });
    tx2.sign_inplace(&signer).await.unwrap();

    blockchain_a
        .add_transaction_to_pool(tx2)
        .await
        .expect("tx2 should enter pool");

    // Remove Block 1's transactions from the pool so they don't interfere
    // with building Block 2 (the mempool doesn't auto-prune on add_block).
    blockchain_a
        .remove_block_transactions_from_pool(&block1)
        .expect("should remove block1 txs from pool");

    // 6. Build block 2 and validate it via single-block path.
    let block2 = build_block(&store_a, &blockchain_a, &block1.header).await;
    blockchain_a
        .add_block(block2.clone())
        .expect("block2 should be valid (single-block)");

    // 7. Create a fresh store B and re-execute both blocks in batch.
    //    This is the code path that was buggy before the fix.
    let (store_b, _) = setup_store(sender).await;
    let blockchain_b = Blockchain::default_with_store(store_b);

    let result = blockchain_b
        .add_blocks_in_batch(vec![block1, block2], CancellationToken::new())
        .await;

    assert!(
        result.is_ok(),
        "add_blocks_in_batch should succeed — got error: {:?}",
        result.err()
    );
}

/// Simpler variant: a single block with a self-destructing contract, executed
/// in batch. Ensures the basic batch path doesn't regress for single-block
/// batches containing selfdestruct.
#[tokio::test]
async fn batch_single_block_selfdestruct() {
    let sk = test_secret_key();
    let sender = sender_from_key(&sk);
    let signer: Signer = LocalSigner::new(sk).into();

    // Single-block path: build and validate.
    let (store_a, chain_id) = setup_store(sender).await;
    let blockchain_a = Blockchain::default_with_store(store_a.clone());
    let genesis_header = store_a.get_block_header(0).unwrap().unwrap();

    let beneficiary = Address::from_low_u64_be(0xBEEF);

    let tx = create_selfdestruct_deploy_tx(chain_id, 0, beneficiary, &signer).await;

    blockchain_a
        .add_transaction_to_pool(tx)
        .await
        .expect("tx should enter pool");

    let block1 = build_block(&store_a, &blockchain_a, &genesis_header).await;
    blockchain_a
        .add_block(block1.clone())
        .expect("block1 should be valid (single-block)");

    // Batch path: re-execute the same block on a fresh store.
    let (store_b, _) = setup_store(sender).await;
    let blockchain_b = Blockchain::default_with_store(store_b);

    let result = blockchain_b
        .add_blocks_in_batch(vec![block1], CancellationToken::new())
        .await;

    assert!(
        result.is_ok(),
        "single-block batch with selfdestruct should succeed — got error: {:?}",
        result.err()
    );
}
