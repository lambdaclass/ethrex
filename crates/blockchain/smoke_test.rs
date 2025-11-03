#[cfg(test)]
mod blockchain_integration_test {
    use ethrex_common::{
        Address, Signature, U256,
        types::{Genesis, GenesisAccount, Transaction, TxKind, TxType},
        utils::keccak,
    };
    use ethrex_rlp::encode::PayloadRLPEncode;
    use secp256k1::{Message, SECP256K1, SecretKey};

    use std::{
        collections::{BTreeMap, HashMap},
        fs::File,
        io::BufReader,
    };

    use crate::{
        Blockchain,
        error::{ChainError, InvalidForkChoice},
        fork_choice::apply_fork_choice,
        is_canonical, latest_canonical_block_hash,
        payload::{BuildPayloadArgs, create_payload},
    };

    use bytes::Bytes;
    use ethrex_common::{
        H160, H256,
        types::{Block, BlockHeader, DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER},
    };
    use ethrex_storage::{EngineType, Store};

    #[tokio::test]
    async fn test_small_to_long_reorg() {
        // Store and genesis
        let store = test_store().await;
        let genesis_header = store.get_block_header(0).unwrap().unwrap();
        let genesis_hash = genesis_header.hash();

        // Create blockchain
        let blockchain = Blockchain::default_with_store(store.clone());

        // Add first block. We'll make it canonical.
        let block_1a = new_block(&store, &genesis_header).await;
        let hash_1a = block_1a.hash();
        blockchain.add_block(block_1a.clone()).unwrap();
        store
            .forkchoice_update(None, 1, hash_1a, None, None)
            .await
            .unwrap();
        let retrieved_1a = store.get_block_header(1).unwrap().unwrap();

        assert_eq!(retrieved_1a, block_1a.header);
        assert!(is_canonical(&store, 1, hash_1a).await.unwrap());

        // Add second block at height 1. Will not be canonical.
        let block_1b = new_block(&store, &genesis_header).await;
        let hash_1b = block_1b.hash();
        blockchain
            .add_block(block_1b.clone())
            .expect("Could not add block 1b.");
        let retrieved_1b = store.get_block_header_by_hash(hash_1b).unwrap().unwrap();

        assert_ne!(retrieved_1a, retrieved_1b);
        assert!(!is_canonical(&store, 1, hash_1b).await.unwrap());

        // Add a third block at height 2, child to the non canonical block.
        let block_2 = new_block(&store, &block_1b.header).await;
        let hash_2 = block_2.hash();
        blockchain
            .add_block(block_2.clone())
            .expect("Could not add block 2.");
        let retrieved_2 = store.get_block_header_by_hash(hash_2).unwrap();

        assert!(retrieved_2.is_some());
        assert!(store.get_canonical_block_hash(2).await.unwrap().is_none());

        // Receive block 2 as new head.
        apply_fork_choice(
            &store,
            block_2.hash(),
            genesis_header.hash(),
            genesis_header.hash(),
        )
        .await
        .unwrap();

        // Check that canonical blocks changed to the new branch.
        assert!(is_canonical(&store, 0, genesis_hash).await.unwrap());
        assert!(is_canonical(&store, 1, hash_1b).await.unwrap());
        assert!(is_canonical(&store, 2, hash_2).await.unwrap());
        assert!(!is_canonical(&store, 1, hash_1a).await.unwrap());
    }

    #[tokio::test]
    async fn test_sync_not_supported_yet() {
        let store = test_store().await;
        let genesis_header = store.get_block_header(0).unwrap().unwrap();

        // Create blockchain
        let blockchain = Blockchain::default_with_store(store.clone());

        // Build a single valid block.
        let block_1 = new_block(&store, &genesis_header).await;
        let hash_1 = block_1.hash();
        blockchain.add_block(block_1.clone()).unwrap();
        apply_fork_choice(&store, hash_1, H256::zero(), H256::zero())
            .await
            .unwrap();

        // Build a child, then change its parent, making it effectively a pending block.
        let mut block_2 = new_block(&store, &block_1.header).await;
        block_2.header.parent_hash = H256::random();
        let hash_2 = block_2.hash();
        let result = blockchain.add_block(block_2.clone());
        assert!(matches!(result, Err(ChainError::ParentNotFound)));

        // block 2 should now be pending.
        assert!(store.get_pending_block(hash_2).await.unwrap().is_some());

        let fc_result = apply_fork_choice(&store, hash_2, H256::zero(), H256::zero()).await;
        assert!(matches!(fc_result, Err(InvalidForkChoice::Syncing)));

        // block 2 should still be pending.
        assert!(store.get_pending_block(hash_2).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_reorg_from_long_to_short_chain() {
        // Store and genesis
        let store = test_store().await;
        let genesis_header = store.get_block_header(0).unwrap().unwrap();
        let genesis_hash = genesis_header.hash();

        // Create blockchain
        let blockchain = Blockchain::default_with_store(store.clone());

        // Add first block. Not canonical.
        let block_1a = new_block(&store, &genesis_header).await;
        let hash_1a = block_1a.hash();
        blockchain.add_block(block_1a.clone()).unwrap();
        let retrieved_1a = store.get_block_header_by_hash(hash_1a).unwrap().unwrap();

        assert!(!is_canonical(&store, 1, hash_1a).await.unwrap());

        // Add second block at height 1. Canonical.
        let block_1b = new_block(&store, &genesis_header).await;
        let hash_1b = block_1b.hash();
        blockchain
            .add_block(block_1b.clone())
            .expect("Could not add block 1b.");
        apply_fork_choice(&store, hash_1b, genesis_hash, genesis_hash)
            .await
            .unwrap();
        let retrieved_1b = store.get_block_header(1).unwrap().unwrap();

        assert_ne!(retrieved_1a, retrieved_1b);
        assert_eq!(retrieved_1b, block_1b.header);
        assert!(is_canonical(&store, 1, hash_1b).await.unwrap());
        assert_eq!(latest_canonical_block_hash(&store).await.unwrap(), hash_1b);

        // Add a third block at height 2, child to the canonical one.
        let block_2 = new_block(&store, &block_1b.header).await;
        let hash_2 = block_2.hash();
        blockchain
            .add_block(block_2.clone())
            .expect("Could not add block 2.");
        apply_fork_choice(&store, hash_2, genesis_hash, genesis_hash)
            .await
            .unwrap();
        let retrieved_2 = store.get_block_header_by_hash(hash_2).unwrap();
        assert_eq!(latest_canonical_block_hash(&store).await.unwrap(), hash_2);

        assert!(retrieved_2.is_some());
        assert!(is_canonical(&store, 2, hash_2).await.unwrap());
        assert_eq!(
            store.get_canonical_block_hash(2).await.unwrap().unwrap(),
            hash_2
        );

        // Receive block 1a as new head.
        apply_fork_choice(
            &store,
            block_1a.hash(),
            genesis_header.hash(),
            genesis_header.hash(),
        )
        .await
        .unwrap();

        // Check that canonical blocks changed to the new branch.
        assert!(is_canonical(&store, 0, genesis_hash).await.unwrap());
        assert!(is_canonical(&store, 1, hash_1a).await.unwrap());
        assert!(!is_canonical(&store, 1, hash_1b).await.unwrap());
        assert!(!is_canonical(&store, 2, hash_2).await.unwrap());
    }

    #[tokio::test]
    async fn new_head_with_canonical_ancestor_should_skip() {
        // Store and genesis
        let store = test_store().await;
        let genesis_header = store.get_block_header(0).unwrap().unwrap();
        let genesis_hash = genesis_header.hash();

        // Create blockchain
        let blockchain = Blockchain::default_with_store(store.clone());

        // Add block at height 1.
        let block_1 = new_block(&store, &genesis_header).await;
        let hash_1 = block_1.hash();
        blockchain
            .add_block(block_1.clone())
            .expect("Could not add block 1b.");

        // Add child at height 2.
        let block_2 = new_block(&store, &block_1.header).await;
        let hash_2 = block_2.hash();
        blockchain
            .add_block(block_2.clone())
            .expect("Could not add block 2.");

        assert!(!is_canonical(&store, 1, hash_1).await.unwrap());
        assert!(!is_canonical(&store, 2, hash_2).await.unwrap());

        // Make that chain the canonical one.
        apply_fork_choice(&store, hash_2, genesis_hash, genesis_hash)
            .await
            .unwrap();

        assert!(is_canonical(&store, 1, hash_1).await.unwrap());
        assert!(is_canonical(&store, 2, hash_2).await.unwrap());

        let result = apply_fork_choice(&store, hash_1, hash_1, hash_1).await;

        assert!(matches!(
            result,
            Err(InvalidForkChoice::NewHeadAlreadyCanonical)
        ));

        // Important blocks should still be the same as before.
        assert!(store.get_finalized_block_number().await.unwrap() == Some(0));
        assert!(store.get_safe_block_number().await.unwrap() == Some(0));
        assert!(store.get_latest_block_number().await.unwrap() == 2);
    }

    #[tokio::test]
    async fn latest_block_number_should_always_be_the_canonical_head() {
        // Goal: put a, b in the same branch, both canonical.
        // Then add one in a different branch. Check that the last one is still the same.

        // Store and genesis
        let store = test_store().await;
        let genesis_header = store.get_block_header(0).unwrap().unwrap();
        let genesis_hash = genesis_header.hash();

        // Create blockchain
        let blockchain = Blockchain::default_with_store(store.clone());

        // Add block at height 1.
        let block_1 = new_block(&store, &genesis_header).await;
        blockchain
            .add_block(block_1.clone())
            .expect("Could not add block 1b.");

        // Add child at height 2.
        let block_2 = new_block(&store, &block_1.header).await;
        let hash_2 = block_2.hash();
        blockchain
            .add_block(block_2.clone())
            .expect("Could not add block 2.");

        assert_eq!(
            latest_canonical_block_hash(&store).await.unwrap(),
            genesis_hash
        );

        // Make that chain the canonical one.
        apply_fork_choice(&store, hash_2, genesis_hash, genesis_hash)
            .await
            .unwrap();

        assert_eq!(latest_canonical_block_hash(&store).await.unwrap(), hash_2);

        // Add a new, non canonical block, starting from genesis.
        let block_1b = new_block(&store, &genesis_header).await;
        let hash_b = block_1b.hash();
        blockchain
            .add_block(block_1b.clone())
            .expect("Could not add block b.");

        // The latest block should be the same.
        assert_eq!(latest_canonical_block_hash(&store).await.unwrap(), hash_2);

        // if we apply fork choice to the new one, then we should
        apply_fork_choice(&store, hash_b, genesis_hash, genesis_hash)
            .await
            .unwrap();

        // The latest block should now be the new head.
        assert_eq!(latest_canonical_block_hash(&store).await.unwrap(), hash_b);
    }

    async fn new_block(store: &Store, parent: &BlockHeader) -> Block {
        let args = BuildPayloadArgs {
            parent: parent.hash(),
            timestamp: parent.timestamp + 12,
            fee_recipient: H160::random(),
            random: H256::random(),
            withdrawals: Some(Vec::new()),
            beacon_root: Some(H256::random()),
            version: 1,
            elasticity_multiplier: ELASTICITY_MULTIPLIER,
            gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
        };

        // Create blockchain
        let blockchain = Blockchain::default_with_store(store.clone());

        let block = create_payload(&args, store, Bytes::new()).unwrap();
        let result = blockchain.build_payload(block).unwrap();
        result.payload
    }

    async fn test_store() -> Store {
        // Get genesis
        let file = File::open("../../fixtures/genesis/execution-api.json")
            .expect("Failed to open genesis file");
        let reader = BufReader::new(file);
        let genesis = serde_json::from_reader(reader).expect("Failed to deserialize genesis file");

        // Build store with genesis
        let mut store =
            Store::new("store.db", EngineType::InMemory).expect("Failed to build DB for testing");

        store
            .add_initial_state(genesis)
            .await
            .expect("Failed to add genesis state");

        store
    }

    // signer

    struct Account {
        pub private_key: SecretKey,
        pub address: Address,
    }

    impl Account {
        pub fn new(private_key: SecretKey) -> Self {
            let address = Address::from(keccak(
                &private_key.public_key(SECP256K1).serialize_uncompressed()[1..],
            ));
            Self {
                private_key,
                address,
            }
        }

        pub fn sign(&self, data: Bytes) -> Signature {
            let hash = keccak(data);
            let msg = Message::from_digest(hash.0);
            let (recovery_id, signature) = SECP256K1
                .sign_ecdsa_recoverable(&msg, &self.private_key)
                .serialize_compact();

            Signature::from_slice(
                &[
                    signature.as_slice(),
                    &[Into::<i32>::into(recovery_id) as u8],
                ]
                .concat(),
            )
        }
    }

    #[tokio::test]
    #[cfg(feature = "rocksdb")]
    async fn test_basic_value_transfer() {
        //Create signer for some account
        let account_1 = Account::new(
            SecretKey::from_slice(
                hex::decode("4f3edf983ac636a6584e819f7aee7e2a3f6f0b8c0c6e8f1a5a4f5f35426f5d7e")
                    .unwrap()
                    .as_ref(),
            )
            .unwrap(),
        );
        let account_2 = Account::new(
            SecretKey::from_slice(
                hex::decode("6c3699283bda56ad74f6b855546325b68d482e983852a7a8297d1e4b7f2abf3c")
                    .unwrap()
                    .as_ref(),
            )
            .unwrap(),
        );

        // Create test accounts
        let account_a = account_1.address;
        let account_b = account_2.address;
        let initial_balance = U256::from(10_000_000_000_000_000_000u64); // 10 ETH

        // Create genesis with test accounts
        let mut genesis_accounts = BTreeMap::new();
        let account_state = GenesisAccount {
            balance: initial_balance,
            code: Bytes::new(),
            nonce: 0,
            storage: HashMap::new(),
        };
        genesis_accounts.insert(account_a, account_state.clone());
        genesis_accounts.insert(account_b, account_state);

        let genesis = Genesis {
            config: ethrex_common::types::ChainConfig::default(),
            alloc: genesis_accounts,
            ..Default::default()
        };

        // Create store with RocksDB
        let temp_dir = tempfile::TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test_db");
        let mut store = Store::new(&db_path, EngineType::RocksDB).expect("Failed to create store");
        store.add_initial_state(genesis).await.unwrap();

        // Create blockchain instance
        let blockchain = Blockchain::default_with_store(store.clone());

        // Get genesis block header
        let genesis_header = store.get_block_header(0).unwrap().unwrap();

        // Create transfer transaction
        let transfer_amount = U256::from(1_000_000_000_000_000_000u64); // 1 ETH
        let mut tx = ethrex_common::types::EIP1559Transaction {
            chain_id: 1,
            nonce: 0,
            max_priority_fee_per_gas: 20_000000,
            max_fee_per_gas: 20_000000,
            gas_limit: 210000,
            to: TxKind::Call(account_b),
            value: transfer_amount,
            data: Bytes::new(),
            ..Default::default()
        };

        let tx_outer = Transaction::EIP1559Transaction;

        let mut payload = vec![TxType::EIP1559 as u8];
        payload.append(tx.encode_payload_to_vec().as_mut());

        let signature = account_1.sign(payload.into());

        tx.signature_r = U256::from_big_endian(&signature[..32]);
        tx.signature_s = U256::from_big_endian(&signature[32..64]);
        tx.signature_y_parity = signature[64] != 0 && signature[64] != 27;

        // Create block with the transaction
        let block_1 = new_block(&store, &genesis_header).await;
        let hash_1 = block_1.hash();
        blockchain.add_block(block_1.clone()).unwrap();

        // We may be able to remove this forkchoice update, we only get by block hash.
        store
            .forkchoice_update(None, 1, hash_1, None, None)
            .await
            .unwrap();

        // Verify account balances
        let account_a_state = store
            .get_account_state_by_hash(hash_1, account_a)
            .unwrap()
            .unwrap();
        let account_b_state = store
            .get_account_state_by_hash(hash_1, account_b)
            .unwrap()
            .unwrap();

        assert_eq!(
            account_a_state.balance,
            U256::from(9_000_000_000_000_000_000u64),
            "Account A should have 9 ETH"
        );
        assert_eq!(
            account_b_state.balance,
            U256::from(11_000_000_000_000_000_000u64),
            "Account B should have 11 ETH"
        );
    }

}
