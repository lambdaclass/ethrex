#[cfg(test)]
mod blockchain_integration_test {
    use std::{fs::File, io::BufReader};

    use crate::{
        add_block,
        error::{ChainError, InvalidForkChoice},
        fork_choice::apply_fork_choice,
        is_canonical, latest_canonical_block_hash,
        payload::{build_payload, create_payload, BuildPayloadArgs},
    };

    use ethereum_rust_core::{
        types::{Block, BlockHeader},
        H160, H256,
    };
    use ethereum_rust_storage::{EngineType, Store};

    #[test]
    fn test_small_to_long_reorg() {
        // Store and genesis
        let store = test_store();
        let genesis_header = store.get_block_header(0).unwrap().unwrap();
        let genesis_hash = genesis_header.compute_block_hash();

        // Add first block. We'll make it canonical.
        let block_1a = new_block(&store, &genesis_header);
        let hash_1a = block_1a.header.compute_block_hash();
        add_block(&block_1a, &store).unwrap();
        store.set_canonical_block(1, hash_1a).unwrap();
        let retrieved_1a = store.get_block_header(1).unwrap().unwrap();

        assert_eq!(retrieved_1a, block_1a.header);
        assert!(is_canonical(&store, 1, hash_1a).unwrap());

        // Add second block at height 1. Will not be canonical.
        let block_1b = new_block(&store, &genesis_header);
        let hash_1b = block_1b.header.compute_block_hash();
        add_block(&block_1b, &store).expect("Could not add block 1b.");
        let retrieved_1b = store.get_block_header_by_hash(hash_1b).unwrap().unwrap();

        assert_ne!(retrieved_1a, retrieved_1b);
        assert!(!is_canonical(&store, 1, hash_1b).unwrap());

        // Add a third block at height 2, child to the non canonical block.
        let block_2 = new_block(&store, &block_1b.header);
        let hash_2 = block_2.header.compute_block_hash();
        add_block(&block_2, &store).expect("Could not add block 2.");
        let retrieved_2 = store.get_block_header_by_hash(hash_2).unwrap();

        assert!(retrieved_2.is_some());
        assert!(store.get_canonical_block_hash(2).unwrap().is_none());

        // Receive block 2 as new head.
        apply_fork_choice(
            &store,
            block_2.header.compute_block_hash(),
            genesis_header.compute_block_hash(),
            genesis_header.compute_block_hash(),
        )
        .unwrap();

        // Check that canonical blocks changed to the new branch.
        assert!(is_canonical(&store, 0, genesis_hash).unwrap());
        assert!(is_canonical(&store, 1, hash_1b).unwrap());
        assert!(is_canonical(&store, 2, hash_2).unwrap());
        assert!(!is_canonical(&store, 1, hash_1a).unwrap());
    }

    #[test]
    fn test_sync_not_supported_yet() {
        let store = test_store();
        let genesis_header = store.get_block_header(0).unwrap().unwrap();

        // Build a single valid block.
        let block_1 = new_block(&store, &genesis_header);
        let hash_1 = block_1.header.compute_block_hash();
        add_block(&block_1, &store).unwrap();
        apply_fork_choice(&store, hash_1, H256::zero(), H256::zero()).unwrap();

        // Build a child, then change its parent, making it effectively a pending block.
        let mut block_2 = new_block(&store, &block_1.header);
        block_2.header.parent_hash = H256::random();
        let hash_2 = block_2.header.compute_block_hash();
        let result = add_block(&block_2, &store);
        assert!(matches!(result, Err(ChainError::ParentNotFound)));

        // block 2 should now be pending.
        assert!(store.get_pending_block(hash_2).unwrap().is_some());

        let fc_result = apply_fork_choice(&store, hash_2, H256::zero(), H256::zero());
        assert!(matches!(fc_result, Err(InvalidForkChoice::Syncing)));

        // block 2 should still be pending.
        assert!(store.get_pending_block(hash_2).unwrap().is_some());
    }

    #[test]
    fn test_reorg_from_long_to_short_chain() {
        // Store and genesis
        let store = test_store();
        let genesis_header = store.get_block_header(0).unwrap().unwrap();
        let genesis_hash = genesis_header.compute_block_hash();

        // Add first block. Not canonical.
        let block_1a = new_block(&store, &genesis_header);
        let hash_1a = block_1a.header.compute_block_hash();
        add_block(&block_1a, &store).unwrap();
        let retrieved_1a = store.get_block_header_by_hash(hash_1a).unwrap().unwrap();

        assert!(!is_canonical(&store, 1, hash_1a).unwrap());

        // Add second block at height 1. Canonical.
        let block_1b = new_block(&store, &genesis_header);
        let hash_1b = block_1b.header.compute_block_hash();
        add_block(&block_1b, &store).expect("Could not add block 1b.");
        apply_fork_choice(&store, hash_1b, genesis_hash, genesis_hash).unwrap();
        let retrieved_1b = store.get_block_header(1).unwrap().unwrap();

        assert_ne!(retrieved_1a, retrieved_1b);
        assert_eq!(retrieved_1b, block_1b.header);
        assert!(is_canonical(&store, 1, hash_1b).unwrap());
        assert_eq!(latest_canonical_block_hash(&store).unwrap(), hash_1b);

        // Add a third block at height 2, child to the canonical one.
        let block_2 = new_block(&store, &block_1b.header);
        let hash_2 = block_2.header.compute_block_hash();
        add_block(&block_2, &store).expect("Could not add block 2.");
        apply_fork_choice(&store, hash_2, genesis_hash, genesis_hash).unwrap();
        let retrieved_2 = store.get_block_header_by_hash(hash_2).unwrap();
        assert_eq!(latest_canonical_block_hash(&store).unwrap(), hash_2);

        assert!(retrieved_2.is_some());
        assert!(is_canonical(&store, 2, hash_2).unwrap());
        assert_eq!(store.get_canonical_block_hash(2).unwrap().unwrap(), hash_2);

        // Receive block 1a as new head.
        apply_fork_choice(
            &store,
            block_1a.header.compute_block_hash(),
            genesis_header.compute_block_hash(),
            genesis_header.compute_block_hash(),
        )
        .unwrap();

        // Check that canonical blocks changed to the new branch.
        assert!(is_canonical(&store, 0, genesis_hash).unwrap());
        assert!(is_canonical(&store, 1, hash_1a).unwrap());
        assert!(!is_canonical(&store, 1, hash_1b).unwrap());
        assert!(!is_canonical(&store, 2, hash_2).unwrap());
    }

    #[test]
    fn new_head_with_canonical_ancestor_should_skip() {
        // Store and genesis
        let store = test_store();
        let genesis_header = store.get_block_header(0).unwrap().unwrap();
        let genesis_hash = genesis_header.compute_block_hash();

        // Add block at height 1.
        let block_1 = new_block(&store, &genesis_header);
        let hash_1 = block_1.header.compute_block_hash();
        add_block(&block_1, &store).expect("Could not add block 1b.");

        // Add child at height 2.
        let block_2 = new_block(&store, &block_1.header);
        let hash_2 = block_2.header.compute_block_hash();
        add_block(&block_2, &store).expect("Could not add block 2.");

        assert!(!is_canonical(&store, 1, hash_1).unwrap());
        assert!(!is_canonical(&store, 2, hash_2).unwrap());

        // Make that chain the canonical one.
        apply_fork_choice(&store, hash_2, genesis_hash, genesis_hash).unwrap();

        assert!(is_canonical(&store, 1, hash_1).unwrap());
        assert!(is_canonical(&store, 2, hash_2).unwrap());

        let result = apply_fork_choice(&store, hash_1, hash_1, hash_1);

        assert!(matches!(
            result,
            Err(InvalidForkChoice::NewHeadAlreadyCanonical)
        ));

        // Important blocks should still be the same as before.
        assert!(store.get_finalized_block_number().unwrap() == Some(0));
        assert!(store.get_safe_block_number().unwrap() == Some(0));
        assert!(store.get_latest_block_number().unwrap() == Some(2));
    }

    #[test]
    fn latest_block_number_should_always_be_the_canonical_head() {
        // Goal: put a, b in the same branch, both canonical.
        // Then add one in a different branch. Check that the last one is still the same.

        // Store and genesis
        let store = test_store();
        let genesis_header = store.get_block_header(0).unwrap().unwrap();
        let genesis_hash = genesis_header.compute_block_hash();

        // Add block at height 1.
        let block_1 = new_block(&store, &genesis_header);
        add_block(&block_1, &store).expect("Could not add block 1b.");

        // Add child at height 2.
        let block_2 = new_block(&store, &block_1.header);
        let hash_2 = block_2.header.compute_block_hash();
        add_block(&block_2, &store).expect("Could not add block 2.");

        assert_eq!(latest_canonical_block_hash(&store).unwrap(), genesis_hash);

        // Make that chain the canonical one.
        apply_fork_choice(&store, hash_2, genesis_hash, genesis_hash).unwrap();

        assert_eq!(latest_canonical_block_hash(&store).unwrap(), hash_2);

        // Add a new, non canonical block, starting from genesis.
        let block_1b = new_block(&store, &genesis_header);
        let hash_b = block_1b.header.compute_block_hash();
        add_block(&block_1b, &store).expect("Could not add block b.");

        // The latest block should be the same.
        assert_eq!(latest_canonical_block_hash(&store).unwrap(), hash_2);

        // if we apply fork choice to the new one, then we should
        apply_fork_choice(&store, hash_b, genesis_hash, genesis_hash).unwrap();

        // The latest block should now be the new head.
        assert_eq!(latest_canonical_block_hash(&store).unwrap(), hash_b);
    }

    fn new_block(store: &Store, parent: &BlockHeader) -> Block {
        let args = BuildPayloadArgs {
            parent: parent.compute_block_hash(),
            timestamp: parent.timestamp + 12,
            fee_recipient: H160::random(),
            random: H256::random(),
            withdrawals: Vec::new(),
            beacon_root: Some(H256::random()),
            version: 1,
        };

        let mut block = create_payload(&args, store).unwrap();
        build_payload(&mut block, store).unwrap();
        block
    }

    fn test_store() -> Store {
        // Get genesis
        let file = File::open("../../test_data/genesis-execution-api.json")
            .expect("Failed to open genesis file");
        let reader = BufReader::new(file);
        let genesis = serde_json::from_reader(reader).expect("Failed to deserialize genesis file");

        // Build store with genesis
        let store =
            Store::new("store.db", EngineType::InMemory).expect("Failed to build DB for testing");

        store
            .add_initial_state(genesis)
            .expect("Failed to add genesis state");

        store
    }
}
