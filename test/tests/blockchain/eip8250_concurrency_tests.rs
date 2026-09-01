//! EIP-8250 concurrency through the real builder.
//!
//! Two frame transactions from one contract sender on disjoint nonce keys, submitted to
//! the mempool and built into a block by `fill_transactions` — the path where the devnet
//! intermittently loses both of them with `VERIFY frame reverted`. The same pair executes
//! cleanly straight through the VM (`two_keyed_transactions_from_one_contract_sender_both_execute`
//! in the levm suite), so anything that fails here is the block environment, not the
//! transactions.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use bytes::Bytes;
use ethrex_blockchain::fork_choice::apply_fork_choice;
use ethrex_blockchain::payload::{BuildPayloadArgs, create_payload};
use ethrex_blockchain::{Blockchain, BlockchainOptions};
use ethrex_common::evm::calculate_create_address;
use ethrex_common::types::{
    EIP1559Transaction, ELASTICITY_MULTIPLIER, Frame, FrameMode, FrameTransaction, Genesis,
    GenesisAccount, Transaction, TxKind,
};
use ethrex_common::{Address, H160, H256, U256};
use ethrex_crypto::NativeCrypto;
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_storage::{EngineType, Store};
use secp256k1::SecretKey;

fn test_secret_key() -> SecretKey {
    // The l1-hegota genesis funds this key's address.
    SecretKey::from_slice(
        &hex::decode("941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e").unwrap(),
    )
    .unwrap()
}

const DEFAULT_BUILDER_GAS_CEIL: u64 = 30_000_000;
const BLOCK_SLOT: u64 = 1;
const TEST_MAX_FEE_PER_GAS: u64 = 1_000_000_000;

/// The contract sender: its whole runtime is `APPROVE(3)`, so it authorizes and pays for
/// any transaction that names it. This is what makes a contract sender able to have two
/// transactions in flight at once — there is no signature to bind them to an order.
const APPROVE_BOTH_CODE: &[u8] = &[0x60, 0x03, 0x60, 0x00, 0x60, 0x00, 0xAA];
const FRAME_SENDER: Address = Address::repeat_byte(0xC5);
/// EIP-8037 STATE_BYTES_PER_NEW_ACCOUNT * CPSB: what a frame pays to create the account
/// it funds.
const NEW_ACCOUNT_STATE_GAS: u64 = 120 * 1530;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

async fn setup_store(store_name: &str) -> (Store, u64) {
    let file = File::open(workspace_root().join("fixtures/genesis/l1-hegota.json"))
        .expect("open l1-hegota genesis");
    let mut genesis: Genesis =
        serde_json::from_reader(BufReader::new(file)).expect("parse l1-hegota genesis");
    genesis.alloc.insert(
        FRAME_SENDER,
        GenesisAccount {
            code: Bytes::from_static(APPROVE_BOTH_CODE),
            storage: BTreeMap::new(),
            balance: U256::from(10u64).pow(U256::from(20u64)),
            nonce: 0,
        },
    );
    let chain_id = genesis.config.chain_id;
    let mut store = Store::new(store_name, EngineType::InMemory).expect("build in-memory store");
    store
        .add_initial_state(genesis)
        .await
        .expect("add genesis state");
    (store, chain_id)
}

/// One keyed transaction: a self-verifying frame that approves, then a SENDER frame
/// funding a fresh address. The recipient differs per key so neither transaction's
/// account-creation charge depends on the other having run.
fn keyed_tx(chain_id: u64, index: u64) -> Transaction {
    Transaction::FrameTransaction(FrameTransaction {
        chain_id,
        nonce_keys: vec![U256::from(0x8250_0000u64 + index)],
        nonce_seq: 0,
        sender: FRAME_SENDER,
        frames: vec![
            Frame {
                mode: FrameMode::Verify as u8,
                flags: 0x03,
                target: Some(FRAME_SENDER),
                gas_limit: 80_000,
                state_limit: 0,
                value: U256::zero(),
                data: Bytes::new(),
            },
            Frame {
                mode: FrameMode::Sender as u8,
                flags: 0,
                target: Some(Address::from_low_u64_be(0xBEEF_0000 + index)),
                gas_limit: 30_000,
                state_limit: NEW_ACCOUNT_STATE_GAS,
                value: U256::from(100u64),
                data: Bytes::new(),
            },
        ],
        signatures: vec![],
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: TEST_MAX_FEE_PER_GAS,
        ..Default::default()
    })
}

#[tokio::test]
async fn two_keyed_transactions_from_one_contract_sender_build_into_one_block() {
    let (store, chain_id) = setup_store("eip8250-concurrency").await;
    let blockchain = Blockchain::new(store.clone(), BlockchainOptions::default());
    let genesis_header = store.get_block_header(0).unwrap().unwrap();

    for index in 0..2 {
        blockchain
            .add_transaction_to_pool(keyed_tx(chain_id, index))
            .await
            .unwrap_or_else(|e| panic!("keyed transaction {index} must be admitted: {e}"));
    }

    let args = BuildPayloadArgs {
        parent: genesis_header.hash(),
        timestamp: genesis_header.timestamp + 12,
        fee_recipient: H160::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        slot_number: Some(BLOCK_SLOT),
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
        inclusion_list_transactions: None,
    };
    let payload = create_payload(&args, &store, Bytes::new()).unwrap();
    let result = blockchain
        .build_payload(payload)
        .expect("the payload must build");

    assert_eq!(
        result.payload.body.transactions.len(),
        2,
        "both keyed transactions must be included — this is the concurrency EIP-8250 \
         exists for, and the builder dropping them is the devnet's intermittent \
         'GONE FROM THE POOL'; receipts={:?}",
        result.receipts
    );
    assert!(
        result.receipts.iter().all(|r| r.succeeded),
        "both must succeed; receipts={:?}",
        result.receipts
    );
}

/// The same pair, but with the sender contract **deployed by a transaction** rather than
/// written into genesis — which is how the devnet gets it, and the one structural
/// difference between the passing test above and the run that intermittently loses both
/// transactions.
///
/// The contract is deployed in block 1, that block is made canonical, and only then are
/// the keyed transactions submitted and block 2 built. If a frame resolves a
/// recently-deployed target to empty code, it takes the default-code branch instead of
/// calling the contract, the VERIFY frame never approves, and the builder discards a
/// transaction it had already acknowledged.
#[tokio::test]
async fn a_deployed_contract_sender_is_not_resolved_as_codeless() {
    let (store, chain_id) = setup_store("eip8250-deployed-sender").await;
    let blockchain = Blockchain::new(store.clone(), BlockchainOptions::default());
    let genesis_header = store.get_block_header(0).unwrap().unwrap();

    // PUSH1 len; PUSH1 12; PUSH1 0; CODECOPY; PUSH1 len; PUSH1 0; RETURN ‖ runtime
    let runtime = APPROVE_BOTH_CODE;
    let mut init = vec![
        0x60,
        runtime.len() as u8,
        0x60,
        0x0C,
        0x60,
        0x00,
        0x39,
        0x60,
        runtime.len() as u8,
        0x60,
        0x00,
        0xF3,
    ];
    init.extend_from_slice(runtime);

    let signer: Signer = LocalSigner::new(test_secret_key()).into();
    let deployer = signer.address();
    let mut deploy = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id,
        nonce: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: TEST_MAX_FEE_PER_GAS,
        gas_limit: 1_000_000,
        to: TxKind::Create,
        value: U256::from(10u64).pow(U256::from(18u64)),
        data: Bytes::from(init),
        ..Default::default()
    });
    deploy.sign_inplace(&signer).await.unwrap();

    let args = BuildPayloadArgs {
        parent: genesis_header.hash(),
        timestamp: genesis_header.timestamp + 12,
        fee_recipient: H160::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        slot_number: Some(BLOCK_SLOT),
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
        inclusion_list_transactions: None,
    };
    let payload = create_payload(&args, &store, Bytes::new()).unwrap();
    let built = blockchain
        .build_payload_with_transactions(payload, vec![deploy])
        .expect("the deployment block must build");
    let block1 = built.payload;
    let block1_hash = block1.hash();
    blockchain.add_block(block1.clone()).unwrap();
    apply_fork_choice(&store, block1_hash, block1_hash, block1_hash, None)
        .await
        .expect("the deployment block must become canonical");

    let contract = calculate_create_address(deployer, 0);
    let code = store
        .get_code_by_account_address(1, contract)
        .await
        .unwrap()
        .unwrap_or_default();
    assert_eq!(
        code.code().to_vec(),
        runtime.to_vec(),
        "the deployment must have installed the runtime at {contract:#x}"
    );

    for index in 0..2 {
        let mut tx = keyed_tx(chain_id, index);
        if let Transaction::FrameTransaction(ft) = &mut tx {
            ft.sender = contract;
            ft.frames[0].target = Some(contract);
        }
        blockchain
            .add_transaction_to_pool(tx)
            .await
            .unwrap_or_else(|e| panic!("keyed transaction {index} must be admitted: {e}"));
    }

    let args2 = BuildPayloadArgs {
        parent: block1_hash,
        timestamp: block1.header.timestamp + 12,
        slot_number: Some(BLOCK_SLOT + 1),
        ..args
    };
    let payload2 = create_payload(&args2, &store, Bytes::new()).unwrap();
    let result = blockchain
        .build_payload(payload2)
        .expect("the second payload must build");

    assert_eq!(
        result.payload.body.transactions.len(),
        2,
        "both keyed transactions from the deployed contract must be included; \
         receipts={:?}",
        result.receipts
    );
}

/// The builder does not execute against the chain head: it executes against the parent of
/// the payload it is building, and it keeps rebuilding that payload as transactions
/// arrive. A frame transaction admitted after that parent was fixed is therefore run
/// against a state older than the one it was admitted against.
///
/// Here the sender contract is deployed in block 1, but the payload being built is rooted
/// at genesis, so the frame resolves its target to an account with no code and the VERIFY
/// frame fails. That failure says nothing about the transaction — it is valid against the
/// head and against every later block — so the builder must leave it in the pool.
/// Evicting it is how the devnet lost roughly one concurrent pair in three, with
/// `eth_sendRawTransaction` having already returned a hash to the sender.
#[tokio::test]
async fn a_frame_tx_is_kept_when_the_builder_runs_it_against_a_parent_that_predates_its_target() {
    let (store, chain_id) = setup_store("eip8250-stale-parent").await;
    let blockchain = Blockchain::new(store.clone(), BlockchainOptions::default());
    let genesis_header = store.get_block_header(0).unwrap().unwrap();

    let runtime = APPROVE_BOTH_CODE;
    let mut init = vec![
        0x60,
        runtime.len() as u8,
        0x60,
        0x0C,
        0x60,
        0x00,
        0x39,
        0x60,
        runtime.len() as u8,
        0x60,
        0x00,
        0xF3,
    ];
    init.extend_from_slice(runtime);

    let signer: Signer = LocalSigner::new(test_secret_key()).into();
    let deployer = signer.address();
    let mut deploy = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id,
        nonce: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: TEST_MAX_FEE_PER_GAS,
        gas_limit: 1_000_000,
        to: TxKind::Create,
        value: U256::from(10u64).pow(U256::from(18u64)),
        data: Bytes::from(init),
        ..Default::default()
    });
    deploy.sign_inplace(&signer).await.unwrap();

    let args = BuildPayloadArgs {
        parent: genesis_header.hash(),
        timestamp: genesis_header.timestamp + 12,
        fee_recipient: H160::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        slot_number: Some(BLOCK_SLOT),
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
        inclusion_list_transactions: None,
    };
    let payload = create_payload(&args, &store, Bytes::new()).unwrap();
    let built = blockchain
        .build_payload_with_transactions(payload, vec![deploy])
        .expect("the deployment block must build");
    let block1 = built.payload;
    let block1_hash = block1.hash();
    blockchain.add_block(block1.clone()).unwrap();
    apply_fork_choice(&store, block1_hash, block1_hash, block1_hash, None)
        .await
        .expect("the deployment block must become canonical");

    let contract = calculate_create_address(deployer, 0);
    let mut tx = keyed_tx(chain_id, 0);
    if let Transaction::FrameTransaction(ft) = &mut tx {
        ft.sender = contract;
        ft.frames[0].target = Some(contract);
    }
    let tx_hash = tx.hash(&NativeCrypto);
    blockchain
        .add_transaction_to_pool(tx)
        .await
        .expect("the transaction is valid against the head, so it is admitted");

    // Build a SECOND payload rooted at genesis — the parent from before the deployment.
    // This is the builder still working a slot whose parent was fixed before the contract
    // existed, which is what the devnet hit.
    let stale = BuildPayloadArgs {
        parent: genesis_header.hash(),
        timestamp: genesis_header.timestamp + 24,
        slot_number: Some(BLOCK_SLOT + 1),
        ..args
    };
    let stale_payload = create_payload(&stale, &store, Bytes::new()).unwrap();
    let result = blockchain
        .build_payload(stale_payload)
        .expect("the stale-parent payload still builds");

    assert!(
        result.payload.body.transactions.is_empty(),
        "the frame tx cannot execute against a parent where its target has no code"
    );
    assert!(
        blockchain
            .mempool
            .get_transaction_by_hash(tx_hash)
            .is_ok_and(|t| t.is_some()),
        "the transaction must still be pooled: failing against one parent says nothing \
         about its validity, and the node already returned its hash to the sender"
    );
}
