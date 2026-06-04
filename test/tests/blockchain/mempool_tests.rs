use ethrex_blockchain::Blockchain;
use ethrex_blockchain::constants::MAX_INITCODE_SIZE;
use ethrex_blockchain::constants::{
    MAX_TRANSACTION_DATA_SIZE, TX_ACCESS_LIST_ADDRESS_GAS, TX_ACCESS_LIST_STORAGE_KEY_GAS,
    TX_CREATE_GAS_COST, TX_DATA_NON_ZERO_GAS, TX_DATA_NON_ZERO_GAS_EIP2028, TX_DATA_ZERO_GAS_COST,
    TX_GAS_COST, TX_INIT_CODE_WORD_GAS_COST,
};
use ethrex_blockchain::error::MempoolError;
use ethrex_blockchain::mempool::{Mempool, transaction_intrinsic_gas};
use std::collections::HashMap;

use ethrex_common::types::{
    BYTES_PER_BLOB, BlobsBundle, BlockHeader, ChainConfig, EIP1559Transaction, EIP4844Transaction,
    FRAME_SIG_SCHEME_P256, FRAME_SIG_SCHEME_SECP256K1, FRAME_TX_EXPIRY_DATA_LENGTH,
    FRAME_TX_MAX_VERIFY_GAS, Frame, FrameMode, FrameSignature, FrameTransaction, Genesis,
    MempoolTransaction, Transaction, TxKind, frame_tx_expiry_verifier,
};
use ethrex_common::{Address, Bytes, H256, U256};
use ethrex_storage::error::StoreError;
use ethrex_storage::{EngineType, Store};

const MEMPOOL_MAX_SIZE_TEST: usize = 10_000;

async fn setup_storage(config: ChainConfig, header: BlockHeader) -> Result<Store, StoreError> {
    let mut store = Store::new("test", EngineType::InMemory)?;
    let block_number = header.number;
    let block_hash = header.hash();
    store.add_block_header(block_hash, header).await?;
    store
        .forkchoice_update(vec![], block_number, block_hash, None, None)
        .await?;
    store.set_chain_config(&config).await?;
    Ok(store)
}

fn build_basic_config_and_header(
    istanbul_active: bool,
    shanghai_active: bool,
) -> (ChainConfig, BlockHeader) {
    let config = ChainConfig {
        shanghai_time: Some(if shanghai_active { 1 } else { 10 }),
        istanbul_block: Some(if istanbul_active { 1 } else { 10 }),
        ..Default::default()
    };

    let header = BlockHeader {
        number: 5,
        timestamp: 5,
        gas_limit: 100_000_000,
        gas_used: 0,
        ..Default::default()
    };

    (config, header)
}

#[test]
fn normal_transaction_intrinsic_gas() {
    let (config, header) = build_basic_config_and_header(false, false);

    let tx = EIP1559Transaction {
        nonce: 3,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit: 100_000,
        to: TxKind::Call(Address::from_low_u64_be(1)), // Normal tx
        value: U256::zero(),                           // Value zero
        data: Bytes::default(),                        // No data
        access_list: Default::default(),               // No access list
        ..Default::default()
    };

    let tx = Transaction::EIP1559Transaction(tx);
    let expected_gas_cost = TX_GAS_COST;
    let intrinsic_gas = transaction_intrinsic_gas(&tx, &header, &config).expect("Intrinsic gas");
    assert_eq!(intrinsic_gas, expected_gas_cost);
}

#[test]
fn create_transaction_intrinsic_gas() {
    let (config, header) = build_basic_config_and_header(false, false);

    let tx = EIP1559Transaction {
        nonce: 3,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit: 100_000,
        to: TxKind::Create,              // Create tx
        value: U256::zero(),             // Value zero
        data: Bytes::default(),          // No data
        access_list: Default::default(), // No access list
        ..Default::default()
    };

    let tx = Transaction::EIP1559Transaction(tx);
    let expected_gas_cost = TX_CREATE_GAS_COST;
    let intrinsic_gas = transaction_intrinsic_gas(&tx, &header, &config).expect("Intrinsic gas");
    assert_eq!(intrinsic_gas, expected_gas_cost);
}

#[test]
fn transaction_intrinsic_data_gas_pre_istanbul() {
    let (config, header) = build_basic_config_and_header(false, false);

    let tx = EIP1559Transaction {
        nonce: 3,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit: 100_000,
        to: TxKind::Call(Address::from_low_u64_be(1)), // Normal tx
        value: U256::zero(),                           // Value zero
        data: Bytes::from(vec![0x0, 0x1, 0x1, 0x0, 0x1, 0x1]), // 6 bytes of data
        access_list: Default::default(),               // No access list
        ..Default::default()
    };

    let tx = Transaction::EIP1559Transaction(tx);
    let expected_gas_cost = TX_GAS_COST + 2 * TX_DATA_ZERO_GAS_COST + 4 * TX_DATA_NON_ZERO_GAS;
    let intrinsic_gas = transaction_intrinsic_gas(&tx, &header, &config).expect("Intrinsic gas");
    assert_eq!(intrinsic_gas, expected_gas_cost);
}

#[test]
fn transaction_intrinsic_data_gas_post_istanbul() {
    let (config, header) = build_basic_config_and_header(true, false);

    let tx = EIP1559Transaction {
        nonce: 3,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit: 100_000,
        to: TxKind::Call(Address::from_low_u64_be(1)), // Normal tx
        value: U256::zero(),                           // Value zero
        data: Bytes::from(vec![0x0, 0x1, 0x1, 0x0, 0x1, 0x1]), // 6 bytes of data
        access_list: Default::default(),               // No access list
        ..Default::default()
    };

    let tx = Transaction::EIP1559Transaction(tx);
    let expected_gas_cost =
        TX_GAS_COST + 2 * TX_DATA_ZERO_GAS_COST + 4 * TX_DATA_NON_ZERO_GAS_EIP2028;
    let intrinsic_gas = transaction_intrinsic_gas(&tx, &header, &config).expect("Intrinsic gas");
    assert_eq!(intrinsic_gas, expected_gas_cost);
}

#[test]
fn transaction_create_intrinsic_gas_pre_shanghai() {
    let (config, header) = build_basic_config_and_header(false, false);

    let n_words: u64 = 10;
    let n_bytes: u64 = 32 * n_words - 3; // Test word rounding

    let tx = EIP1559Transaction {
        nonce: 3,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit: 100_000,
        to: TxKind::Create,                                // Create tx
        value: U256::zero(),                               // Value zero
        data: Bytes::from(vec![0x1_u8; n_bytes as usize]), // Bytecode data
        access_list: Default::default(),                   // No access list
        ..Default::default()
    };

    let tx = Transaction::EIP1559Transaction(tx);
    let expected_gas_cost = TX_CREATE_GAS_COST + n_bytes * TX_DATA_NON_ZERO_GAS;
    let intrinsic_gas = transaction_intrinsic_gas(&tx, &header, &config).expect("Intrinsic gas");
    assert_eq!(intrinsic_gas, expected_gas_cost);
}

#[test]
fn transaction_create_intrinsic_gas_post_shanghai() {
    let (config, header) = build_basic_config_and_header(false, true);

    let n_words: u64 = 10;
    let n_bytes: u64 = 32 * n_words - 3; // Test word rounding

    let tx = EIP1559Transaction {
        nonce: 3,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit: 100_000,
        to: TxKind::Create,                                // Create tx
        value: U256::zero(),                               // Value zero
        data: Bytes::from(vec![0x1_u8; n_bytes as usize]), // Bytecode data
        access_list: Default::default(),                   // No access list
        ..Default::default()
    };

    let tx = Transaction::EIP1559Transaction(tx);
    let expected_gas_cost =
        TX_CREATE_GAS_COST + n_bytes * TX_DATA_NON_ZERO_GAS + n_words * TX_INIT_CODE_WORD_GAS_COST;
    let intrinsic_gas = transaction_intrinsic_gas(&tx, &header, &config).expect("Intrinsic gas");
    assert_eq!(intrinsic_gas, expected_gas_cost);
}

#[test]
fn transaction_intrinsic_gas_access_list() {
    let (config, header) = build_basic_config_and_header(false, false);

    let access_list = vec![
        (Address::zero(), vec![H256::default(); 10]),
        (Address::zero(), vec![]),
        (Address::zero(), vec![H256::default(); 5]),
    ];

    let tx = EIP1559Transaction {
        nonce: 3,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit: 100_000,
        to: TxKind::Call(Address::from_low_u64_be(1)), // Normal tx
        value: U256::zero(),                           // Value zero
        data: Bytes::default(),                        // No data
        access_list,
        ..Default::default()
    };

    let tx = Transaction::EIP1559Transaction(tx);
    let expected_gas_cost =
        TX_GAS_COST + 3 * TX_ACCESS_LIST_ADDRESS_GAS + 15 * TX_ACCESS_LIST_STORAGE_KEY_GAS;
    let intrinsic_gas = transaction_intrinsic_gas(&tx, &header, &config).expect("Intrinsic gas");
    assert_eq!(intrinsic_gas, expected_gas_cost);
}

#[tokio::test]
async fn transaction_with_big_init_code_in_shanghai_fails() {
    let (config, header) = build_basic_config_and_header(false, true);

    let store = setup_storage(config, header).await.expect("Storage setup");
    let blockchain = Blockchain::default_with_store(store);

    let tx = EIP1559Transaction {
        nonce: 3,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit: 99_000_000,
        to: TxKind::Create,                                           // Create tx
        value: U256::zero(),                                          // Value zero
        data: Bytes::from(vec![0x1; MAX_INITCODE_SIZE as usize + 1]), // Large init code
        access_list: Default::default(),                              // No access list
        ..Default::default()
    };

    let tx = Transaction::EIP1559Transaction(tx);
    let validation = blockchain.validate_transaction(&tx, Address::random());
    assert!(matches!(
        validation.await,
        Err(MempoolError::TxMaxInitCodeSizeError)
    ));
}

#[tokio::test]
async fn transaction_with_gas_limit_higher_than_of_the_block_should_fail() {
    let (config, header) = build_basic_config_and_header(false, false);

    let store = setup_storage(config, header).await.expect("Storage setup");
    let blockchain = Blockchain::default_with_store(store);

    let tx = EIP1559Transaction {
        nonce: 3,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit: 100_000_001,
        to: TxKind::Call(Address::from_low_u64_be(1)), // Normal tx
        value: U256::zero(),                           // Value zero
        data: Bytes::default(),                        // No data
        access_list: Default::default(),               // No access list
        ..Default::default()
    };

    let tx = Transaction::EIP1559Transaction(tx);
    let validation = blockchain.validate_transaction(&tx, Address::random());
    assert!(matches!(
        validation.await,
        Err(MempoolError::TxGasLimitExceededError)
    ));
}

#[tokio::test]
async fn transaction_with_priority_fee_higher_than_gas_fee_should_fail() {
    let (config, header) = build_basic_config_and_header(false, false);

    let store = setup_storage(config, header).await.expect("Storage setup");
    let blockchain = Blockchain::default_with_store(store);

    let tx = EIP1559Transaction {
        nonce: 3,
        max_priority_fee_per_gas: 101,
        max_fee_per_gas: 100,
        gas_limit: 50_000_000,
        to: TxKind::Call(Address::from_low_u64_be(1)), // Normal tx
        value: U256::zero(),                           // Value zero
        data: Bytes::default(),                        // No data
        access_list: Default::default(),               // No access list
        ..Default::default()
    };

    let tx = Transaction::EIP1559Transaction(tx);
    let validation = blockchain.validate_transaction(&tx, Address::random());
    assert!(matches!(
        validation.await,
        Err(MempoolError::TxTipAboveFeeCapError)
    ));
}

#[tokio::test]
async fn transaction_with_gas_limit_lower_than_intrinsic_gas_should_fail() {
    let (config, header) = build_basic_config_and_header(false, false);
    let store = setup_storage(config, header).await.expect("Storage setup");
    let blockchain = Blockchain::default_with_store(store);
    let intrinsic_gas_cost = TX_GAS_COST;

    let tx = EIP1559Transaction {
        nonce: 3,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit: intrinsic_gas_cost - 1,
        to: TxKind::Call(Address::from_low_u64_be(1)), // Normal tx
        value: U256::zero(),                           // Value zero
        data: Bytes::default(),                        // No data
        access_list: Default::default(),               // No access list
        ..Default::default()
    };

    let tx = Transaction::EIP1559Transaction(tx);
    let validation = blockchain.validate_transaction(&tx, Address::random());
    assert!(matches!(
        validation.await,
        Err(MempoolError::TxIntrinsicGasCostAboveLimitError)
    ));
}

#[tokio::test]
async fn transaction_with_blob_base_fee_below_min_should_fail() {
    let (config, header) = build_basic_config_and_header(false, false);
    let store = setup_storage(config, header).await.expect("Storage setup");
    let blockchain = Blockchain::default_with_store(store);

    let tx = EIP4844Transaction {
        nonce: 3,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        max_fee_per_blob_gas: 0.into(),
        gas: 15_000_000,
        to: Address::from_low_u64_be(1), // Normal tx
        value: U256::zero(),             // Value zero
        data: Bytes::default(),          // No data
        access_list: Default::default(), // No access list
        ..Default::default()
    };

    let tx = Transaction::EIP4844Transaction(tx);
    let validation = blockchain.validate_transaction(&tx, Address::random());
    assert!(matches!(
        validation.await,
        Err(MempoolError::TxBlobBaseFeeTooLowError)
    ));
}

#[test]
fn test_filter_mempool_transactions() {
    let plain_tx_decoded = Transaction::decode_canonical(&hex::decode("f86d80843baa0c4082f618946177843db3138ae69679a54b95cf345ed759450d870aa87bee538000808360306ba0151ccc02146b9b11adf516e6787b59acae3e76544fdcd75e77e67c6b598ce65da064c5dd5aae2fbb535830ebbdad0234975cd7ece3562013b63ea18cc0df6c97d4").unwrap()).unwrap();
    let plain_tx_sender = plain_tx_decoded.sender().unwrap();
    let plain_tx = MempoolTransaction::new(plain_tx_decoded, plain_tx_sender);
    let blob_tx_decoded = Transaction::decode_canonical(&hex::decode("03f88f0780843b9aca008506fc23ac00830186a09400000000000000000000000000000000000001008080c001e1a0010657f37554c781402a22917dee2f75def7ab966d7b770905398eba3c44401401a0840650aa8f74d2b07f40067dc33b715078d73422f01da17abdbd11e02bbdfda9a04b2260f6022bf53eadb337b3e59514936f7317d872defb891a708ee279bdca90").unwrap()).unwrap();
    let blob_tx_sender = blob_tx_decoded.sender().unwrap();
    let blob_tx = MempoolTransaction::new(blob_tx_decoded, blob_tx_sender);
    let plain_tx_hash = plain_tx.hash();
    let blob_tx_hash = blob_tx.hash();
    let mempool = Mempool::new(MEMPOOL_MAX_SIZE_TEST);
    let filter = |tx: &Transaction| -> bool { matches!(tx, Transaction::EIP4844Transaction(_)) };
    mempool
        .add_transaction(blob_tx_hash, blob_tx_sender, blob_tx.clone())
        .unwrap();
    mempool
        .add_transaction(plain_tx_hash, plain_tx_sender, plain_tx)
        .unwrap();
    let txs = mempool.filter_transactions_with_filter_fn(&filter).unwrap();
    assert_eq!(txs, HashMap::from([(blob_tx.sender(), vec![blob_tx])]));
}

#[test]
fn blobs_bundle_loadtest() {
    // Write a bundle of 6 blobs 10 times
    // If this test fails please adjust the max_size in the DB config
    let mempool = Mempool::new(MEMPOOL_MAX_SIZE_TEST);
    for i in 0..300 {
        let blobs = [[i as u8; BYTES_PER_BLOB]; 6];
        let commitments = [[i as u8; 48]; 6];
        let proofs = [[i as u8; 48]; 6];
        let bundle = BlobsBundle {
            blobs: blobs.to_vec(),
            commitments: commitments.to_vec(),
            proofs: proofs.to_vec(),
            version: 0,
        };
        mempool.add_blobs_bundle(H256::random(), bundle).unwrap();
    }
}

// ---------------------------------------------------------------------------
// EIP-8141 frame transaction mempool admission tests
// ---------------------------------------------------------------------------

/// In-memory store whose genesis head has the Hegota fork active (so frame txs
/// pass the FrameTxPreFork gate) and a real state trie root (so account lookups
/// during admission succeed instead of erroring on a missing trie root).
async fn setup_hegota_store() -> Store {
    let genesis = Genesis {
        config: ChainConfig {
            chain_id: 0,
            shanghai_time: Some(0),
            hegota_time: Some(0),
            ..Default::default()
        },
        gas_limit: 100_000_000,
        ..Default::default()
    };
    let mut store = Store::new("hegota-test", EngineType::InMemory).expect("Storage setup");
    store
        .add_initial_state(genesis)
        .await
        .expect("add genesis state");
    store
}

/// A minimal, statically-valid 1-frame transaction with an empty signature list.
fn minimal_valid_frame_tx() -> FrameTransaction {
    FrameTransaction {
        chain_id: 0, // matches ChainConfig::default().chain_id
        nonce: 0,
        sender: Address::from_low_u64_be(0xABCD),
        frames: vec![Frame {
            mode: FrameMode::Default as u8,
            flags: 0x00,
            target: Some(Address::from_low_u64_be(0x1234)),
            // Small per-frame gas so total_gas_limit() stays below the legacy
            // 21000 intrinsic floor: this tx is only admitted once the frame-tx
            // intrinsic-gas fix prices it correctly.
            gas_limit: 100,
            value: U256::zero(),
            data: Bytes::from_static(b"call_data"),
        }],
        signatures: vec![],
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        max_fee_per_blob_gas: U256::zero(),
        blob_versioned_hashes: vec![],
        ..Default::default()
    }
}

#[tokio::test]
async fn mempool_rejects_frame_tx_with_invalid_signature() {
    let store = setup_hegota_store().await;
    let blockchain = Blockchain::default_with_store(store);

    let mut frame_tx = minimal_valid_frame_tx();
    // One secp256k1 signature with the right length (65 bytes) but garbage bytes:
    // ecrecover will not recover the claimed signer, so admission must reject it.
    frame_tx.signatures = vec![FrameSignature {
        scheme: FRAME_SIG_SCHEME_SECP256K1,
        signer: Address::from_low_u64_be(0xABCD),
        msg: Bytes::new(),
        signature: Bytes::from(vec![0xAB; 65]),
    }];

    let tx = Transaction::FrameTransaction(frame_tx);
    let validation = blockchain.validate_transaction(&tx, tx.sender().unwrap());
    assert!(matches!(
        validation.await,
        Err(MempoolError::InvalidFrameSignature)
    ));
}

#[tokio::test]
async fn mempool_rejects_frame_tx_violating_static_constraints() {
    let store = setup_hegota_store().await;
    let blockchain = Blockchain::default_with_store(store);

    let mut frame_tx = minimal_valid_frame_tx();
    // mode 5 is reserved (modes 3-255 are invalid) -> static-constraint failure.
    frame_tx.frames[0].mode = 5;

    let tx = Transaction::FrameTransaction(frame_tx);
    let validation = blockchain.validate_transaction(&tx, tx.sender().unwrap());
    assert!(matches!(
        validation.await,
        Err(MempoolError::InvalidFrameTransaction(_))
    ));
}

#[tokio::test]
async fn mempool_accepts_small_frame_tx() {
    let store = setup_hegota_store().await;
    let blockchain = Blockchain::default_with_store(store);

    // Empty signature list passes signature validation (nothing to reject), and
    // the tx otherwise satisfies static constraints + nonce/fee checks.
    let frame_tx = minimal_valid_frame_tx();
    let tx = Transaction::FrameTransaction(frame_tx);
    let validation = blockchain.validate_transaction(&tx, tx.sender().unwrap());
    assert!(
        validation.await.is_ok(),
        "minimal valid frame tx should be admitted"
    );
}

#[tokio::test]
async fn mempool_rejects_oversized_frame_data() {
    let store = setup_hegota_store().await;
    let blockchain = Blockchain::default_with_store(store);

    let mut frame_tx = minimal_valid_frame_tx();
    // Frame data whose length reaches the 128KB cap; tx.data() is empty for frame
    // txs, so only the per-frame data check can catch this.
    frame_tx.frames[0].data = Bytes::from(vec![0u8; MAX_TRANSACTION_DATA_SIZE as usize]);

    let tx = Transaction::FrameTransaction(frame_tx);
    let validation = blockchain.validate_transaction(&tx, tx.sender().unwrap());
    assert!(matches!(
        validation.await,
        Err(MempoolError::TxMaxDataSizeError)
    ));
}

#[tokio::test]
async fn mempool_rejects_frame_tx_with_blobs() {
    let store = setup_hegota_store().await;
    let blockchain = Blockchain::default_with_store(store);

    let mut frame_tx = minimal_valid_frame_tx();
    // Add a blob versioned hash; no sidecar transport exists for frame-tx
    // blobs yet, so admission must reject such txs as unsupported.
    frame_tx.blob_versioned_hashes = vec![H256::random()];

    let tx = Transaction::FrameTransaction(frame_tx);
    let validation = blockchain.validate_transaction(&tx, tx.sender().unwrap());
    assert!(matches!(
        validation.await,
        Err(MempoolError::FrameTxBlobsUnsupported)
    ));
}

#[tokio::test]
async fn mempool_rejects_frame_tx_exceeding_max_verify_gas() {
    let store = setup_hegota_store().await;
    let blockchain = Blockchain::default_with_store(store);

    // EIP-8141 §Mempool rule #6: signature validation counts against
    // MAX_VERIFY_GAS (100_000). P256 sigs cost 6700 each, so 15 sigs cost
    // 100_500 > MAX_VERIFY_GAS and the tx must be rejected at admission BEFORE
    // any per-signature crypto runs. The signature bytes need not be valid:
    // the cap rejects first. Static constraints only require a known scheme and
    // an empty-or-32-byte msg, which these satisfy.
    let n_sigs = (FRAME_TX_MAX_VERIFY_GAS / 6700) as usize + 1; // 15
    let mut frame_tx = minimal_valid_frame_tx();
    frame_tx.signatures = (0..n_sigs)
        .map(|_| FrameSignature {
            scheme: FRAME_SIG_SCHEME_P256,
            signer: Address::from_low_u64_be(0xABCD),
            msg: Bytes::new(),
            signature: Bytes::from(vec![0u8; 128]),
        })
        .collect();
    assert!(frame_tx.signature_verification_cost() > FRAME_TX_MAX_VERIFY_GAS);

    let tx = Transaction::FrameTransaction(frame_tx);
    let validation = blockchain.validate_transaction(&tx, tx.sender().unwrap());
    assert!(matches!(
        validation.await,
        Err(MempoolError::FrameTxVerifyGasExceeded)
    ));
}

#[tokio::test]
async fn mempool_rejects_frame_tx_from_unknown_sender_with_sentinel_nonce() {
    let store = setup_hegota_store().await;
    let blockchain = Blockchain::default_with_store(store);

    // Sender 0xABCD does not exist in the genesis state. A frame tx from a
    // not-yet-existent sender is legitimate (sponsored txs fund gas via a
    // separate payer), but its implied nonce is 0, so the u64::MAX sentinel can
    // never match and must be rejected — not skipped as it was before.
    let mut frame_tx = minimal_valid_frame_tx();
    frame_tx.nonce = u64::MAX;

    let tx = Transaction::FrameTransaction(frame_tx);
    let validation = blockchain.validate_transaction(&tx, tx.sender().unwrap());
    assert!(matches!(validation.await, Err(MempoolError::NonceTooLow)));
}

#[tokio::test]
async fn mempool_accepts_frame_tx_from_unknown_sender_with_zero_nonce() {
    let store = setup_hegota_store().await;
    let blockchain = Blockchain::default_with_store(store);

    // Regression guard for the sponsored-tx use case: a fresh (non-existent)
    // sender with nonce 0 must still be admitted after the nonce hardening —
    // the new guard only rejects sub-current / sentinel nonces.
    let frame_tx = minimal_valid_frame_tx(); // sender 0xABCD (absent), nonce 0
    let tx = Transaction::FrameTransaction(frame_tx);
    let validation = blockchain.validate_transaction(&tx, tx.sender().unwrap());
    assert!(
        validation.await.is_ok(),
        "fresh sponsored sender with nonce 0 should still be admitted"
    );
}

// ---------------------------------------------------------------------------
// EIP-8141 fork gate and expiry gate tests
// ---------------------------------------------------------------------------

/// Store where Hegota is NOT active: hegota_time is None, so frame txs must be
/// rejected with FrameTxPreFork regardless of their content.
async fn setup_pre_hegota_store() -> Store {
    let genesis = Genesis {
        config: ChainConfig {
            chain_id: 0,
            shanghai_time: Some(0),
            hegota_time: None, // Hegota never activates
            ..Default::default()
        },
        gas_limit: 100_000_000,
        ..Default::default()
    };
    let mut store = Store::new("pre-hegota-test", EngineType::InMemory).expect("Storage setup");
    store
        .add_initial_state(genesis)
        .await
        .expect("add genesis state");
    store
}

/// Store where Hegota IS active (hegota_time == 0) and the head block has a
/// non-zero timestamp (1000), so expiry tests can use deadlines below that.
async fn setup_hegota_store_ts1000() -> Store {
    let genesis = Genesis {
        config: ChainConfig {
            chain_id: 0,
            shanghai_time: Some(0),
            hegota_time: Some(0),
            ..Default::default()
        },
        gas_limit: 100_000_000,
        timestamp: 1000, // head.timestamp == 1000
        ..Default::default()
    };
    let mut store = Store::new("hegota-ts1000-test", EngineType::InMemory).expect("Storage setup");
    store
        .add_initial_state(genesis)
        .await
        .expect("add genesis state");
    store
}

/// Build a minimal frame tx carrying a single expiry verifier frame with the
/// given `deadline`. The frame is a VERIFY frame targeting the expiry verifier
/// address with exactly 8 bytes of big-endian deadline data and flags == 0.
fn frame_tx_with_expiry(deadline: u64) -> FrameTransaction {
    let mut data = [0u8; FRAME_TX_EXPIRY_DATA_LENGTH];
    data.copy_from_slice(&deadline.to_be_bytes());
    FrameTransaction {
        chain_id: 0,
        nonce: 0,
        sender: Address::from_low_u64_be(0xABCD),
        frames: vec![Frame {
            mode: FrameMode::Verify as u8,
            flags: 0x00,
            target: Some(frame_tx_expiry_verifier()),
            gas_limit: 100,
            value: U256::zero(),
            data: Bytes::from(data.to_vec()),
        }],
        signatures: vec![],
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        max_fee_per_blob_gas: U256::zero(),
        blob_versioned_hashes: vec![],
        ..Default::default()
    }
}

#[tokio::test]
async fn mempool_rejects_frame_tx_before_hegota() {
    // With hegota_time == None the fork gate must fire before any other check.
    let store = setup_pre_hegota_store().await;
    let blockchain = Blockchain::default_with_store(store);

    let frame_tx = minimal_valid_frame_tx();
    let tx = Transaction::FrameTransaction(frame_tx);
    let result = blockchain
        .validate_transaction(&tx, tx.sender().unwrap())
        .await;
    assert!(
        matches!(result, Err(MempoolError::FrameTxPreFork)),
        "expected FrameTxPreFork, got {result:?}"
    );
}

#[tokio::test]
async fn mempool_rejects_expired_frame_tx() {
    // Head timestamp == 1000. A deadline of 999 is strictly less than 1000,
    // so the expiry gate must fire.
    let store = setup_hegota_store_ts1000().await;
    let blockchain = Blockchain::default_with_store(store);

    let frame_tx = frame_tx_with_expiry(999);
    let tx = Transaction::FrameTransaction(frame_tx);
    let result = blockchain
        .validate_transaction(&tx, tx.sender().unwrap())
        .await;
    assert!(
        matches!(result, Err(MempoolError::FrameTxExpired)),
        "expected FrameTxExpired for deadline 999 < head.timestamp 1000, got {result:?}"
    );
}

#[tokio::test]
async fn mempool_accepts_frame_tx_with_deadline_at_head_timestamp() {
    // Head timestamp == 1000. A deadline of exactly 1000 is the boundary:
    // the on-chain verifier only reverts when block.timestamp > deadline, so
    // deadline == timestamp is still valid at mempool admission time.
    let store = setup_hegota_store_ts1000().await;
    let blockchain = Blockchain::default_with_store(store);

    let frame_tx = frame_tx_with_expiry(1000);
    let tx = Transaction::FrameTransaction(frame_tx);
    let result = blockchain
        .validate_transaction(&tx, tx.sender().unwrap())
        .await;
    assert!(
        result.is_ok(),
        "frame tx with deadline == head.timestamp must be admitted; got {result:?}"
    );
}
