use bytes::Bytes;
use ethereum_types::{H256, U256};
use ethrex_common::{
    Address, Bloom, H160,
    constants::{EMPTY_KECCACK_HASH, EMPTY_TRIE_HASH},
    types::{
        AccountState, BlockBody, BlockHeader, ChainConfig, Code, Genesis, Receipt, Transaction,
        TxType,
    },
    utils::keccak,
};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_storage::{EngineType, Store, error::StoreError};
use std::{fs, str::FromStr};

#[tokio::test]
async fn test_in_memory_store() {
    test_store_suite(EngineType::InMemory).await;
}

#[cfg(feature = "rocksdb")]
#[tokio::test]
async fn test_rocksdb_store() {
    test_store_suite(EngineType::RocksDB).await;
}

// Creates an empty store, runs the test and then removes the store (if needed)
async fn run_test<F, Fut>(test_func: F, engine_type: EngineType)
where
    F: FnOnce(Store) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let nonce: u64 = H256::random().to_low_u64_be();
    let path = format!("store-test-db-{nonce}");
    // Remove preexistent DBs in case of a failed previous test
    if !matches!(engine_type, EngineType::InMemory) {
        remove_test_dbs(&path);
    };
    // Build a new store
    let store = Store::new(&path, engine_type).expect("Failed to create test db");
    // Run the test
    test_func(store).await;
    // Remove store (if needed)
    if !matches!(engine_type, EngineType::InMemory) {
        remove_test_dbs(&path);
    };
}

async fn test_store_suite(engine_type: EngineType) {
    run_test(test_store_block, engine_type).await;
    run_test(test_store_block_number, engine_type).await;
    run_test(test_store_block_receipt, engine_type).await;
    run_test(test_store_account_code, engine_type).await;
    run_test(test_store_block_tags, engine_type).await;
    run_test(test_chain_config_storage, engine_type).await;
    run_test(test_genesis_block, engine_type).await;
    run_test(test_iter_accounts, engine_type).await;
    run_test(test_iter_storage, engine_type).await;
}

async fn test_iter_accounts(store: Store) {
    let mut accounts: Vec<_> = (0u64..1_000)
        .map(|i| {
            (
                keccak(i.to_be_bytes()),
                AccountState {
                    nonce: 2 * i,
                    balance: U256::from(3 * i),
                    code_hash: *EMPTY_KECCACK_HASH,
                    storage_root: *EMPTY_TRIE_HASH,
                },
            )
        })
        .collect();
    accounts.sort_by_key(|a| a.0);
    let mut trie = store.open_direct_state_trie(*EMPTY_TRIE_HASH).unwrap();
    for (address, state) in &accounts {
        trie.insert(address.0.to_vec(), state.encode_to_vec())
            .unwrap();
    }
    let state_root = trie.hash().unwrap();
    let pivot = H256::random();
    let pos = accounts.partition_point(|(key, _)| key < &pivot);
    let account_iter = store.iter_accounts_from(state_root, pivot).unwrap();
    for (expected, actual) in std::iter::zip(accounts.drain(pos..), account_iter) {
        assert_eq!(expected, actual);
    }
}

async fn test_iter_storage(store: Store) {
    let address = keccak(12345u64.to_be_bytes());
    let mut slots: Vec<_> = (0u64..1_000)
        .map(|i| (keccak(i.to_be_bytes()), U256::from(2 * i)))
        .collect();
    slots.sort_by_key(|a| a.0);
    let mut trie = store
        .open_direct_storage_trie(address, *EMPTY_TRIE_HASH)
        .unwrap();
    for (slot, value) in &slots {
        trie.insert(slot.0.to_vec(), value.encode_to_vec()).unwrap();
    }
    let storage_root = trie.hash().unwrap();
    let mut trie = store.open_direct_state_trie(*EMPTY_TRIE_HASH).unwrap();
    trie.insert(
        address.0.to_vec(),
        AccountState {
            nonce: 1,
            balance: U256::zero(),
            storage_root,
            code_hash: *EMPTY_KECCACK_HASH,
        }
        .encode_to_vec(),
    )
    .unwrap();
    let state_root = trie.hash().unwrap();
    let pivot = H256::random();
    let pos = slots.partition_point(|(key, _)| key < &pivot);
    let storage_iter = store
        .iter_storage_from(state_root, address, pivot)
        .unwrap()
        .unwrap();
    for (expected, actual) in std::iter::zip(slots.drain(pos..), storage_iter) {
        assert_eq!(expected, actual);
    }
}

async fn test_genesis_block(mut store: Store) {
    const GENESIS_KURTOSIS: &str = include_str!("../../../fixtures/genesis/kurtosis.json");
    const GENESIS_HIVE: &str = include_str!("../../../fixtures/genesis/hive.json");
    assert_ne!(GENESIS_KURTOSIS, GENESIS_HIVE);
    let genesis_kurtosis: Genesis =
        serde_json::from_str(GENESIS_KURTOSIS).expect("deserialize kurtosis.json");
    let genesis_hive: Genesis = serde_json::from_str(GENESIS_HIVE).expect("deserialize hive.json");
    store
        .add_initial_state(genesis_kurtosis.clone())
        .await
        .expect("first genesis");
    store
        .add_initial_state(genesis_kurtosis)
        .await
        .expect("second genesis with same block");
    let result = store.add_initial_state(genesis_hive).await;
    assert!(result.is_err());
    assert!(matches!(result, Err(StoreError::IncompatibleChainConfig)));
}

fn remove_test_dbs(path: &str) {
    // Removes all test databases from filesystem
    if std::path::Path::new(path).exists() {
        fs::remove_dir_all(path).expect("Failed to clean test db dir");
    }
}

async fn test_store_block(store: Store) {
    let (block_header, block_body) = create_block_for_testing();
    let block_number = 6;
    let hash = block_header.hash();

    store
        .add_block_header(hash, block_header.clone())
        .await
        .unwrap();
    store
        .add_block_body(hash, block_body.clone())
        .await
        .unwrap();
    store
        .forkchoice_update(vec![], block_number, hash, None, None)
        .await
        .unwrap();

    let stored_header = store.get_block_header(block_number).unwrap().unwrap();
    let stored_body = store.get_block_body(block_number).await.unwrap().unwrap();

    // Ensure both headers have their hashes computed for comparison
    let _ = stored_header.hash();
    let _ = block_header.hash();
    assert_eq!(stored_header, block_header);
    assert_eq!(stored_body, block_body);
}

fn create_block_for_testing() -> (BlockHeader, BlockBody) {
    let block_header = BlockHeader {
        parent_hash: H256::from_str(
            "0x1ac1bf1eef97dc6b03daba5af3b89881b7ae4bc1600dc434f450a9ec34d44999",
        )
        .unwrap(),
        ommers_hash: H256::from_str(
            "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
        )
        .unwrap(),
        coinbase: Address::from_str("0x2adc25665018aa1fe0e6bc666dac8fc2697ff9ba").unwrap(),
        state_root: H256::from_str(
            "0x9de6f95cb4ff4ef22a73705d6ba38c4b927c7bca9887ef5d24a734bb863218d9",
        )
        .unwrap(),
        transactions_root: H256::from_str(
            "0x578602b2b7e3a3291c3eefca3a08bc13c0d194f9845a39b6f3bcf843d9fed79d",
        )
        .unwrap(),
        receipts_root: H256::from_str(
            "0x035d56bac3f47246c5eed0e6642ca40dc262f9144b582f058bc23ded72aa72fa",
        )
        .unwrap(),
        logs_bloom: Bloom::from([0; 256]),
        difficulty: U256::zero(),
        number: 1,
        gas_limit: 0x016345785d8a0000,
        gas_used: 0xa8de,
        timestamp: 0x03e8,
        extra_data: Bytes::new(),
        prev_randao: H256::zero(),
        nonce: 0x0000000000000000,
        base_fee_per_gas: Some(0x07),
        withdrawals_root: Some(
            H256::from_str("0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421")
                .unwrap(),
        ),
        blob_gas_used: Some(0x00),
        excess_blob_gas: Some(0x00),
        parent_beacon_block_root: Some(H256::zero()),
        requests_hash: Some(*EMPTY_KECCACK_HASH),
        ..Default::default()
    };
    let block_body = BlockBody {
        transactions: vec![Transaction::decode(&hex::decode("b86f02f86c8330182480114e82f618946177843db3138ae69679a54b95cf345ed759450d870aa87bee53800080c080a0151ccc02146b9b11adf516e6787b59acae3e76544fdcd75e77e67c6b598ce65da064c5dd5aae2fbb535830ebbdad0234975cd7ece3562013b63ea18cc0df6c97d4").unwrap()).unwrap(),
        Transaction::decode(&hex::decode("f86d80843baa0c4082f618946177843db3138ae69679a54b95cf345ed759450d870aa87bee538000808360306ba0151ccc02146b9b11adf516e6787b59acae3e76544fdcd75e77e67c6b598ce65da064c5dd5aae2fbb535830ebbdad0234975cd7ece3562013b63ea18cc0df6c97d4").unwrap()).unwrap()],
        ommers: Default::default(),
        withdrawals: Default::default(),
    };
    (block_header, block_body)
}

async fn test_store_block_number(store: Store) {
    let block_hash = H256::random();
    let block_number = 6;

    store
        .add_block_number(block_hash, block_number)
        .await
        .unwrap();

    let stored_number = store.get_block_number(block_hash).await.unwrap().unwrap();

    assert_eq!(stored_number, block_number);
}

async fn test_store_block_receipt(store: Store) {
    let receipt = Receipt {
        tx_type: TxType::EIP2930,
        succeeded: true,
        cumulative_gas_used: 1747,
        gas_spent: None,
        logs: vec![],
    };
    let block_number = 6;
    let index = 4;
    let block_header = BlockHeader::default();

    store
        .add_receipt(block_header.hash(), index, receipt.clone())
        .await
        .unwrap();

    store
        .add_block_header(block_header.hash(), block_header.clone())
        .await
        .unwrap();

    store
        .forkchoice_update(vec![], block_number, block_header.hash(), None, None)
        .await
        .unwrap();

    let stored_receipt = store
        .get_receipt(block_number, index)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(stored_receipt, receipt);
}

async fn test_store_account_code(store: Store) {
    let code = Code::from_bytecode(Bytes::from("kiwi"));
    let code_hash = code.hash;

    store.add_account_code(code.clone()).await.unwrap();

    let stored_code = store.get_account_code(code_hash).unwrap().unwrap();

    assert_eq!(stored_code, code);
}

async fn test_store_block_tags(store: Store) {
    let earliest_block_number = 0;
    let finalized_block_number = 7;
    let safe_block_number = 6;
    let latest_block_number = 8;
    let pending_block_number = 9;

    let (mut block_header, block_body) = create_block_for_testing();
    block_header.number = latest_block_number;
    let hash = block_header.hash();

    store
        .add_block_header(hash, block_header.clone())
        .await
        .unwrap();
    store
        .add_block_body(hash, block_body.clone())
        .await
        .unwrap();

    store
        .update_earliest_block_number(earliest_block_number)
        .await
        .unwrap();
    store
        .update_pending_block_number(pending_block_number)
        .await
        .unwrap();
    store
        .forkchoice_update(
            vec![],
            latest_block_number,
            hash,
            Some(safe_block_number),
            Some(finalized_block_number),
        )
        .await
        .unwrap();

    let stored_earliest_block_number = store.get_earliest_block_number().await.unwrap();
    let stored_finalized_block_number = store.get_finalized_block_number().await.unwrap().unwrap();
    let stored_latest_block_number = store.get_latest_block_number().await.unwrap();
    let stored_safe_block_number = store.get_safe_block_number().await.unwrap().unwrap();
    let stored_pending_block_number = store.get_pending_block_number().await.unwrap().unwrap();

    assert_eq!(earliest_block_number, stored_earliest_block_number);
    assert_eq!(finalized_block_number, stored_finalized_block_number);
    assert_eq!(safe_block_number, stored_safe_block_number);
    assert_eq!(latest_block_number, stored_latest_block_number);
    assert_eq!(pending_block_number, stored_pending_block_number);
}

async fn test_chain_config_storage(mut store: Store) {
    let chain_config = example_chain_config();
    store.set_chain_config(&chain_config).await.unwrap();
    let retrieved_chain_config = store.get_chain_config();
    assert_eq!(chain_config, retrieved_chain_config);
}

fn example_chain_config() -> ChainConfig {
    ChainConfig {
        chain_id: 3151908_u64,
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
        prague_time: Some(1718232101),
        terminal_total_difficulty: Some(58750000000000000000000),
        terminal_total_difficulty_passed: true,
        deposit_contract_address: H160::from_str("0x4242424242424242424242424242424242424242")
            .unwrap(),
        ..Default::default()
    }
}
