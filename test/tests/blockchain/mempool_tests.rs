use ethrex_blockchain::Blockchain;
use ethrex_blockchain::constants::MAX_INITCODE_SIZE;
use ethrex_blockchain::constants::{
    TX_ACCESS_LIST_ADDRESS_GAS, TX_ACCESS_LIST_STORAGE_KEY_GAS, TX_CREATE_GAS_COST,
    TX_DATA_NON_ZERO_GAS, TX_DATA_NON_ZERO_GAS_EIP2028, TX_DATA_ZERO_GAS_COST, TX_GAS_COST,
    TX_INIT_CODE_WORD_GAS_COST,
};
use ethrex_blockchain::error::MempoolError;
use ethrex_blockchain::mempool::{Mempool, transaction_intrinsic_gas};
use ethrex_blockchain::{BlockchainOptions, BlockchainType};
use ethrex_crypto::NativeCrypto;
use rustc_hash::FxHashMap;

use ethrex_common::types::{
    BYTES_PER_BLOB, BlobsBundle, BlockHeader, ChainConfig, EIP1559Transaction, EIP4844Transaction,
    EIP7702_DELEGATION_PREFIX, Genesis, GenesisAccount, MempoolTransaction, Transaction, TxKind,
    kzg_commitment_to_versioned_hash,
};
use ethrex_common::{Address, Bytes, H160, H256, U256};
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
    let plain_tx_sender = plain_tx_decoded.sender(&NativeCrypto).unwrap();
    let plain_tx = MempoolTransaction::new(plain_tx_decoded, plain_tx_sender);
    let blob_tx_decoded = Transaction::decode_canonical(&hex::decode("03f88f0780843b9aca008506fc23ac00830186a09400000000000000000000000000000000000001008080c001e1a0010657f37554c781402a22917dee2f75def7ab966d7b770905398eba3c44401401a0840650aa8f74d2b07f40067dc33b715078d73422f01da17abdbd11e02bbdfda9a04b2260f6022bf53eadb337b3e59514936f7317d872defb891a708ee279bdca90").unwrap()).unwrap();
    let blob_tx_sender = blob_tx_decoded.sender(&NativeCrypto).unwrap();
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
    assert_eq!(
        txs,
        FxHashMap::from_iter([(blob_tx.sender(), vec![blob_tx])])
    );
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

#[test]
fn blobs_bundle_insert_and_remove() {
    // Insert two bundles with 2 blobs, and where both bundles contain one specific blob.
    // Then remove one bundle making sure that blob-version-hash to tx-hash cache still points to
    // the other txn. And finally remove second bundle as well.
    let mempool = Mempool::new(MEMPOOL_MAX_SIZE_TEST);
    let (blob, commitment, proof) = ([255u8; BYTES_PER_BLOB], [255u8; 48], [255u8; 48]);
    let versioned_hash = kzg_commitment_to_versioned_hash(&commitment);
    let mut txn_hash = vec![];

    for i in 1..=2 {
        let blobs = [blob, [i as u8; BYTES_PER_BLOB]];
        let commitments = [commitment, [i as u8; 48]];
        let proofs = [proof, [i as u8; 48]];
        let bundle = BlobsBundle {
            blobs: blobs.to_vec(),
            commitments: commitments.to_vec(),
            proofs: proofs.to_vec(),
            version: 0,
        };
        let tx = EIP4844Transaction {
            nonce: 3,
            max_priority_fee_per_gas: 0,
            max_fee_per_gas: 0,
            max_fee_per_blob_gas: 0.into(),
            gas: 15_000_000,
            to: Address::from_low_u64_be(1), // Normal tx
            ..Default::default()
        };

        let tx = Transaction::EIP4844Transaction(tx);
        let sender = H160::random();
        let hash = H256::random();
        txn_hash.push(hash);
        mempool
            .add_blobs_bundle(txn_hash[i as usize - 1], bundle)
            .unwrap();

        mempool
            .add_transaction(hash, sender, MempoolTransaction::new(tx, sender))
            .expect("Failed to add blob transaction");
    }

    // When a txn is removed it should not remove the associated bundle as another txn also has the same bundle.
    for txn_hash in txn_hash.into_iter() {
        assert_eq!(
            mempool
                .get_blobs_data_by_versioned_hashes(&[versioned_hash])
                .expect("should return a bundle")
                .len(),
            1
        );

        mempool
            .remove_transaction(&txn_hash)
            .expect("should remove blob bundle by txn_hash");
    }

    // Once both transactions are removed it should remove the bundle as well.
    assert_eq!(
        mempool
            .get_blobs_data_by_versioned_hashes(&[versioned_hash])
            .expect("should return empty"),
        vec![None]
    );
}

// ===========================================================================
// EIP-7702 delegated-sender cap
//
// Senders whose code is an EIP-7702 delegation designation (`0xef0100 ||
// address`) can be invoked by their delegate to act on behalf of multiple
// identities. To bound the spam surface of a single signer, ethrex caps the
// number of pending transactions from a delegated EOA at
// `BlockchainOptions::delegated_sender_cap` (default 1). RBF replacements
// bypass the cap because they swap a pool slot rather than consume a new one.
// ===========================================================================

const TEST_CHAIN_ID: u64 = 1337;

/// Bytecode for an EIP-7702 delegation designation pointing at a fixed
/// (arbitrary) 20-byte address. The exact delegate is irrelevant for
/// mempool admission — only the prefix and length matter.
fn delegation_code() -> Bytes {
    let mut code = EIP7702_DELEGATION_PREFIX.to_vec();
    code.extend_from_slice(&[0x42u8; 20]);
    Bytes::from(code)
}

/// Build a post-Cancun genesis with `sender` pre-funded and (optionally)
/// pre-installed with EIP-7702 delegation code. `chain_id` is wired to
/// match transactions signed against `TEST_CHAIN_ID`.
fn delegated_sender_genesis(sender: Address, install_delegation: bool) -> Genesis {
    let mut alloc = std::collections::BTreeMap::new();
    alloc.insert(
        sender,
        GenesisAccount {
            balance: U256::from(10u64).pow(U256::from(20u64)),
            code: if install_delegation {
                delegation_code()
            } else {
                Bytes::new()
            },
            storage: Default::default(),
            nonce: 0,
        },
    );

    Genesis {
        config: ChainConfig {
            chain_id: TEST_CHAIN_ID,
            // Activate enough forks for EIP-1559 (London) gas semantics.
            homestead_block: Some(0),
            eip150_block: Some(0),
            eip155_block: Some(0),
            eip158_block: Some(0),
            byzantium_block: Some(0),
            constantinople_block: Some(0),
            petersburg_block: Some(0),
            istanbul_block: Some(0),
            berlin_block: Some(0),
            london_block: Some(0),
            merge_netsplit_block: Some(0),
            shanghai_time: Some(0),
            cancun_time: Some(0),
            terminal_total_difficulty: Some(0),
            ..Default::default()
        },
        alloc,
        gas_limit: 30_000_000,
        timestamp: 0,
        ..Default::default()
    }
}

async fn setup_blockchain_with_sender(
    sender: Address,
    install_delegation: bool,
    delegated_sender_cap: u64,
) -> Blockchain {
    let mut store =
        Store::new("test_delegated", EngineType::InMemory).expect("storage for delegated cap test");
    let genesis = delegated_sender_genesis(sender, install_delegation);
    store
        .add_initial_state(genesis)
        .await
        .expect("add genesis state");

    Blockchain::new(
        store,
        BlockchainOptions {
            r#type: BlockchainType::L1,
            delegated_sender_cap,
            ..Default::default()
        },
    )
}

fn delegated_sender_tx(sender_nonce: u64) -> Transaction {
    Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: TEST_CHAIN_ID,
        nonce: sender_nonce,
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: 1_000_000_000,
        gas_limit: 21_000,
        to: TxKind::Call(Address::from_low_u64_be(1)),
        value: U256::zero(),
        data: Bytes::default(),
        access_list: Default::default(),
        ..Default::default()
    })
}

/// Insert an unsigned transaction into the mempool under a fabricated sender,
/// bypassing the sender-recovery path. The tests below only care about the
/// admission decision (`validate_transaction`), which takes `sender` as an
/// explicit argument — they don't exercise signature recovery.
fn seed_pending_tx(blockchain: &Blockchain, sender: Address, tx: Transaction) {
    let hash = tx.hash();
    blockchain
        .mempool
        .add_transaction(hash, sender, MempoolTransaction::new(tx, sender))
        .expect("seed mempool with unsigned tx");
}

#[tokio::test]
async fn delegated_sender_second_tx_rejected_by_default_cap() {
    let sender = Address::from_low_u64_be(0xD1);
    let blockchain = setup_blockchain_with_sender(sender, true, 1).await;

    seed_pending_tx(&blockchain, sender, delegated_sender_tx(0));

    let tx1 = delegated_sender_tx(1);
    let err = blockchain
        .validate_transaction(&tx1, sender)
        .await
        .expect_err("second delegated tx should be rejected");
    assert!(
        matches!(err, MempoolError::MaxDelegatedPendingTxsExceeded(1)),
        "unexpected error: {err:?}"
    );
}

#[tokio::test]
async fn delegated_sender_replacement_bypasses_cap() {
    let sender = Address::from_low_u64_be(0xD2);
    let blockchain = setup_blockchain_with_sender(sender, true, 1).await;

    seed_pending_tx(&blockchain, sender, delegated_sender_tx(0));

    // RBF: same nonce, strictly higher fees → must be admitted even though the
    // delegated sender already has 1 pending tx (the cap).
    let replacement = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: TEST_CHAIN_ID,
        nonce: 0,
        max_priority_fee_per_gas: 2,
        max_fee_per_gas: 2_000_000_000,
        gas_limit: 21_000,
        to: TxKind::Call(Address::from_low_u64_be(1)),
        value: U256::zero(),
        data: Bytes::default(),
        access_list: Default::default(),
        ..Default::default()
    });
    let tx_to_replace = blockchain
        .validate_transaction(&replacement, sender)
        .await
        .expect("RBF replacement bypasses the delegated cap");
    assert!(
        tx_to_replace.is_some(),
        "validate_transaction should report the slot being replaced"
    );
}

#[tokio::test]
async fn non_delegated_sender_second_tx_admitted() {
    let sender = Address::from_low_u64_be(0xD3);
    // Cap delegated senders at 1, but `sender` here is NOT delegated, so the
    // cap must not apply.
    let blockchain = setup_blockchain_with_sender(sender, false, 1).await;

    seed_pending_tx(&blockchain, sender, delegated_sender_tx(0));

    let tx1 = delegated_sender_tx(1);
    blockchain
        .validate_transaction(&tx1, sender)
        .await
        .expect("non-delegated sender is not subject to the delegated cap");
}

#[tokio::test]
async fn delegated_sender_cap_override_admits_more_txs() {
    let sender = Address::from_low_u64_be(0xD4);
    // Custom override: allow 2 pending txs from delegated senders.
    let blockchain = setup_blockchain_with_sender(sender, true, 2).await;

    seed_pending_tx(&blockchain, sender, delegated_sender_tx(0));
    seed_pending_tx(&blockchain, sender, delegated_sender_tx(1));

    let tx2 = delegated_sender_tx(2);
    let err = blockchain
        .validate_transaction(&tx2, sender)
        .await
        .expect_err("third delegated tx should hit the override cap");
    assert!(
        matches!(err, MempoolError::MaxDelegatedPendingTxsExceeded(2)),
        "unexpected error: {err:?}"
    );
}
