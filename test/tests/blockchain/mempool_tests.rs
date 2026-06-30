use std::collections::BTreeMap;
use std::{fs::File, io::BufReader, path::PathBuf};

use ethrex_blockchain::constants::MAX_INITCODE_SIZE;
use ethrex_blockchain::constants::{
    TX_ACCESS_LIST_ADDRESS_GAS, TX_ACCESS_LIST_STORAGE_KEY_GAS, TX_CREATE_GAS_COST,
    TX_DATA_NON_ZERO_GAS_EIP2028, TX_DATA_ZERO_GAS_COST, TX_GAS_COST, TX_INIT_CODE_WORD_GAS_COST,
};
use ethrex_blockchain::error::MempoolError;
use ethrex_blockchain::mempool::{Mempool, transaction_intrinsic_gas};
use ethrex_blockchain::{Blockchain, BlockchainOptions};
use ethrex_crypto::NativeCrypto;
use rustc_hash::FxHashMap;

use ethrex_common::types::{
    AuthorizationTuple, BYTES_PER_BLOB, BlobsBundle, BlockHeader, ChainConfig, EIP1559Transaction,
    EIP4844Transaction, EIP7702Transaction, Genesis, GenesisAccount, MempoolTransaction,
    Transaction, TxKind, kzg_commitment_to_versioned_hash,
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
    let intrinsic_gas = transaction_intrinsic_gas(&tx, Address::default(), &header, &config)
        .expect("Intrinsic gas");
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
    let intrinsic_gas = transaction_intrinsic_gas(&tx, Address::default(), &header, &config)
        .expect("Intrinsic gas");
    assert_eq!(intrinsic_gas, expected_gas_cost);
}

/// EIP-2780 (PRELIMINARY EIPs#11645): Amsterdam CREATE tx intrinsic must match
/// the VM charge, not the legacy `TX_CREATE_GAS_COST = 53000`. The regular
/// portion is the resource-based decomposition
/// `TX_BASE_COST_AMSTERDAM (12000) + CREATE_ACCESS_AMSTERDAM (11000) = 23000`
/// (no value transfer here), plus a state portion
/// (`STATE_BYTES_PER_NEW_ACCOUNT * cpsb`). Mempool admission must return the
/// total so txs whose `gas_limit` is below the VM intrinsic are rejected before
/// they enter the pool, and txs above it aren't spuriously rejected.
#[test]
fn amsterdam_create_intrinsic_matches_vm_dimensions() {
    use ethrex_levm::gas_cost::{
        CREATE_ACCESS_AMSTERDAM, STATE_BYTES_PER_NEW_ACCOUNT, cost_per_state_byte,
    };
    const TX_BASE_COST_AMSTERDAM: u64 = 12000;

    let (mut config, header) = build_basic_config_and_header(true, true);
    // Activate Amsterdam at genesis. Intermediate forks must also be active
    // so `config.fork(timestamp)` returns Amsterdam, not an earlier variant.
    config.cancun_time = Some(0);
    config.prague_time = Some(0);
    config.osaka_time = Some(0);
    config.bpo1_time = Some(0);
    config.bpo2_time = Some(0);
    config.amsterdam_time = Some(0);

    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        nonce: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit: 1_000_000,
        to: TxKind::Create,
        value: U256::zero(),
        data: Bytes::default(),
        access_list: Default::default(),
        ..Default::default()
    });

    let cpsb = cost_per_state_byte(header.gas_limit);
    let expected =
        TX_BASE_COST_AMSTERDAM + CREATE_ACCESS_AMSTERDAM + STATE_BYTES_PER_NEW_ACCOUNT * cpsb;

    let intrinsic_gas = transaction_intrinsic_gas(&tx, Address::default(), &header, &config)
        .expect("intrinsic gas");
    assert_eq!(
        intrinsic_gas, expected,
        "Amsterdam CREATE intrinsic must be TX_BASE_COST_AMSTERDAM + \
         CREATE_ACCESS_AMSTERDAM + STATE_BYTES_PER_NEW_ACCOUNT * cpsb, not the legacy 53000"
    );
    // Guard against regression to the legacy 53000 constant.
    assert_ne!(
        intrinsic_gas, TX_CREATE_GAS_COST,
        "Amsterdam CREATE must NOT use legacy TX_CREATE_GAS_COST"
    );
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
    let intrinsic_gas = transaction_intrinsic_gas(&tx, Address::default(), &header, &config)
        .expect("Intrinsic gas");
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
    let expected_gas_cost = TX_CREATE_GAS_COST + n_bytes * TX_DATA_NON_ZERO_GAS_EIP2028;
    let intrinsic_gas = transaction_intrinsic_gas(&tx, Address::default(), &header, &config)
        .expect("Intrinsic gas");
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
    let expected_gas_cost = TX_CREATE_GAS_COST
        + n_bytes * TX_DATA_NON_ZERO_GAS_EIP2028
        + n_words * TX_INIT_CODE_WORD_GAS_COST;
    let intrinsic_gas = transaction_intrinsic_gas(&tx, Address::default(), &header, &config)
        .expect("Intrinsic gas");
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
    let intrinsic_gas = transaction_intrinsic_gas(&tx, Address::default(), &header, &config)
        .expect("Intrinsic gas");
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

#[tokio::test]
async fn validate_transaction_rejects_oversize_non_blob() {
    // EIP-1559 tx with serialized RLP > MAX_TX_SIZE must be rejected at
    // admission with `TxSizeExceeded`. The size cap is the first
    // size-themed check; it runs before init-code, intrinsic gas, and
    // balance lookups, so an unsigned tx with no sender state is enough.
    use ethrex_common::types::MAX_TX_SIZE;

    let (config, header) = build_basic_config_and_header(false, false);
    let store = setup_storage(config, header).await.expect("Storage setup");
    let blockchain = Blockchain::default_with_store(store);

    // Pad calldata above MAX_TX_SIZE so the *encoded* tx is also oversized.
    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        data: Bytes::from(vec![0u8; MAX_TX_SIZE + 1]),
        ..Default::default()
    });

    let res = blockchain
        .validate_transaction(&tx, Address::random())
        .await;
    match res {
        Err(MempoolError::TxSizeExceeded { actual, limit }) => {
            assert!(actual > limit);
            assert_eq!(limit, MAX_TX_SIZE);
        }
        other => panic!("expected TxSizeExceeded, got {:?}", other),
    }
}

#[test]
fn test_filter_mempool_transactions() {
    let plain_tx_decoded = Transaction::decode_canonical(&hex::decode("f86d80843baa0c4082f618946177843db3138ae69679a54b95cf345ed759450d870aa87bee538000808360306ba0151ccc02146b9b11adf516e6787b59acae3e76544fdcd75e77e67c6b598ce65da064c5dd5aae2fbb535830ebbdad0234975cd7ece3562013b63ea18cc0df6c97d4").unwrap()).unwrap();
    let plain_tx_sender = plain_tx_decoded.sender(&NativeCrypto).unwrap();
    let plain_tx = MempoolTransaction::new(plain_tx_decoded, plain_tx_sender);
    let blob_tx_decoded = Transaction::decode_canonical(&hex::decode("03f88f0780843b9aca008506fc23ac00830186a09400000000000000000000000000000000000001008080c001e1a0010657f37554c781402a22917dee2f75def7ab966d7b770905398eba3c44401401a0840650aa8f74d2b07f40067dc33b715078d73422f01da17abdbd11e02bbdfda9a04b2260f6022bf53eadb337b3e59514936f7317d872defb891a708ee279bdca90").unwrap()).unwrap();
    let blob_tx_sender = blob_tx_decoded.sender(&NativeCrypto).unwrap();
    let blob_tx = MempoolTransaction::new(blob_tx_decoded, blob_tx_sender);
    let plain_tx_hash = plain_tx.hash(&NativeCrypto);
    let blob_tx_hash = blob_tx.hash(&NativeCrypto);
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

#[test]
fn blob_txs_are_not_evicted_by_regular_tx_flood() {
    // Regression: blob txs live in a dedicated sub-pool, so a flood of regular
    // txs that fills (and evicts from) the regular pool must not reduce the set
    // of retained blob txs. Pre-fix, blob txs shared the regular FIFO and were
    // flushed out by regular-tx pressure, starving block building of blobs.
    let regular_cap = 4;
    let mempool = Mempool::new(regular_cap);

    // Insert more blob txs than the regular cap, so the blob set can only be
    // fully retained if blobs are NOT bound by the regular cap (bundle inserted
    // first, mirroring add_blob_transaction_to_pool).
    let blob_count = regular_cap + 2;
    let mut blob_hashes = Vec::new();
    for i in 0..blob_count {
        let bundle = BlobsBundle {
            blobs: vec![[i as u8; BYTES_PER_BLOB]],
            commitments: vec![[i as u8; 48]],
            proofs: vec![[i as u8; 48]],
            version: 0,
        };
        let blob_tx = Transaction::EIP4844Transaction(EIP4844Transaction {
            gas: 21_000,
            to: Address::from_low_u64_be(1),
            ..Default::default()
        });
        let blob_hash = H256::random();
        let blob_sender = H160::random();
        mempool.add_blobs_bundle(blob_hash, bundle).unwrap();
        mempool
            .add_transaction(
                blob_hash,
                blob_sender,
                MempoolTransaction::new(blob_tx, blob_sender),
            )
            .expect("Failed to add blob transaction");
        blob_hashes.push(blob_hash);
    }

    // Flood with regular txs far beyond the regular cap, tracking the first one
    // so we can confirm the flood actually triggered regular-tx eviction.
    let first_regular_hash = H256::random();
    for i in 0..(regular_cap * 10) {
        let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
            to: TxKind::Call(Address::from_low_u64_be(2)),
            ..Default::default()
        });
        let sender = H160::random();
        let hash = if i == 0 {
            first_regular_hash
        } else {
            H256::random()
        };
        mempool
            .add_transaction(hash, sender, MempoolTransaction::new(tx, sender))
            .expect("Failed to add regular transaction");
    }

    // The flood must have evicted regular txs (proving it exceeded the cap that
    // pre-fix would also have flushed blobs).
    assert!(
        !mempool
            .contains_tx(first_regular_hash)
            .expect("contains_tx should succeed"),
        "regular-tx flood did not evict regular txs; test is not exercising eviction"
    );

    // Despite the eviction, every blob tx must still be retained (100% blob
    // retention vs a capped regular pool).
    for blob_hash in blob_hashes {
        assert!(
            mempool
                .contains_tx(blob_hash)
                .expect("contains_tx should succeed"),
            "blob tx {blob_hash:?} was evicted by a regular-tx flood"
        );
    }
}

// Inserts a 1-blob tx straight into the pool (bypassing validation) with the
// given nonce and blob fee; returns its hash.
fn add_blob_tx(mempool: &Mempool, nonce: u64, blob_fee: u64) -> H256 {
    let bundle = BlobsBundle {
        blobs: vec![[0u8; BYTES_PER_BLOB]],
        commitments: vec![[0u8; 48]],
        proofs: vec![[0u8; 48]],
        version: 0,
    };
    let tx = Transaction::EIP4844Transaction(EIP4844Transaction {
        nonce,
        gas: 21_000,
        max_fee_per_blob_gas: blob_fee.into(),
        to: Address::from_low_u64_be(1),
        ..Default::default()
    });
    let hash = H256::random();
    let sender = H160::random();
    mempool.add_blobs_bundle(hash, bundle).unwrap();
    mempool
        .add_transaction(hash, sender, MempoolTransaction::new(tx, sender))
        .expect("Failed to add blob transaction");
    hash
}

// Like `add_blob_tx` but with an explicit sender; returns its hash.
fn add_blob_tx_with_sender(mempool: &Mempool, sender: Address, nonce: u64) -> H256 {
    let bundle = BlobsBundle {
        blobs: vec![[0u8; BYTES_PER_BLOB]],
        commitments: vec![[0u8; 48]],
        proofs: vec![[0u8; 48]],
        version: 0,
    };
    let tx = Transaction::EIP4844Transaction(EIP4844Transaction {
        nonce,
        gas: 21_000,
        max_fee_per_blob_gas: 1.into(),
        to: Address::from_low_u64_be(1),
        ..Default::default()
    });
    let hash = H256::random();
    mempool.add_blobs_bundle(hash, bundle).unwrap();
    mempool
        .add_transaction(hash, sender, MempoolTransaction::new(tx, sender))
        .expect("Failed to add blob transaction");
    hash
}

#[test]
fn blob_txs_lists_only_blob_txs_with_sender_and_nonce() {
    let mempool = Mempool::new(MEMPOOL_MAX_SIZE_TEST);
    let sender = H160::random();
    let blob0 = add_blob_tx_with_sender(&mempool, sender, 0);
    let blob1 = add_blob_tx_with_sender(&mempool, sender, 1);

    // A regular tx must not appear in the blob listing.
    let plain = Transaction::EIP1559Transaction(EIP1559Transaction {
        nonce: 0,
        gas_limit: 21_000,
        to: TxKind::Call(Address::from_low_u64_be(1)),
        ..Default::default()
    });
    let plain_hash = plain.hash(&NativeCrypto);
    mempool
        .add_transaction(plain_hash, sender, MempoolTransaction::new(plain, sender))
        .unwrap();

    let mut got = mempool.blob_txs().unwrap();
    got.sort_by_key(|(_, _, nonce)| *nonce);
    assert_eq!(got, vec![(blob0, sender, 0), (blob1, sender, 1)]);
}

#[test]
fn blob_eviction_keeps_includable_low_nonce_tx() {
    // When the blob sub-pool is over its cap, eviction must drop the least
    // includable blob tx (highest nonce), not the earliest-inserted one. A FIFO
    // would evict the first-added low-nonce tx (which is the includable one);
    // the value/nonce-ordered policy keeps it.
    let blob_cap = 4;
    let mempool = Mempool::new(MEMPOOL_MAX_SIZE_TEST).with_max_blob_mempool_size(blob_cap);

    // Insert FIRST (oldest): an includable low-nonce, high-fee blob tx.
    let keep = add_blob_tx(&mempool, 0, 1000);

    // Then flood the blob sub-pool past its cap with higher-nonce, low-fee txs.
    for n in 0..(blob_cap as u64 + 4) {
        add_blob_tx(&mempool, 100 + n, 1);
    }

    // FIFO would have evicted `keep` (oldest); the new policy must keep it.
    assert!(
        mempool
            .contains_tx(keep)
            .expect("contains_tx should succeed"),
        "includable low-nonce blob tx was evicted in favor of high-nonce ones"
    );
}

#[test]
fn blob_eviction_offset_is_per_sender_not_cross_sender() {
    // Regression: eviction ranks by nonce offset *within a sender*, not by raw
    // cross-sender nonce. A high-throughput sender (e.g. a rollup sequencer)
    // accumulates large on-wire nonces while staying perfectly includable; a
    // raw cross-sender comparison would evict its txs first. The deepest-in-its-
    // own-queue blob must be dropped instead.
    let blob_cap = 4;
    let mempool = Mempool::new(MEMPOOL_MAX_SIZE_TEST).with_max_blob_mempool_size(blob_cap);

    // Sequencer: a single blob at a very high nonce (offset 0 from its own min).
    let sequencer = H160::random();
    let seq_tx = add_blob_tx_with_sender(&mempool, sequencer, 1_000_000);

    // A backlogged sender with a deep, contiguous queue (offsets 0..=4).
    let backlogged = H160::random();
    let deep: Vec<H256> = (0..=4)
        .map(|n| add_blob_tx_with_sender(&mempool, backlogged, n))
        .collect();

    // 6 blobs, cap 4 ⇒ 2 backlogged blobs evicted. The sequencer's high-nonce
    // tx must survive: under the old cross-sender nonce key it had the globally
    // highest nonce and would have been the first evicted.
    assert!(
        mempool.contains_tx(seq_tx).unwrap(),
        "high-nonce sequencer blob was wrongly evicted by cross-sender nonce"
    );
    // Its lowest-offset (nonce 0) blob is the most includable and must stay.
    assert!(
        mempool.contains_tx(deep[0]).unwrap(),
        "includable nonce-0 blob must stay"
    );
    let present_deep = deep
        .iter()
        .filter(|h| mempool.contains_tx(**h).unwrap())
        .count();
    assert_eq!(
        present_deep, 3,
        "two of the backlogged sender's blobs evicted"
    );
}

mod alternates {
    use super::*;
    use ethrex_blockchain::mempool::MAX_ALTERNATES_PER_HASH;
    use std::time::Duration;

    fn h(n: u8) -> H256 {
        let mut b = [0u8; 32];
        b[31] = n;
        H256::from(b)
    }

    /// Helper that reserves `hashes` with synthetic per-hash (type, size)
    /// metadata. Tests that don't care about the metadata can use this.
    fn reserve(mp: &Mempool, hashes: &[H256], announcer: H256) -> Vec<H256> {
        let types = vec![0u8; hashes.len()];
        let sizes = vec![0usize; hashes.len()];
        mp.reserve_unknown_hashes(hashes, &types, &sizes, announcer)
            .unwrap()
    }

    #[test]
    fn primary_requester_is_not_an_alternate() {
        let mp = Mempool::new(64);
        let peer_a = h(1);
        let tx = h(0xa);

        // peer_a is the first announcer: it becomes the primary requester
        // (returned in `unknown`), so no alternates entry should be created.
        let unknown = reserve(&mp, &[tx], peer_a);
        assert_eq!(unknown, vec![tx]);
        assert!(mp.pop_alternate(tx).unwrap().is_none());
    }

    #[test]
    fn second_announcer_recorded_as_alternate() {
        let mp = Mempool::new(64);
        let peer_a = h(1);
        let peer_b = h(2);
        let tx_a = h(0xa);
        let tx_b = h(0xb);

        let unknown = reserve(&mp, &[tx_a, tx_b], peer_a);
        assert_eq!(unknown, vec![tx_a, tx_b]);

        // peer_b sees the same hashes already in-flight from peer_a, so it
        // should be filed as an alternate for each hash.
        let unknown = reserve(&mp, &[tx_a, tx_b], peer_b);
        assert!(unknown.is_empty());

        let alt_a = mp.pop_alternate(tx_a).unwrap().expect("alt for tx_a");
        let alt_b = mp.pop_alternate(tx_b).unwrap().expect("alt for tx_b");
        assert_eq!(alt_a.peer_id, peer_b);
        assert_eq!(alt_b.peer_id, peer_b);
    }

    #[test]
    fn alternate_carries_per_hash_type_and_size() {
        let mp = Mempool::new(64);
        let primary = h(1);
        let alt_peer = h(2);
        let tx = h(0xa);

        // primary announces with one (type, size); alt announces with another.
        // The stored alternate must carry the alt peer's metadata, not the
        // primary's, so a later retry validates the alt peer's response
        // against the alt's own announcement.
        mp.reserve_unknown_hashes(&[tx], &[0x03], &[42], primary)
            .unwrap();
        mp.reserve_unknown_hashes(&[tx], &[0x03], &[131072], alt_peer)
            .unwrap();

        let popped = mp.pop_alternate(tx).unwrap().expect("alt present");
        assert_eq!(popped.peer_id, alt_peer);
        assert_eq!(popped.tx_type, 0x03);
        assert_eq!(popped.tx_size, 131072);
    }

    #[test]
    fn pop_alternates_is_fifo_and_drains() {
        let mp = Mempool::new(64);
        let tx = h(0xab);
        let primary = h(99);
        let p1 = h(1);
        let p2 = h(2);
        let p3 = h(3);

        reserve(&mp, &[tx], primary);
        reserve(&mp, &[tx], p1);
        reserve(&mp, &[tx], p2);
        reserve(&mp, &[tx], p3);

        assert_eq!(mp.pop_alternate(tx).unwrap().unwrap().peer_id, p1);
        assert_eq!(mp.pop_alternate(tx).unwrap().unwrap().peer_id, p2);
        assert_eq!(mp.pop_alternate(tx).unwrap().unwrap().peer_id, p3);
        assert!(mp.pop_alternate(tx).unwrap().is_none());
    }

    #[test]
    fn alternates_capped() {
        let mp = Mempool::new(64);
        let tx = h(0xcd);
        let primary = h(0xff);
        reserve(&mp, &[tx], primary);
        for i in 0..(MAX_ALTERNATES_PER_HASH + 4) {
            reserve(&mp, &[tx], h(i as u8 + 1));
        }
        let mut count = 0;
        while mp.pop_alternate(tx).unwrap().is_some() {
            count += 1;
        }
        assert_eq!(count, MAX_ALTERNATES_PER_HASH);
    }

    #[test]
    fn duplicate_announcer_not_double_counted() {
        let mp = Mempool::new(64);
        let tx = h(0xef);
        let primary = h(0xff);
        let peer = h(42);
        reserve(&mp, &[tx], primary);
        reserve(&mp, &[tx], peer);
        reserve(&mp, &[tx], peer);
        reserve(&mp, &[tx], peer);
        let popped = mp.pop_alternate(tx).unwrap().expect("alt present");
        assert_eq!(popped.peer_id, peer);
        assert!(mp.pop_alternate(tx).unwrap().is_none());
    }

    #[test]
    fn prune_alternates_drops_stale_entries() {
        let mp = Mempool::new(64);
        let tx = h(0xaa);
        reserve(&mp, &[tx], h(1));
        reserve(&mp, &[tx], h(2));
        // Sleep well past the TTL so a loaded CI scheduler that gives us a
        // shorter-than-asked sleep still observes the entries as stale.
        std::thread::sleep(Duration::from_millis(20));
        mp.prune_alternates(Duration::from_millis(5)).unwrap();
        assert!(mp.pop_alternate(tx).unwrap().is_none());
    }
}

// `mempool-calldata-floor-gas-gap` (issue #6889): on Prague (the active fork)
// mempool admission must compute intrinsic gas the same way LEVM does at
// execution, and must reject empty EIP-7702 authorization lists. Otherwise txs
// that the VM will reject at inclusion are admitted, polluting the pool.

/// Prague config (Amsterdam NOT active); `config.fork(timestamp)` resolves to
/// Prague, so `intrinsic_gas_dimensions` exercises its pre-Amsterdam sub-path
/// (flat EIP-7702 auth-list cost + EIP-7623 calldata floor).
fn prague_config_and_header() -> (ChainConfig, BlockHeader) {
    let config = ChainConfig {
        istanbul_block: Some(0),
        shanghai_time: Some(0),
        cancun_time: Some(0),
        prague_time: Some(0),
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

/// EIP-7702: each authorization tuple costs `PER_EMPTY_ACCOUNT_COST` (25000) of
/// intrinsic gas (LEVM charges it in `intrinsic_gas_dimensions`). Mempool
/// admission must charge it too. Unfixed mempool omits the auth-list cost
/// entirely on the pre-Amsterdam path.
#[test]
fn prague_eip7702_intrinsic_includes_auth_list_cost() {
    use ethrex_levm::constants::PER_EMPTY_ACCOUNT_COST;

    let (config, header) = prague_config_and_header();

    let auth = AuthorizationTuple::default();
    let tx = Transaction::EIP7702Transaction(EIP7702Transaction {
        chain_id: 0,
        nonce: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit: 1_000_000,
        to: Address::from_low_u64_be(1),
        value: U256::zero(),
        data: Bytes::default(),
        access_list: Default::default(),
        authorization_list: vec![auth, auth], // 2 tuples
        ..Default::default()
    });

    let expected = TX_GAS_COST + 2 * PER_EMPTY_ACCOUNT_COST; // 21000 + 50000
    let got = transaction_intrinsic_gas(&tx, Address::default(), &header, &config)
        .expect("intrinsic gas");
    assert_eq!(
        got, expected,
        "Prague type-4 mempool intrinsic must include PER_EMPTY_ACCOUNT_COST per \
         auth tuple (matches LEVM); unfixed mempool omits it"
    );
}

/// EIP-7623: a tx whose calldata floor exceeds its execution intrinsic must be
/// charged the floor. 1000 zero-bytes → legacy intrinsic 25000, floor 31000.
/// Unfixed mempool returns the sub-floor 25000 and admits a tx the VM rejects.
#[test]
fn prague_intrinsic_applies_eip7623_calldata_floor() {
    let (config, header) = prague_config_and_header();

    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        nonce: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit: 1_000_000,
        to: TxKind::Call(Address::from_low_u64_be(1)),
        value: U256::zero(),
        data: Bytes::from(vec![0u8; 1000]),
        access_list: Default::default(),
        ..Default::default()
    });

    // floor = TX_BASE_COST + 1000 tokens * 10 = 31000; legacy intrinsic = 25000.
    let expected_floor = 31_000;
    let got = transaction_intrinsic_gas(&tx, Address::default(), &header, &config)
        .expect("intrinsic gas");
    assert_eq!(
        got, expected_floor,
        "Prague mempool intrinsic must apply the EIP-7623 calldata floor (matches \
         LEVM); unfixed mempool returns the sub-floor legacy value 25000"
    );
}

/// EIP-7702: an empty `authorization_list` makes a type-4 tx invalid
/// (`validate_type_4_tx` rejects it). Mempool admission must reject it too.
/// Unfixed mempool admits it (sender is funded so the balance check passes).
#[tokio::test]
async fn validate_transaction_rejects_empty_auth_list() {
    let sender = Address::from_low_u64_be(0x1234);
    let mut alloc = BTreeMap::new();
    alloc.insert(
        sender,
        GenesisAccount {
            code: Bytes::new(),
            storage: BTreeMap::new(),
            balance: U256::from(10).pow(U256::from(20)), // 100 ETH
            nonce: 0,
        },
    );
    let genesis = Genesis {
        config: ChainConfig {
            chain_id: 1,
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
            terminal_total_difficulty: Some(0),
            terminal_total_difficulty_passed: true,
            shanghai_time: Some(0),
            cancun_time: Some(0),
            prague_time: Some(0),
            ..Default::default()
        },
        alloc,
        gas_limit: 30_000_000,
        base_fee_per_gas: Some(0),
        ..Default::default()
    };

    let mut store = Store::new("", EngineType::InMemory).expect("open in-memory store");
    store
        .add_initial_state(genesis)
        .await
        .expect("initialize genesis");
    let blockchain = Blockchain::default_with_store(store);

    let tx = Transaction::EIP7702Transaction(EIP7702Transaction {
        chain_id: 1,
        nonce: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit: 100_000,
        to: Address::from_low_u64_be(1),
        value: U256::zero(),
        data: Bytes::default(),
        access_list: Default::default(),
        authorization_list: vec![], // EMPTY — invalid per EIP-7702
        ..Default::default()
    });

    let res = blockchain.validate_transaction(&tx, sender).await;
    assert!(
        matches!(res, Err(MempoolError::EmptyAuthorizationList)),
        "type-4 tx with an empty authorization_list must be rejected at admission \
         with EmptyAuthorizationList; unfixed mempool admits it (got {res:?})"
    );
}

/// The empty-auth-list check must run before the intrinsic-gas check, so a
/// type-4 tx that is both empty-auth and under-gassed reports the structural
/// fault (`EmptyAuthorizationList`) rather than the downstream gas symptom —
/// matching LEVM's `validate_type_4_tx` ordering.
#[tokio::test]
async fn validate_transaction_empty_auth_reported_before_intrinsic() {
    let (config, header) = prague_config_and_header();
    let store = setup_storage(config, header).await.expect("Storage setup");
    let blockchain = Blockchain::default_with_store(store);

    let tx = Transaction::EIP7702Transaction(EIP7702Transaction {
        chain_id: 0,
        nonce: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit: 1_000, // below the 21000 intrinsic
        to: Address::from_low_u64_be(1),
        value: U256::zero(),
        data: Bytes::default(),
        access_list: Default::default(),
        authorization_list: vec![], // empty — the structural fault
        ..Default::default()
    });

    let res = blockchain
        .validate_transaction(&tx, Address::random())
        .await;
    assert!(
        matches!(res, Err(MempoolError::EmptyAuthorizationList)),
        "empty auth-list must be reported before the intrinsic-gas check (got {res:?})"
    );
}

/// EIP-7702 (type-4) txs only exist from Prague onward; LEVM rejects them with
/// `Type4TxPreFork` otherwise. Mempool admission must mirror that gate so a
/// pre-Prague node does not pool a type-4 tx that execution will reject.
#[tokio::test]
async fn validate_transaction_rejects_pre_prague_eip7702() {
    // Cancun active, Prague NOT active.
    let config = ChainConfig {
        istanbul_block: Some(0),
        shanghai_time: Some(0),
        cancun_time: Some(0),
        ..Default::default()
    };
    let header = BlockHeader {
        number: 5,
        timestamp: 5,
        gas_limit: 100_000_000,
        gas_used: 0,
        ..Default::default()
    };
    let store = setup_storage(config, header).await.expect("Storage setup");
    let blockchain = Blockchain::default_with_store(store);

    let tx = Transaction::EIP7702Transaction(EIP7702Transaction {
        chain_id: 0,
        nonce: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit: 100_000,
        to: Address::from_low_u64_be(1),
        value: U256::zero(),
        data: Bytes::default(),
        access_list: Default::default(),
        authorization_list: vec![AuthorizationTuple::default()], // non-empty
        ..Default::default()
    });

    let res = blockchain
        .validate_transaction(&tx, Address::random())
        .await;
    assert!(
        matches!(res, Err(MempoolError::Eip7702TxPreFork)),
        "pre-Prague type-4 tx must be rejected with Eip7702TxPreFork (got {res:?})"
    );
}

// ----------------------------------------------------------------------------
// Gap-admission tests
// ----------------------------------------------------------------------------
//
// These tests exercise the rule that, when the mempool is heavily occupied,
// incoming transactions with a nonce gap relative to the sender's on-chain
// nonce are rejected. Replacements (same nonce as a tx already in the pool)
// must bypass this rule, since they are not gapped.

const GAP_TEST_MEMPOOL_MAX: usize = 10;
const GAP_TEST_THRESHOLD_PCT: u8 = 90;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

async fn setup_funded_store(sender: Address) -> (Store, u64) {
    let file = File::open(workspace_root().join("fixtures/genesis/execution-api.json"))
        .expect("Failed to open genesis file");
    let reader = BufReader::new(file);
    let mut genesis: ethrex_common::types::Genesis =
        serde_json::from_reader(reader).expect("Failed to deserialize genesis file");

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

    let mut store =
        Store::new("store.db", EngineType::InMemory).expect("Failed to build DB for testing");

    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to add genesis state");

    (store, chain_id)
}

/// Build a non-blob tx with the given nonce. The signature is dummy — the tests
/// here exercise `validate_transaction`, which never inspects the signature.
fn build_tx(chain_id: u64, nonce: u64) -> Transaction {
    Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id,
        nonce,
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: 1_000_000_000,
        gas_limit: 100_000,
        to: TxKind::Call(Address::from_low_u64_be(0xABBA)),
        value: U256::zero(),
        data: Bytes::default(),
        access_list: Default::default(),
        ..Default::default()
    })
}

/// Inject `count` dummy transactions from random senders directly into the
/// mempool, bypassing validation. Used to push occupancy above a threshold.
fn fill_mempool(mempool: &Mempool, count: usize) {
    for i in 0..count {
        let sender = Address::from_low_u64_be(0x1000 + i as u64);
        let hash = H256::from_low_u64_be(0x1000 + i as u64);
        let tx = build_tx(1, 0);
        mempool
            .add_transaction(hash, sender, MempoolTransaction::new(tx, sender))
            .expect("Failed to add transaction");
    }
}

fn blockchain_with_threshold(store: Store, threshold_pct: u8) -> Blockchain {
    Blockchain::new(
        store,
        BlockchainOptions {
            max_mempool_size: GAP_TEST_MEMPOOL_MAX,
            gap_admit_occupancy_threshold: threshold_pct,
            ..Default::default()
        },
    )
}

#[tokio::test]
async fn gap_admission_rejected_when_pool_above_threshold() {
    let sender = Address::from_low_u64_be(0xAAA);
    let (store, chain_id) = setup_funded_store(sender).await;
    let blockchain = blockchain_with_threshold(store, GAP_TEST_THRESHOLD_PCT);

    // Push occupancy to 100% (10/10) — well above 90%.
    fill_mempool(&blockchain.mempool, GAP_TEST_MEMPOOL_MAX);

    // On-chain nonce is 0; submitting nonce=5 introduces a gap.
    let gapped_tx = build_tx(chain_id, 5);
    let result = blockchain.validate_transaction(&gapped_tx, sender).await;
    assert!(
        matches!(
            result,
            Err(MempoolError::GapAdmissionDeniedUnderPressure { .. })
        ),
        "expected GapAdmissionDeniedUnderPressure, got {result:?}"
    );
}

#[tokio::test]
async fn gap_admission_accepted_when_pool_below_threshold() {
    let sender = Address::from_low_u64_be(0xAAB);
    let (store, chain_id) = setup_funded_store(sender).await;
    let blockchain = blockchain_with_threshold(store, GAP_TEST_THRESHOLD_PCT);

    // Push occupancy to 50% — below 90%.
    fill_mempool(&blockchain.mempool, GAP_TEST_MEMPOOL_MAX / 2);

    let gapped_tx = build_tx(chain_id, 5);
    let result = blockchain.validate_transaction(&gapped_tx, sender).await;
    assert!(
        result.is_ok(),
        "expected gapped tx to be accepted under low pressure, got {result:?}"
    );
}

#[tokio::test]
async fn gap_admission_disabled_at_threshold_100() {
    let sender = Address::from_low_u64_be(0xAAC);
    let (store, chain_id) = setup_funded_store(sender).await;
    // Threshold of 100 disables the check entirely.
    let blockchain = blockchain_with_threshold(store, 100);

    // Fill to 100% to make the pool maximally occupied.
    fill_mempool(&blockchain.mempool, GAP_TEST_MEMPOOL_MAX);

    let gapped_tx = build_tx(chain_id, 5);
    let result = blockchain.validate_transaction(&gapped_tx, sender).await;
    assert!(
        result.is_ok(),
        "expected gapped tx to be accepted when threshold is 100, got {result:?}"
    );
}

#[tokio::test]
async fn contiguous_nonce_tx_accepted_under_high_occupancy() {
    let sender = Address::from_low_u64_be(0xAAD);
    let (store, chain_id) = setup_funded_store(sender).await;
    let blockchain = blockchain_with_threshold(store, GAP_TEST_THRESHOLD_PCT);

    fill_mempool(&blockchain.mempool, GAP_TEST_MEMPOOL_MAX);

    // On-chain nonce is 0; submitting nonce=0 is contiguous.
    let contiguous_tx = build_tx(chain_id, 0);
    let result = blockchain
        .validate_transaction(&contiguous_tx, sender)
        .await;
    assert!(
        result.is_ok(),
        "expected contiguous tx to be accepted under high pressure, got {result:?}"
    );
}

#[tokio::test]
async fn replacement_at_existing_nonce_bypasses_gap_admission() {
    let sender = Address::from_low_u64_be(0xAAE);
    let (store, chain_id) = setup_funded_store(sender).await;
    let blockchain = blockchain_with_threshold(store, GAP_TEST_THRESHOLD_PCT);

    // First, add a gapped tx while the pool has plenty of room.
    let original_tx = build_tx(chain_id, 5);
    let original_hash = original_tx.hash(&NativeCrypto);
    blockchain
        .mempool
        .add_transaction(
            original_hash,
            sender,
            MempoolTransaction::new(original_tx, sender),
        )
        .expect("Failed to seed the pool with a tx at nonce 5");

    // Now push the pool above the threshold.
    fill_mempool(&blockchain.mempool, GAP_TEST_MEMPOOL_MAX.saturating_sub(1));

    // Build a replacement at the same nonce with strictly higher fees so that
    // `find_tx_to_replace` returns Some(_), bypassing the gap-admission rule.
    let replacement_tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id,
        nonce: 5,
        max_priority_fee_per_gas: 2,
        max_fee_per_gas: 2_000_000_000,
        gas_limit: 100_000,
        to: TxKind::Call(Address::from_low_u64_be(0xABBA)),
        value: U256::zero(),
        data: Bytes::default(),
        access_list: Default::default(),
        ..Default::default()
    });

    let result = blockchain
        .validate_transaction(&replacement_tx, sender)
        .await;
    assert!(
        matches!(result, Ok(Some(h)) if h == original_hash),
        "expected replacement to bypass gap-admission rule, got {result:?}"
    );
}
