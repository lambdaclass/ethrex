//! Transactions of blocks a reorg takes off the canonical chain go back to the
//! mempool.
//!
//! The engine handler removes a block's transactions from the pool when that
//! block becomes head. Seen on the Hegotá testnet: a node built a block with a
//! sender's next two transactions, made it head, then followed a competing
//! empty block at the same height. Both transactions were in neither chain nor
//! pool, so the sender's later transactions waited forever behind the gap.

use std::{fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain,
    fork_choice::apply_fork_choice,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    types::{
        Block, BlockHeader, DEFAULT_BUILDER_GAS_CEIL, EIP1559Transaction, ELASTICITY_MULTIPLIER,
        GenesisAccount, Transaction, TxKind,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_storage::{EngineType, Store};
use secp256k1::SecretKey;

const KEY: &str = "850643a0224065ecce3882673c21f56bcf6eef86274cc21cadff15930b59fc8c";

async fn setup(sender: Address) -> (Store, Blockchain, u64) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures/genesis/execution-api.json");
    let mut genesis: ethrex_common::types::Genesis =
        serde_json::from_reader(BufReader::new(File::open(path).unwrap())).unwrap();
    let chain_id = genesis.config.chain_id;
    genesis.alloc.insert(
        sender,
        GenesisAccount {
            balance: U256::from(10).pow(U256::from(20)),
            code: Bytes::new(),
            storage: Default::default(),
            nonce: 0,
        },
    );
    let mut store = Store::new("mempool-reorg.db", EngineType::InMemory).unwrap();
    store.add_initial_state(genesis).await.unwrap();
    let blockchain = Blockchain::default_with_store(store.clone());
    (store, blockchain, chain_id)
}

async fn transfer(chain_id: u64, nonce: u64, signer: &Signer) -> Transaction {
    let mut tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id,
        nonce,
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: 10_000_000_000,
        gas_limit: 100_000,
        to: TxKind::Call(Address::from_low_u64_be(0xAAAA)),
        value: U256::zero(),
        data: Bytes::new(),
        ..Default::default()
    });
    tx.sign_inplace(signer).await.unwrap();
    tx
}

/// A block on `parent` from whatever the pool holds. `fee_recipient` tells
/// sibling blocks apart.
fn build(
    store: &Store,
    blockchain: &Blockchain,
    parent: &BlockHeader,
    fee_recipient: H160,
) -> Block {
    let args = BuildPayloadArgs {
        parent: parent.hash(),
        timestamp: parent.timestamp + 12,
        fee_recipient,
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        slot_number: None,
        version: 3,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
        inclusion_list_transactions: None,
    };
    let block = create_payload(&args, store, Bytes::new()).unwrap();
    blockchain.build_payload(block).unwrap().payload
}

#[tokio::test]
async fn transactions_of_a_retracted_block_return_to_the_pool() {
    let sk = SecretKey::from_slice(&hex::decode(KEY).unwrap()).unwrap();
    let sender = LocalSigner::new(sk).address;
    let signer: Signer = LocalSigner::new(sk).into();
    let (store, blockchain, chain_id) = setup(sender).await;
    let genesis = store.get_block_header(0).unwrap().unwrap();

    // The competing block: empty, built before the pool holds anything.
    let empty = build(&store, &blockchain, &genesis, H160::repeat_byte(0xBB));
    assert!(empty.body.transactions.is_empty());
    let empty_hash = empty.hash();
    blockchain.add_block(empty).unwrap();

    // The block that will lose: it carries the sender's first two transactions.
    let first = transfer(chain_id, 0, &signer).await;
    let second = transfer(chain_id, 1, &signer).await;
    blockchain
        .add_transaction_to_pool(first.clone())
        .await
        .unwrap();
    blockchain
        .add_transaction_to_pool(second.clone())
        .await
        .unwrap();
    let full = build(&store, &blockchain, &genesis, H160::repeat_byte(0xAA));
    assert_eq!(full.body.transactions.len(), 2);
    let full_hash = full.hash();
    blockchain.add_block(full.clone()).unwrap();

    // It becomes head, and the engine handler prunes its transactions.
    apply_fork_choice(&store, full_hash, H256::zero(), H256::zero(), None)
        .await
        .unwrap();
    blockchain
        .remove_block_transactions_from_pool(&full)
        .unwrap();
    let in_pool = |tx: &Transaction| {
        blockchain
            .mempool
            .contains_tx(tx.hash(&NativeCrypto))
            .unwrap()
    };
    assert!(!in_pool(&first) && !in_pool(&second));

    // Their first submission queued both for peers; drain that, as the p2p
    // broadcaster would, so the check below sees only what reinjection queues.
    blockchain
        .mempool
        .remove_broadcasted_txs(&[first.hash(&NativeCrypto), second.hash(&NativeCrypto)])
        .unwrap();

    // The head moves to the empty sibling: both transactions are retracted.
    apply_fork_choice(&store, empty_hash, H256::zero(), H256::zero(), None)
        .await
        .unwrap();
    let reinjected = blockchain
        .reinject_retracted_transactions(full_hash)
        .await
        .unwrap();
    assert_eq!(reinjected, 2);
    assert!(
        in_pool(&first) && in_pool(&second),
        "the sender has no gap left"
    );
    // Never queued for peers: a reorg must not publish what was private before.
    let queued = |blockchain: &Blockchain| -> Vec<H256> {
        blockchain
            .mempool
            .get_txs_for_broadcast()
            .unwrap()
            .iter()
            .map(|tx| tx.hash(&NativeCrypto))
            .collect()
    };
    assert!(
        !queued(&blockchain).contains(&first.hash(&NativeCrypto)),
        "a re-admitted transaction must not be queued for broadcast"
    );

    // A head that is still canonical retracted nothing.
    assert_eq!(
        blockchain
            .reinject_retracted_transactions(empty_hash)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn a_retracted_transaction_the_new_chain_includes_stays_out() {
    let sk = SecretKey::from_slice(&hex::decode(KEY).unwrap()).unwrap();
    let sender = LocalSigner::new(sk).address;
    let signer: Signer = LocalSigner::new(sk).into();
    let (store, blockchain, chain_id) = setup(sender).await;
    let genesis = store.get_block_header(0).unwrap().unwrap();

    // Two siblings carrying the same transaction.
    let tx = transfer(chain_id, 0, &signer).await;
    blockchain
        .add_transaction_to_pool(tx.clone())
        .await
        .unwrap();
    let a = build(&store, &blockchain, &genesis, H160::repeat_byte(0xAA));
    let b = build(&store, &blockchain, &genesis, H160::repeat_byte(0xBB));
    assert_eq!(a.body.transactions.len(), 1);
    assert_eq!(b.body.transactions.len(), 1);
    let (a_hash, b_hash) = (a.hash(), b.hash());
    blockchain.add_block(a.clone()).unwrap();
    blockchain.add_block(b.clone()).unwrap();

    apply_fork_choice(&store, a_hash, H256::zero(), H256::zero(), None)
        .await
        .unwrap();
    blockchain.remove_block_transactions_from_pool(&a).unwrap();
    apply_fork_choice(&store, b_hash, H256::zero(), H256::zero(), None)
        .await
        .unwrap();
    blockchain.remove_block_transactions_from_pool(&b).unwrap();

    // The new head already contains it, so admission refuses it on its nonce.
    assert_eq!(
        blockchain
            .reinject_retracted_transactions(a_hash)
            .await
            .unwrap(),
        0
    );
    assert!(
        !blockchain
            .mempool
            .contains_tx(tx.hash(&NativeCrypto))
            .unwrap()
    );
}
