#[cfg(test)]
mod blockchain_integration_test {
    use std::{fs::File, io::BufReader};

    use crate::{
        error::{ChainError, InvalidForkChoice},
        payload::{build_payload, create_payload, BuildPayloadArgs},
        BlockChain,
    };

    use ethrex_core::{
        types::{Block, BlockHeader},
        H160, H256,
    };
    use ethrex_storage::{EngineType, Store};
    use ethrex_vm::EVM;

    #[test]
    fn test_small_to_long_reorg() {
        // Chain and genesis
        let chain = blockchain();
        let genesis_header = chain.store.get_block_header(0).unwrap().unwrap();
        let genesis_hash = genesis_header.compute_block_hash();

        // Add first block. We'll make it canonical.
        let block_1a = new_block(&chain.store, &genesis_header);
        let hash_1a = block_1a.hash();
        chain.add_block(&block_1a).unwrap();
        chain.store.set_canonical_block(1, hash_1a).unwrap();
        let retrieved_1a = chain.store.get_block_header(1).unwrap().unwrap();

        assert_eq!(retrieved_1a, block_1a.header);
        assert!(chain.is_canonical(1, hash_1a).unwrap());

        // Add second block at height 1. Will not be canonical.
        let block_1b = new_block(&chain.store, &genesis_header);
        let hash_1b = block_1b.hash();
        chain.add_block(&block_1b).expect("Could not add block 1b.");
        let retrieved_1b = chain
            .store
            .get_block_header_by_hash(hash_1b)
            .unwrap()
            .unwrap();

        assert_ne!(retrieved_1a, retrieved_1b);
        assert!(!chain.is_canonical(1, hash_1b).unwrap());

        // Add a third block at height 2, child to the non canonical block.
        let block_2 = new_block(&chain.store, &block_1b.header);
        let hash_2 = block_2.hash();
        chain.add_block(&block_2).expect("Could not add block 2.");
        let retrieved_2 = chain.store.get_block_header_by_hash(hash_2).unwrap();

        assert!(retrieved_2.is_some());
        assert!(chain.store.get_canonical_block_hash(2).unwrap().is_none());

        // Receive block 2 as new head.
        chain
            .apply_fork_choice(
                block_2.hash(),
                genesis_header.compute_block_hash(),
                genesis_header.compute_block_hash(),
            )
            .unwrap();

        // Check that canonical blocks changed to the new branch.
        assert!(chain.is_canonical(0, genesis_hash).unwrap());
        assert!(chain.is_canonical(1, hash_1b).unwrap());
        assert!(chain.is_canonical(2, hash_2).unwrap());
        assert!(!chain.is_canonical(1, hash_1a).unwrap());
    }

    #[test]
    fn test_sync_not_supported_yet() {
        let chain = blockchain();
        let genesis_header = chain.store.get_block_header(0).unwrap().unwrap();

        // Build a single valid block.
        let block_1 = new_block(&chain.store, &genesis_header);
        let hash_1 = block_1.header.compute_block_hash();
        chain.add_block(&block_1).unwrap();
        chain
            .apply_fork_choice(hash_1, H256::zero(), H256::zero())
            .unwrap();

        // Build a child, then change its parent, making it effectively a pending block.
        let mut block_2 = new_block(&chain.store, &block_1.header);
        block_2.header.parent_hash = H256::random();
        let hash_2 = block_2.header.compute_block_hash();
        let result = chain.add_block(&block_2);
        assert!(matches!(result, Err(ChainError::ParentNotFound)));

        // block 2 should now be pending.
        assert!(chain.store.get_pending_block(hash_2).unwrap().is_some());

        let fc_result = chain.apply_fork_choice(hash_2, H256::zero(), H256::zero());
        assert!(matches!(fc_result, Err(InvalidForkChoice::Syncing)));

        // block 2 should still be pending.
        assert!(chain.store.get_pending_block(hash_2).unwrap().is_some());
    }

    #[test]
    fn test_reorg_from_long_to_short_chain() {
        // Chain and genesis
        let chain = blockchain();
        let genesis_header = chain.store.get_block_header(0).unwrap().unwrap();
        let genesis_hash = genesis_header.compute_block_hash();

        // Add first block. Not canonical.
        let block_1a = new_block(&chain.store, &genesis_header);
        let hash_1a = block_1a.hash();
        chain.add_block(&block_1a).unwrap();
        let retrieved_1a = chain
            .store
            .get_block_header_by_hash(hash_1a)
            .unwrap()
            .unwrap();

        assert!(!chain.is_canonical(1, hash_1a).unwrap());

        // Add second block at height 1. Canonical.
        let block_1b = new_block(&chain.store, &genesis_header);
        let hash_1b = block_1b.hash();
        chain.add_block(&block_1b).expect("Could not add block 1b.");
        chain
            .apply_fork_choice(hash_1b, genesis_hash, genesis_hash)
            .unwrap();
        let retrieved_1b = chain.store.get_block_header(1).unwrap().unwrap();

        assert_ne!(retrieved_1a, retrieved_1b);
        assert_eq!(retrieved_1b, block_1b.header);
        assert!(chain.is_canonical(1, hash_1b).unwrap());
        assert_eq!(chain.latest_canonical_block_hash().unwrap(), hash_1b);

        // Add a third block at height 2, child to the canonical one.
        let block_2 = new_block(&chain.store, &block_1b.header);
        let hash_2 = block_2.hash();
        chain.add_block(&block_2).expect("Could not add block 2.");
        chain
            .apply_fork_choice(hash_2, genesis_hash, genesis_hash)
            .unwrap();
        let retrieved_2 = chain.store.get_block_header_by_hash(hash_2).unwrap();
        assert_eq!(chain.latest_canonical_block_hash().unwrap(), hash_2);

        assert!(retrieved_2.is_some());
        assert!(chain.is_canonical(2, hash_2).unwrap());
        assert_eq!(
            chain.store.get_canonical_block_hash(2).unwrap().unwrap(),
            hash_2
        );

        // Receive block 1a as new head.
        chain
            .apply_fork_choice(
                block_1a.hash(),
                genesis_header.compute_block_hash(),
                genesis_header.compute_block_hash(),
            )
            .unwrap();

        // Check that canonical blocks changed to the new branch.
        assert!(chain.is_canonical(0, genesis_hash).unwrap());
        assert!(chain.is_canonical(1, hash_1a).unwrap());
        assert!(!chain.is_canonical(1, hash_1b).unwrap());
        assert!(!chain.is_canonical(2, hash_2).unwrap());
    }

    #[test]
    fn new_head_with_canonical_ancestor_should_skip() {
        // Chain and genesis
        let chain = blockchain();
        let genesis_header = chain.store.get_block_header(0).unwrap().unwrap();
        let genesis_hash = genesis_header.compute_block_hash();

        // Add block at height 1.
        let block_1 = new_block(&chain.store, &genesis_header);
        let hash_1 = block_1.hash();
        chain.add_block(&block_1).expect("Could not add block 1b.");

        // Add child at height 2.
        let block_2 = new_block(&chain.store, &block_1.header);
        let hash_2 = block_2.hash();
        chain.add_block(&block_2).expect("Could not add block 2.");

        assert!(!chain.is_canonical(1, hash_1).unwrap());
        assert!(!chain.is_canonical(2, hash_2).unwrap());

        // Make that chain the canonical one.
        chain
            .apply_fork_choice(hash_2, genesis_hash, genesis_hash)
            .unwrap();

        assert!(chain.is_canonical(1, hash_1).unwrap());
        assert!(chain.is_canonical(2, hash_2).unwrap());

        let result = chain.apply_fork_choice(hash_1, hash_1, hash_1);

        assert!(matches!(
            result,
            Err(InvalidForkChoice::NewHeadAlreadyCanonical)
        ));

        // Important blocks should still be the same as before.
        assert!(chain.store.get_finalized_block_number().unwrap() == Some(0));
        assert!(chain.store.get_safe_block_number().unwrap() == Some(0));
        assert!(chain.store.get_latest_block_number().unwrap() == 2);
    }

    #[test]
    fn latest_block_number_should_always_be_the_canonical_head() {
        // Goal: put a, b in the same branch, both canonical.
        // Then add one in a different branch. Check that the last one is still the same.

        // Chain and genesis
        let chain = blockchain();
        let genesis_header = chain.store.get_block_header(0).unwrap().unwrap();
        let genesis_hash = genesis_header.compute_block_hash();

        // Add block at height 1.
        let block_1 = new_block(&chain.store, &genesis_header);
        chain.add_block(&block_1).expect("Could not add block 1b.");

        // Add child at height 2.
        let block_2 = new_block(&chain.store, &block_1.header);
        let hash_2 = block_2.hash();
        chain.add_block(&block_2).expect("Could not add block 2.");

        assert_eq!(chain.latest_canonical_block_hash().unwrap(), genesis_hash);

        // Make that chain the canonical one.
        chain
            .apply_fork_choice(hash_2, genesis_hash, genesis_hash)
            .unwrap();

        assert_eq!(chain.latest_canonical_block_hash().unwrap(), hash_2);

        // Add a new, non canonical block, starting from genesis.
        let block_1b = new_block(&chain.store, &genesis_header);
        let hash_b = block_1b.hash();
        chain.add_block(&block_1b).expect("Could not add block b.");

        // The latest block should be the same.
        assert_eq!(chain.latest_canonical_block_hash().unwrap(), hash_2);

        // if we apply fork choice to the new one, then we should
        chain
            .apply_fork_choice(hash_b, genesis_hash, genesis_hash)
            .unwrap();

        // The latest block should now be the new head.
        assert_eq!(chain.latest_canonical_block_hash().unwrap(), hash_b);
    }

    fn new_block(store: &Store, parent: &BlockHeader) -> Block {
        let args = BuildPayloadArgs {
            parent: parent.compute_block_hash(),
            timestamp: parent.timestamp + 12,
            fee_recipient: H160::random(),
            random: H256::random(),
            withdrawals: Some(Vec::new()),
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
        let store = Store::new("chain.store.db", EngineType::InMemory)
            .expect("Failed to build DB for testing");

        store
            .add_initial_state(genesis)
            .expect("Failed to add genesis state");

        store
    }

    fn blockchain() -> BlockChain {
        let store = test_store();
        let evm = EVM::REVM;
        BlockChain::new(store, evm)
    }
}
