//! EIP-8312 pool-level admission: the per-UTXO-index identity, through a real
//! mempool with real spends.
//!
//! A vault-sender spend has no meaningful sender and no nonce — every one of them
//! is sent from `0x8312` — so the pool cannot key it the way it keys an ordinary
//! transaction. Its conflict domain is the set of input indices it claims, and
//! that map is **global across senders**: two spends of one index conflict whoever
//! submitted them, because the spent bit is not set until inclusion and nothing
//! else stops the second one being pooled.
//!
//! The failure mode this guards is specific and has bitten this devnet before in
//! another guise: if vault-sender transactions went into a per-sender map, they
//! would all collide on `0x8312` and the network would accept exactly one pending
//! spend at a time. The concurrency test below is the one that would catch that.
//!
//! These cases need a real pool rather than the unit-level checks elsewhere,
//! because admission does more than shape validation: it pre-verifies each input's
//! opening against the committed root in head state, and only then consults the
//! per-index map under the write lock.

use std::{collections::BTreeMap, fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_blockchain::{Blockchain, error::MempoolError};
use ethrex_common::{
    Address, Bytes as CommonBytes, H256, U256,
    types::{
        Frame, FrameMode, FrameSignature, FrameTransaction, Genesis, GenesisAccount,
        SLOT_NEXT_INDEX, Spend, SpendInput, SpendOutput, Transaction, merkle_proof, merkle_root,
        opening_leaf, ring_slot, utxo_vault,
    },
};
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::{EngineType, Store};
use ethrex_vm::system_contracts::UTXO_VAULT_RUNTIME_BYTECODE;
use k256::ecdsa::SigningKey;

/// EIP-8141 signature scheme id for secp256k1.
const FRAME_SIG_SCHEME_SECP256K1: u8 = 1;
/// The UTXOs are committed in genesis, i.e. block 0. Admission treats an input as
/// not-yet-spendable while `creation_block >= head + 1`, so a genesis-committed
/// UTXO is already spendable against a genesis-only store.
const CREATION_BLOCK: u64 = 0;
const INPUT_VALUE_WEI: u64 = 1_000_000_000_000_000_000;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn key_and_address(seed: u8) -> (SigningKey, Address) {
    let key = SigningKey::from_bytes(&[seed; 32].into()).unwrap();
    let uncompressed = key.verifying_key().to_encoded_point(false);
    let pub_hash = ethrex_crypto::keccak::keccak_hash(&uncompressed.as_bytes()[1..]);
    (key, Address::from_slice(&pub_hash[12..]))
}

fn sign_digest(key: &SigningKey, digest: H256, signer: Address) -> FrameSignature {
    let (raw, recovery_id) = key.sign_prehash_recoverable(digest.as_bytes()).unwrap();
    let mut bytes = vec![0u8; 65];
    bytes[0] = recovery_id.to_byte();
    bytes[1..33].copy_from_slice(&raw.to_bytes()[..32]);
    bytes[33..65].copy_from_slice(&raw.to_bytes()[32..]);
    FrameSignature {
        scheme: FRAME_SIG_SCHEME_SECP256K1,
        signer: Some(signer),
        msg: CommonBytes::copy_from_slice(digest.as_bytes()),
        signature: CommonBytes::from(bytes),
    }
}

/// The two UTXOs every test in this file spends: index 0 owned by actor A, index 1
/// by actor B, both committed in one genesis openings tree.
struct Utxos {
    keys: Vec<SigningKey>,
    owners: Vec<Address>,
    leaves: Vec<H256>,
    source: Address,
}

fn utxos() -> Utxos {
    let (key_a, owner_a) = key_and_address(0x31);
    let (key_b, owner_b) = key_and_address(0x32);
    let source = Address::from_low_u64_be(0x50_11);
    let value = U256::from(INPUT_VALUE_WEI);
    let leaves = vec![
        opening_leaf(0, source, owner_a, value),
        opening_leaf(1, source, owner_b, value),
    ];
    Utxos {
        keys: vec![key_a, key_b],
        owners: vec![owner_a, owner_b],
        leaves,
        source,
    }
}

/// A store whose genesis already carries the vault, the openings root committing
/// to both UTXOs, and the value the vault custodies for them.
///
/// Preloading rather than producing the deposits keeps these tests about admission:
/// the pool only needs the committed root to be readable from head state.
async fn setup_store(utxos: &Utxos) -> (Store, u64) {
    let file = File::open(workspace_root().join("fixtures/genesis/l1-hegota.json"))
        .expect("open l1-hegota genesis");
    let mut genesis: Genesis =
        serde_json::from_reader(BufReader::new(file)).expect("parse l1-hegota genesis");
    genesis.config.utxo_frames_time = Some(0);
    let chain_id = genesis.config.chain_id;

    let root = merkle_root(&utxos.leaves);
    let mut storage = BTreeMap::new();
    storage.insert(
        ring_slot(CREATION_BLOCK),
        U256::from_big_endian(root.as_bytes()),
    );
    storage.insert(U256::from(SLOT_NEXT_INDEX), U256::from(utxos.leaves.len()));
    genesis.alloc.insert(
        utxo_vault(),
        GenesisAccount {
            code: Bytes::from_static(&UTXO_VAULT_RUNTIME_BYTECODE),
            storage,
            balance: U256::from(INPUT_VALUE_WEI) * U256::from(utxos.leaves.len()),
            nonce: 1,
        },
    );

    let mut store = Store::new("store.db", EngineType::InMemory).expect("in-memory store");
    store
        .add_initial_state(genesis)
        .await
        .expect("add genesis state");
    (store, chain_id)
}

/// A self-funded spend of one UTXO: vault-sender, one UTXO frame, no payer.
///
/// `payee_tag` distinguishes otherwise-identical spends of the same input, so a
/// test can submit two genuinely different transactions competing for one index.
fn spend_tx(
    utxos: &Utxos,
    which: usize,
    chain_id: u64,
    payee_tag: u64,
    max_fee_per_gas: u64,
) -> FrameTransaction {
    let value = U256::from(INPUT_VALUE_WEI);
    let owner = utxos.owners[which];
    let spend = Spend {
        actors: vec![owner],
        inputs: vec![SpendInput {
            index: which as u64,
            creation_block: CREATION_BLOCK,
            source: utxos.source,
            recipient: owner,
            value,
            position: which as u64,
            siblings: merkle_proof(&utxos.leaves, which).expect("proof"),
            batch_siblings: vec![],
        }],
        // A modest account output plus the change entry (signed with zero).
        utxo_outs: vec![SpendOutput {
            recipient: owner,
            value: U256::zero(),
        }],
        account_outs: vec![SpendOutput {
            recipient: Address::from_low_u64_be(0xC0FFEE + payee_tag),
            value: U256::from(1_000_000_000_000_000u64),
        }],
        change_index: 0,
        payer: CommonBytes::new(),
        max_fee_per_gas: U256::from(1_000_000u64),
        max_priority_fee_per_gas: U256::from(1_000_000u64),
        max_gas_limit: 30_000_000,
    };

    let mut tx = FrameTransaction {
        chain_id,
        nonce_keys: vec![],
        nonce_seq: 0,
        sender: utxo_vault(),
        frames: vec![Frame {
            mode: FrameMode::Utxo as u8,
            flags: 0,
            target: None,
            gas_limit: 3_000_000,
            state_limit: 0,
            value: U256::zero(),
            data: CommonBytes::from(spend.encode_to_vec()),
        }],
        signatures: vec![],
        // Both fee fields must rise for a replacement to count as a fee bump, so the
        // priority fee scales with the bid rather than being pinned.
        max_priority_fee_per_gas: max_fee_per_gas / 2,
        max_fee_per_gas,
        ..Default::default()
    };
    let digest = spend.spend_hash(tx.chain_id);
    tx.signatures
        .push(sign_digest(&utxos.keys[which], digest, utxos.owners[which]));
    tx
}

async fn pool_with_utxos() -> (Blockchain, Utxos, u64) {
    let utxos = utxos();
    let (store, chain_id) = setup_store(&utxos).await;
    (Blockchain::default_with_store(store), utxos, chain_id)
}

#[tokio::test]
async fn a_self_funded_spend_is_admitted_to_the_pool() {
    // The precondition every other case here rests on: a genuine self-funded spend
    // clears admission, including the opening pre-verification against head state.
    // If this ever fails, the rejections below stop meaning anything.
    let (blockchain, utxos, chain_id) = pool_with_utxos().await;
    let tx = Transaction::FrameTransaction(spend_tx(&utxos, 0, chain_id, 0, 1_000));
    blockchain
        .add_transaction_to_pool(tx)
        .await
        .expect("a valid self-funded spend must be admitted");
}

#[tokio::test]
async fn a_second_transaction_cannot_claim_a_pending_utxo_index() {
    // The per-index rule. Two different transactions spending the same input: the
    // first holds the index, the second is refused. Without the per-index map both
    // would sit in the pool and the builder would try to include a double spend.
    let (blockchain, utxos, chain_id) = pool_with_utxos().await;

    let first = Transaction::FrameTransaction(spend_tx(&utxos, 0, chain_id, 0, 1_000));
    let first_hash = blockchain
        .add_transaction_to_pool(first)
        .await
        .expect("the first spend must be admitted");

    // Same input, different payee, same fees: not a fee bump, so it must be refused
    // rather than replace.
    let competitor = Transaction::FrameTransaction(spend_tx(&utxos, 0, chain_id, 1, 1_000));
    let result = blockchain.add_transaction_to_pool(competitor).await;
    assert!(
        matches!(
            result,
            Err(MempoolError::FrameTxSenderAlreadyPending)
                | Err(MempoolError::UnderpricedReplacement)
        ),
        "a second claim on a pending index must be refused, got {result:?}"
    );

    // And the incumbent is still there — the refusal must not have evicted it.
    assert!(
        blockchain
            .mempool
            .get_transaction_by_hash(first_hash)
            .is_ok(),
        "the refused competitor must leave the incumbent pooled"
    );
}

#[tokio::test]
async fn disjoint_utxo_indices_are_pending_concurrently() {
    // The stall class this guards. Every vault-sender transaction shares sender
    // 0x8312, so keying them per sender would cap the whole network at one pending
    // spend. Two spends of *different* indices must therefore both be pending — and
    // this is the test that fails if that keying is ever reintroduced.
    let (blockchain, utxos, chain_id) = pool_with_utxos().await;

    let a = blockchain
        .add_transaction_to_pool(Transaction::FrameTransaction(spend_tx(
            &utxos, 0, chain_id, 0, 1_000,
        )))
        .await
        .expect("the first spend must be admitted");
    let b = blockchain
        .add_transaction_to_pool(Transaction::FrameTransaction(spend_tx(
            &utxos, 1, chain_id, 1, 1_000,
        )))
        .await
        .expect("a spend of a DIFFERENT index must be admitted concurrently");

    assert_ne!(a, b, "the two spends must be distinct transactions");
    for hash in [a, b] {
        assert!(
            blockchain
                .mempool
                .contains_tx(hash)
                .expect("contains_tx must succeed"),
            "both disjoint-index spends must remain pooled"
        );
    }
}

#[tokio::test]
async fn a_fee_bump_replaces_the_pending_claim_on_an_index() {
    // Replacement, and the release that comes with it: a strictly higher bid for
    // the same index takes the claim over, and the predecessor leaves the pool. The
    // per-index map must end up pointing at the newcomer, not at a hash that is no
    // longer pooled.
    let (blockchain, utxos, chain_id) = pool_with_utxos().await;

    let low = blockchain
        .add_transaction_to_pool(Transaction::FrameTransaction(spend_tx(
            &utxos, 0, chain_id, 0, 1_000,
        )))
        .await
        .expect("the low-fee spend must be admitted");

    let high = blockchain
        .add_transaction_to_pool(Transaction::FrameTransaction(spend_tx(
            &utxos, 0, chain_id, 0, 2_000,
        )))
        .await
        .expect("a strictly higher bid on the same index must replace");

    assert_ne!(low, high, "the replacement must be a different transaction");
    assert!(
        !blockchain
            .mempool
            .contains_tx(low)
            .expect("contains_tx must succeed"),
        "the replaced spend must be evicted from the pool"
    );
    assert!(
        blockchain
            .mempool
            .contains_tx(high)
            .expect("contains_tx must succeed"),
        "the replacement must be pooled"
    );

    // The claim is held by the replacement now: a third, lower bid must be refused
    // against it rather than sneaking in because the map still named the evicted tx.
    let result = blockchain
        .add_transaction_to_pool(Transaction::FrameTransaction(spend_tx(
            &utxos, 0, chain_id, 2, 1_500,
        )))
        .await;
    assert!(
        result.is_err(),
        "a bid below the current claimant must be refused, got {result:?}"
    );
}

#[tokio::test]
async fn a_forged_opening_is_refused_at_admission() {
    // Pool-level counterpart to the execution-level forged-proof test: admission
    // pre-verifies the opening against the committed root, so a spend claiming a
    // value the leaf does not commit to never reaches the pool at all. This is what
    // keeps a builder from spending gas discovering it.
    let (blockchain, utxos, chain_id) = pool_with_utxos().await;
    let mut tx = spend_tx(&utxos, 0, chain_id, 0, 1_000);

    // Inflate the claimed input value; the leaf commits to the real one, so the
    // proof can no longer fold to the stored root.
    let mut spend = Spend::decode_frame_data(&tx.frames[0].data).expect("decode");
    spend.inputs[0].value = U256::from(INPUT_VALUE_WEI) * U256::from(2u64);
    tx.frames[0].data = CommonBytes::from(spend.encode_to_vec());
    let digest = spend.spend_hash(tx.chain_id);
    tx.signatures = vec![sign_digest(&utxos.keys[0], digest, utxos.owners[0])];

    let result = blockchain
        .add_transaction_to_pool(Transaction::FrameTransaction(tx))
        .await;
    match result {
        Err(MempoolError::InvalidFrameTransaction(msg)) => assert!(
            msg.contains("does not prove against the committed root"),
            "expected the opening check to reject it, got: {msg}"
        ),
        other => panic!("expected an opening-verification rejection, got {other:?}"),
    }
}
