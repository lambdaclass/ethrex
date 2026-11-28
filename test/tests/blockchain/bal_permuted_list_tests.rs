//! A block access list supplied by a peer in another order than the canonical one.
//!
//! The header's commitment is computed over a re-sorted encoding, so a permuted copy of a
//! block's canonical list matches it, while the parallel path seeds each transaction's
//! starting state by binary search over the list. A supplied list must be canonically
//! ordered before its commitment counts, and the parallel path checks the order before
//! seeding.

use std::{fs::File, io::BufReader, path::PathBuf, sync::Arc};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    types::{
        Block, DEFAULT_BUILDER_GAS_CEIL, EIP1559Transaction, ELASTICITY_MULTIPLIER, Genesis,
        GenesisAccount, Transaction, TxKind,
        block_access_list::{AccountChanges, BlockAccessList},
    },
    validate_supplied_block_access_list,
};
use ethrex_crypto::NativeCrypto;
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_storage::{DB_COMMIT_THRESHOLD, EngineType, Store};
use secp256k1::SecretKey;

const SINK: Address = H160([0x22; 20]);
const MAX_FEE_PER_GAS: u64 = 10_000_000_000;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

async fn setup_store(sender: Address) -> Store {
    let file = File::open(workspace_root().join("fixtures/genesis/l1-bal.json"))
        .expect("open l1-bal genesis");
    let mut genesis: Genesis =
        serde_json::from_reader(BufReader::new(file)).expect("parse l1-bal genesis");
    genesis.alloc.insert(
        sender,
        GenesisAccount {
            code: Bytes::new(),
            storage: Default::default(),
            balance: U256::from(10u64).pow(U256::from(18u64)),
            nonce: 0,
        },
    );
    let mut store = Store::new("store.db", EngineType::InMemory).expect("in-memory store");
    store.add_initial_state(genesis).await.expect("genesis");
    store
}

/// A valid block of three transfers from one sender (nonces 0, 1 and 2), built by the
/// builder, with the canonical list it produced.
async fn three_transaction_block(sender: &Signer) -> (Block, BlockAccessList) {
    let store = setup_store(sender.address()).await;
    let chain_id = store.get_chain_config().chain_id;
    let mut txs = Vec::new();
    for nonce in 0..3 {
        let mut tx = Transaction::EIP1559Transaction(EIP1559Transaction {
            chain_id,
            nonce,
            max_priority_fee_per_gas: 0,
            max_fee_per_gas: MAX_FEE_PER_GAS,
            gas_limit: 100_000,
            to: TxKind::Call(SINK),
            value: U256::one(),
            ..Default::default()
        });
        tx.sign_inplace(sender).await.unwrap();
        txs.push(tx);
    }
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
    let payload = create_payload(&args, &store, Bytes::new()).unwrap();
    let result = blockchain
        .build_payload_with_transactions(payload, txs)
        .expect("the block builds");
    assert_eq!(result.payload.body.transactions.len(), 3);
    (
        result.payload,
        result.block_access_list.expect("amsterdam BAL"),
    )
}

/// The canonical list with the sender's first two nonce changes swapped.
fn permuted(canonical: &BlockAccessList, sender: Address) -> BlockAccessList {
    let mut accounts: Vec<AccountChanges> = canonical.accounts().to_vec();
    let account = accounts
        .iter_mut()
        .find(|a| a.address == sender)
        .expect("the sender is in the list");
    assert!(
        account.nonce_changes.len() >= 3,
        "three nonce changes expected"
    );
    account.nonce_changes.swap(0, 1);
    BlockAccessList::from_accounts(accounts)
}

fn parallel_blockchain(store: Store) -> Blockchain {
    Blockchain::new(
        store,
        BlockchainOptions {
            bal_parallel_exec_enabled: true,
            ..Default::default()
        },
    )
}

/// The permuted list passes the commitment alone but not the supplied-list validation
/// that full sync applies before keeping a peer's list.
#[tokio::test]
async fn permuted_list_matches_the_commitment_but_is_not_accepted() {
    let signer: Signer = LocalSigner::new(SecretKey::from_slice(&[7; 32]).unwrap()).into();
    let (block, canonical) = three_transaction_block(&signer).await;
    let commitment = block.header.block_access_list_hash;
    let permuted = permuted(&canonical, signer.address());

    assert!(permuted.matches_commitment(commitment, &NativeCrypto));
    assert!(permuted.validate_ordering().is_err());
    validate_supplied_block_access_list(&canonical, commitment, &NativeCrypto)
        .expect("the canonical list is accepted");
    validate_supplied_block_access_list(&permuted, commitment, &NativeCrypto)
        .expect_err("the permuted list is rejected");
}

/// Importing the block with the permuted list, as full sync does, succeeds: the parallel
/// path refuses the list before seeding, and the block runs again without it.
#[tokio::test]
async fn valid_block_with_a_permuted_list_is_imported() {
    let signer: Signer = LocalSigner::new(SecretKey::from_slice(&[7; 32]).unwrap()).into();
    let (block, canonical) = three_transaction_block(&signer).await;
    let permuted = permuted(&canonical, signer.address());

    let blockchain = parallel_blockchain(setup_store(signer.address()).await);
    blockchain
        .add_block_pipeline_bounded(block.clone(), Some(Arc::new(permuted)), DB_COMMIT_THRESHOLD)
        .expect("a valid block must be imported whatever list a peer sent for it");

    let blockchain = parallel_blockchain(setup_store(signer.address()).await);
    blockchain
        .add_block_pipeline_bounded(block, Some(Arc::new(canonical)), DB_COMMIT_THRESHOLD)
        .expect("the canonical list still imports the block");
}
