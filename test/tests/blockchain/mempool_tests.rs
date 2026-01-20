use ethrex_blockchain::Blockchain;
use ethrex_blockchain::constants::MAX_INITCODE_SIZE;
use ethrex_blockchain::constants::{
    TX_ACCESS_LIST_ADDRESS_GAS, TX_ACCESS_LIST_STORAGE_KEY_GAS, TX_CREATE_GAS_COST,
    TX_DATA_NON_ZERO_GAS, TX_DATA_NON_ZERO_GAS_EIP2028, TX_DATA_ZERO_GAS_COST, TX_GAS_COST,
    TX_INIT_CODE_WORD_GAS_COST,
};
use ethrex_blockchain::error::MempoolError;
use ethrex_blockchain::mempool::{Mempool, transaction_intrinsic_gas};
use std::collections::HashMap;

use ethrex_common::types::{
    BYTES_PER_BLOB, BlobsBundle, BlockHeader, ChainConfig, EIP1559Transaction, EIP4844Transaction,
    MempoolTransaction, Transaction, TxKind,
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
