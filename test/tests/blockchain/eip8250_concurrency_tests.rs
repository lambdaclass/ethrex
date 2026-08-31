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
use ethrex_blockchain::payload::{BuildPayloadArgs, create_payload};
use ethrex_blockchain::{Blockchain, BlockchainOptions};
use ethrex_common::types::{
    ELASTICITY_MULTIPLIER, Frame, FrameMode, FrameTransaction, Genesis, GenesisAccount, Transaction,
};
use ethrex_common::{Address, H160, H256, U256};
use ethrex_storage::{EngineType, Store};

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
