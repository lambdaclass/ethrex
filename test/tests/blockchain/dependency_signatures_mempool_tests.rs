//! Dependency signatures in the mempool: a frame transaction declaring leanSPHINCS dependencies
//! is admitted only with their signatures beside it, which the pool verifies,
//! keeps and serves with it.

use std::collections::BTreeMap;

use ethrex_blockchain::Blockchain;
use ethrex_blockchain::error::MempoolError;
use ethrex_common::types::{
    APPROVE_EXECUTION_AND_PAYMENT, BlobsBundle, ChainConfig, DEPENDENCY_SCHEME_LEANSPHINCS,
    DependencyTriple, Frame, FrameMode, FrameTransaction, Genesis, GenesisAccount, P2PTransaction,
    Transaction, WrappedFrameTransaction,
};
use ethrex_common::utils::keccak;
use ethrex_common::{Address, Bytes, H256, U256};
use ethrex_crypto::NativeCrypto;
use ethrex_dep_aggregation::test_support::leansphincs_sign;
use ethrex_dep_aggregation::verify_leansphincs;
use ethrex_p2p::rlpx::eth::transactions::NewPooledTransactionHashes;
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::{EngineType, Store};

const SENDER: u64 = 0xABCD;
const SEED: [u8; 32] = [0x42; 32];

/// A store whose head runs EIP-8288, so dependency frames are valid, with the
/// sender seeded with APPROVE(3) code so its VERIFY frame establishes a payer.
async fn store() -> Store {
    let genesis = Genesis {
        config: ChainConfig {
            chain_id: 0,
            shanghai_time: Some(0),
            hegota_time: Some(0),
            eip8288_prototype_time: Some(0),
            ..Default::default()
        },
        gas_limit: 100_000_000,
        alloc: [(
            Address::from_low_u64_be(SENDER),
            GenesisAccount {
                code: Bytes::from(vec![
                    0x60,
                    APPROVE_EXECUTION_AND_PAYMENT,
                    0x60,
                    0x00,
                    0x60,
                    0x00,
                    0xAA,
                    0x00,
                ]),
                storage: BTreeMap::new(),
                balance: U256::zero(),
                nonce: 0,
            },
        )]
        .into_iter()
        .collect(),
        ..Default::default()
    };
    let mut store = Store::new("dependency-signatures-mempool", EngineType::InMemory).unwrap();
    store.add_initial_state(genesis).await.unwrap();
    store
}

/// A self-verifying frame transaction that declares `data_hashes` under `pk`,
/// one dependency each, in a single dependency verification frame.
fn dependency_tx(pk: [u8; 32], data_hashes: &[H256]) -> FrameTransaction {
    let sender = Address::from_low_u64_be(SENDER);
    let data: Vec<u8> = data_hashes
        .iter()
        .flat_map(|data_hash| {
            DependencyTriple {
                scheme: DEPENDENCY_SCHEME_LEANSPHINCS,
                data_hash: *data_hash,
                verification_key_hash: H256(pk),
            }
            .encode()
        })
        .collect();
    let mut tx = FrameTransaction {
        has_nonce_keys: true,
        sender,
        frames: vec![
            Frame {
                mode: FrameMode::Verify as u8,
                flags: APPROVE_EXECUTION_AND_PAYMENT,
                target: Some(sender),
                gas_limit: 1_000,
                ..Default::default()
            },
            Frame {
                mode: FrameMode::DepVerify as u8,
                gas_limit: 3_000 * data_hashes.len() as u64,
                data: Bytes::from(data),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let floor = tx.calldata_floor_gas(ethrex_common::types::Fork::Hegota);
    tx.frames[0].gas_limit = tx.frames[0].gas_limit.max(floor);
    tx
}

/// A transaction that depends on a signature of its own signature hash (a zero
/// `data_hash`), and that signature.
fn signed_tx() -> (FrameTransaction, Vec<Bytes>) {
    let (pk, _) = leansphincs_sign(SEED, [0; 32]);
    let tx = dependency_tx(pk, &[H256::zero()]);
    let (_, signature) = leansphincs_sign(SEED, tx.compute_sig_hash().0);
    (tx, vec![Bytes::from(signature)])
}

#[test]
fn geths_leansphincs_signature_is_reproduced_byte_for_byte() {
    // From go-ethereum's crypto/leansphincs: KeyGen and Sign of seed 0x42..42 and
    // message 0x07..07, and the Keccak-256 of that signature.
    let (pk, signature) = leansphincs_sign(SEED, [0x07; 32]);
    assert_eq!(
        hex::encode(pk),
        "fa3cbec584c9d1ff2a079506d275e7b0f39da2ee62284a9648688bb1f606bc1e"
    );
    assert_eq!(signature.len(), 7_740);
    assert_eq!(
        hex::encode(keccak(&signature)),
        "4a8ca8a280d265aa75f02cf93877fe2f52a5d2d2f439d367719d9cc61491e645"
    );
    assert_eq!(verify_leansphincs(&pk, &[0x07; 32], &signature), Ok(true));

    let mut tampered = signature.clone();
    tampered[100] ^= 1;
    assert_eq!(verify_leansphincs(&pk, &[0x07; 32], &tampered), Ok(false));
    assert_eq!(
        verify_leansphincs(&pk, &[0x07; 32], &signature[1..]),
        Ok(false)
    );
}

#[test]
fn the_wrapper_carries_signatures_and_leaves_the_blob_wrapper_unchanged() {
    let (tx, signatures) = signed_tx();
    let wrapped = P2PTransaction::with_dependency_signatures(tx.clone(), signatures.clone());
    let decoded = P2PTransaction::decode(&wrapped.encode_to_vec()).unwrap();
    assert_eq!(decoded, wrapped);
    let (decoded_tx, blobs_bundle, decoded_signatures) =
        decoded.into_transaction_and_sidecars().unwrap();
    assert_eq!(decoded_tx, Transaction::FrameTransaction(tx.clone()));
    assert!(blobs_bundle.is_none());
    assert_eq!(decoded_signatures, signatures);

    // Without signatures the wrapper encodes exactly as the blob wrapper did:
    // [tx, blobs, commitments, proofs].
    let blob_wrapper = WrappedFrameTransaction {
        tx: tx.clone(),
        wrapper_version: None,
        blobs_bundle: BlobsBundle::empty(),
        signatures: vec![],
    };
    let mut expected = Vec::new();
    ethrex_rlp::structs::Encoder::new(&mut expected)
        .encode_field(&tx)
        .encode_field(&blob_wrapper.blobs_bundle.blobs)
        .encode_field(&blob_wrapper.blobs_bundle.commitments)
        .encode_field(&blob_wrapper.blobs_bundle.proofs)
        .finish();
    assert_eq!(blob_wrapper.encode_to_vec(), expected);
    assert_eq!(
        WrappedFrameTransaction::decode(&expected).unwrap(),
        blob_wrapper
    );
}

#[tokio::test]
async fn a_dependency_tx_is_pooled_with_its_signatures_and_served_with_them() {
    let blockchain = Blockchain::default_with_store(store().await);
    let (tx, signatures) = signed_tx();
    let transaction = Transaction::FrameTransaction(tx.clone());

    let hash = blockchain
        .add_local_dependency_transaction_to_pool(transaction.clone(), signatures.clone())
        .await
        .unwrap();
    assert_eq!(
        blockchain.mempool.get_dependency_signatures(hash).unwrap(),
        Some(signatures.clone())
    );

    let served = blockchain.get_p2p_transaction_by_hash(&hash).unwrap();
    // Compared on the wire: the pooled copy has its hash caches filled.
    assert_eq!(
        served.encode_to_vec(),
        P2PTransaction::with_dependency_signatures(tx, signatures).encode_to_vec()
    );
    let announced = NewPooledTransactionHashes::new(vec![transaction], &blockchain).unwrap();
    assert_eq!(
        announced.transaction_sizes,
        vec![served.encode_canonical_len()]
    );

    blockchain.remove_transaction_from_pool(&hash).unwrap();
    assert_eq!(
        blockchain.mempool.get_dependency_signatures(hash).unwrap(),
        None
    );
}

#[tokio::test]
async fn a_dependency_tx_without_valid_signatures_is_refused() {
    let blockchain = Blockchain::default_with_store(store().await);
    let (tx, signatures) = signed_tx();
    let transaction = Transaction::FrameTransaction(tx.clone());

    let bare = blockchain
        .add_local_transaction_to_pool(transaction.clone())
        .await;
    assert!(matches!(
        bare,
        Err(MempoolError::FrameTxDependencyNeedsWrapper)
    ));

    let two = vec![signatures[0].clone(), signatures[0].clone()];
    let count = blockchain
        .add_local_dependency_transaction_to_pool(transaction.clone(), two)
        .await;
    assert!(matches!(
        count,
        Err(MempoolError::FrameTxDependencySignatureCount {
            expected: 1,
            actual: 2
        })
    ));

    let mut tampered = signatures[0].to_vec();
    tampered[0] ^= 1;
    let invalid = blockchain
        .add_local_dependency_transaction_to_pool(transaction, vec![Bytes::from(tampered)])
        .await;
    assert!(matches!(
        invalid,
        Err(MempoolError::FrameTxDependencySignatureInvalid { index: 0 })
    ));

    // Rejected before any signature is checked.
    let (pk, _) = leansphincs_sign(SEED, [0; 32]);
    let many: Vec<H256> = (1..=17u64).map(H256::from_low_u64_be).collect();
    let too_many = Transaction::FrameTransaction(dependency_tx(pk, &many));
    let refused = blockchain
        .add_local_dependency_transaction_to_pool(too_many, vec![Bytes::new(); 17])
        .await;
    assert!(matches!(
        refused,
        Err(MempoolError::FrameTxTooManyDependencies {
            count: 17,
            limit: 16
        })
    ));

    let hash = transaction_hash(&tx);
    assert_eq!(
        blockchain.mempool.get_dependency_signatures(hash).unwrap(),
        None
    );
}

fn transaction_hash(tx: &FrameTransaction) -> H256 {
    Transaction::FrameTransaction(tx.clone()).hash(&NativeCrypto)
}
