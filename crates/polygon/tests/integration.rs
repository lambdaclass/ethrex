//! Integration tests for the Polygon block processing pipeline.
//!
//! These tests exercise the validation, snapshot, sprint/span detection,
//! difficulty calculation, system call encoding, and fork choice logic
//! in combination — without requiring real secp256k1 signatures.

use std::cmp::Ordering;

use bytes::Bytes;
use ethereum_types::{Address, H256, U256};

use ethrex_common::constants::DEFAULT_OMMERS_HASH;
use ethrex_common::types::BlockHeader;
use ethrex_polygon::bor_config::BorConfig;
use ethrex_polygon::consensus::engine::BorEngine;
use ethrex_polygon::consensus::snapshot::{Snapshot, ValidatorInfo};
use ethrex_polygon::genesis::{
    amoy_bor_config, amoy_genesis, polygon_genesis_block, polygon_mainnet_bor_config,
    polygon_mainnet_genesis,
};
use ethrex_polygon::system_calls;
use ethrex_polygon::validation::validate_bor_header;

// ====================================================================
// Helpers
// ====================================================================

/// Build a minimal BorConfig with sprint=16 and all early forks active.
fn test_config() -> BorConfig {
    serde_json::from_str(
        r#"{
            "period": {"0": 2},
            "producerDelay": {"0": 6},
            "sprint": {"0": 16},
            "backupMultiplier": {"0": 2},
            "validatorContract": "0x0000000000000000000000000000000000001000",
            "stateReceiverContract": "0x0000000000000000000000000000000000001001",
            "jaipurBlock": 0,
            "delhiBlock": 0,
            "indoreBlock": 0
        }"#,
    )
    .expect("valid test config")
}

fn make_validator(addr_byte: u8, power: u64) -> ValidatorInfo {
    ValidatorInfo {
        address: Address::from_low_u64_be(addr_byte as u64),
        voting_power: power,
        proposer_priority: 0,
    }
}

fn make_validator_with_priority(addr_byte: u8, power: u64, priority: i64) -> ValidatorInfo {
    ValidatorInfo {
        address: Address::from_low_u64_be(addr_byte as u64),
        voting_power: power,
        proposer_priority: priority,
    }
}

/// Create a valid Bor-style parent header at a given block number.
fn make_parent_at(number: u64, timestamp: u64) -> BlockHeader {
    BlockHeader {
        number,
        timestamp,
        difficulty: U256::from(1),
        coinbase: Address::zero(),
        ommers_hash: *DEFAULT_OMMERS_HASH,
        gas_limit: 30_000_000,
        gas_used: 1_000_000,
        base_fee_per_gas: Some(7),
        extra_data: Bytes::from(vec![0u8; 97]),
        ..Default::default()
    }
}

/// Create a valid Bor-style child header referencing the given parent.
fn make_child_of(parent: &BlockHeader) -> BlockHeader {
    BlockHeader {
        parent_hash: parent.hash(),
        number: parent.number + 1,
        timestamp: parent.timestamp + 2,
        difficulty: U256::from(1),
        coinbase: Address::zero(),
        ommers_hash: *DEFAULT_OMMERS_HASH,
        gas_limit: 30_000_000,
        gas_used: 500_000,
        base_fee_per_gas: Some(7),
        extra_data: Bytes::from(vec![0u8; 97]),
        ..Default::default()
    }
}

// ====================================================================
// 1. validate_bor_header: chain of headers
// ====================================================================

#[test]
fn validate_chain_of_three_blocks() {
    let parent = make_parent_at(100, 1000);
    let child1 = make_child_of(&parent);
    assert!(validate_bor_header(&child1, &parent).is_ok());

    let child2 = make_child_of(&child1);
    assert!(validate_bor_header(&child2, &child1).is_ok());
}

#[test]
fn validate_rejects_skipped_block_number() {
    let parent = make_parent_at(100, 1000);
    let mut child = make_child_of(&parent);
    child.number = parent.number + 2; // Skip a block
    assert!(validate_bor_header(&child, &parent).is_err());
}

#[test]
fn validate_rejects_non_increasing_timestamp() {
    let parent = make_parent_at(100, 1000);
    let mut child = make_child_of(&parent);
    child.timestamp = parent.timestamp; // Not strictly greater
    assert!(validate_bor_header(&child, &parent).is_err());
}

#[test]
fn validate_rejects_post_merge_fields() {
    let parent = make_parent_at(100, 1000);

    // withdrawals_root
    let mut child = make_child_of(&parent);
    child.withdrawals_root = Some(H256::zero());
    assert!(validate_bor_header(&child, &parent).is_err());

    // blob_gas_used
    let mut child = make_child_of(&parent);
    child.blob_gas_used = Some(0);
    assert!(validate_bor_header(&child, &parent).is_err());

    // excess_blob_gas
    let mut child = make_child_of(&parent);
    child.excess_blob_gas = Some(0);
    assert!(validate_bor_header(&child, &parent).is_err());

    // parent_beacon_block_root
    let mut child = make_child_of(&parent);
    child.parent_beacon_block_root = Some(H256::zero());
    assert!(validate_bor_header(&child, &parent).is_err());

    // requests_hash
    let mut child = make_child_of(&parent);
    child.requests_hash = Some(H256::zero());
    assert!(validate_bor_header(&child, &parent).is_err());
}

#[test]
fn validate_accepts_various_difficulties() {
    let parent = make_parent_at(100, 1000);

    for diff in [1u64, 2, 5, 21] {
        let mut child = make_child_of(&parent);
        child.difficulty = U256::from(diff);
        assert!(
            validate_bor_header(&child, &parent).is_ok(),
            "difficulty {diff} should be accepted"
        );
    }
}

#[test]
fn validate_rejects_zero_difficulty() {
    let parent = make_parent_at(100, 1000);
    let mut child = make_child_of(&parent);
    child.difficulty = U256::zero();
    assert!(validate_bor_header(&child, &parent).is_err());
}

// ====================================================================
// 2. Sprint/span boundary detection via BorConfig
// ====================================================================

#[test]
fn sprint_boundary_detection_across_blocks() {
    let config = test_config(); // sprint=16

    // Sprint starts: 0, 16, 32, 48, ...
    let sprint_starts: Vec<u64> = (0..=64).filter(|b| config.is_sprint_start(*b)).collect();
    assert_eq!(sprint_starts, vec![0, 16, 32, 48, 64]);

    // Sprint ends: 15, 31, 47, 63, ...
    let sprint_ends: Vec<u64> = (0..=64).filter(|b| config.is_sprint_end(*b)).collect();
    assert_eq!(sprint_ends, vec![15, 31, 47, 63]);
}

#[test]
fn sprint_size_changes_at_fork_boundary() {
    // Mainnet: sprint changes from 64 to 16 at Delhi (38189056)
    let config = polygon_mainnet_bor_config();

    assert_eq!(config.get_sprint_size(0), 64);
    assert_eq!(config.get_sprint_size(38_189_055), 64);
    assert_eq!(config.get_sprint_size(38_189_056), 16);
    assert_eq!(config.get_sprint_size(50_000_000), 16);
}

#[test]
fn span_id_progression() {
    let config = test_config();

    // Span 0: blocks 0..=255
    assert_eq!(config.span_id_at(0), 0);
    assert_eq!(config.span_id_at(255), 0);

    // Span 1: blocks 256..=6655
    assert_eq!(config.span_id_at(256), 1);
    assert_eq!(config.span_id_at(6655), 1);

    // Span 2: blocks 6656..=13055
    assert_eq!(config.span_id_at(6656), 2);
}

#[test]
fn need_to_commit_span_at_boundary() {
    let config = polygon_mainnet_bor_config();

    // Block 0 always excluded
    assert!(!config.need_to_commit_span(0));

    // Block 192: sprint start, sprint 192..255, span 0 ends at 255
    // Next block after sprint (256) is span 1 → need commit
    assert!(config.need_to_commit_span(192));

    // Block 128: sprint start but no span crossing
    assert!(!config.need_to_commit_span(128));

    // Block 63: sprint end, not a sprint start
    assert!(!config.need_to_commit_span(63));
}

#[test]
fn amoy_sprint_always_16() {
    let config = amoy_bor_config();
    // Amoy has sprint=16 from block 0
    for block in [0, 1, 100, 1_000_000, 30_000_000] {
        assert_eq!(config.get_sprint_size(block), 16, "block {block}");
    }
}

// ====================================================================
// 3. Difficulty calculation via Snapshot::expected_difficulty
// ====================================================================

#[test]
fn difficulty_calculation_with_three_validators() {
    // Validators: [A(prio=0), B(prio=100), C(prio=50)]
    // Proposer is B (highest priority)
    let validators = vec![
        make_validator_with_priority(1, 10, 0),   // A, idx 0
        make_validator_with_priority(2, 10, 100), // B, idx 1 (proposer)
        make_validator_with_priority(3, 10, 50),  // C, idx 2
    ];
    let snap = Snapshot::new(0, H256::zero(), validators);

    let addr_a = Address::from_low_u64_be(1);
    let addr_b = Address::from_low_u64_be(2);
    let addr_c = Address::from_low_u64_be(3);

    // B is proposer: succession=0, difficulty = 3-0 = 3
    assert_eq!(snap.expected_difficulty(&addr_b), Some(3));
    // C: succession=1, difficulty = 3-1 = 2
    assert_eq!(snap.expected_difficulty(&addr_c), Some(2));
    // A: succession=2, difficulty = 3-2 = 1
    assert_eq!(snap.expected_difficulty(&addr_a), Some(1));
}

#[test]
fn difficulty_calculation_single_validator() {
    let validators = vec![make_validator(1, 10)];
    let snap = Snapshot::new(0, H256::zero(), validators);
    let addr = Address::from_low_u64_be(1);
    assert_eq!(snap.expected_difficulty(&addr), Some(1));
}

#[test]
fn difficulty_unknown_signer_returns_none() {
    let validators = vec![make_validator(1, 10), make_validator(2, 20)];
    let snap = Snapshot::new(0, H256::zero(), validators);
    let unknown = Address::from_low_u64_be(99);
    assert_eq!(snap.expected_difficulty(&unknown), None);
}

#[test]
fn snapshot_tracks_validator_updates() {
    let mut snap = Snapshot::new(0, H256::zero(), vec![make_validator(1, 10)]);
    assert_eq!(snap.validator_set.len(), 1);

    // Update the validator set (simulating span change)
    snap.update_validator_set(vec![
        make_validator(2, 20),
        make_validator(3, 30),
        make_validator(4, 40),
    ]);
    assert_eq!(snap.validator_set.len(), 3);
    assert_eq!(snap.validator_set[0].address, Address::from_low_u64_be(2));

    // Difficulty should now be based on the new validator set
    let addr2 = Address::from_low_u64_be(2);
    assert!(snap.expected_difficulty(&addr2).is_some());
    // Old validator should not be found
    let addr1 = Address::from_low_u64_be(1);
    assert_eq!(snap.expected_difficulty(&addr1), None);
}

// ====================================================================
// 4. System call encoding
// ====================================================================

#[test]
fn commit_span_encoding_roundtrip() {
    let data = system_calls::encode_commit_span(10, 256, 6655, &[0x01; 40], &[0x02; 40]);

    // Verify selector
    assert_eq!(&data[..4], &hex::decode("23c2a2b4").unwrap());

    // Verify span ID = 10
    assert_eq!(data[4 + 31], 10);
    // Verify start_block = 256
    assert_eq!(data[4 + 32 + 30], 1); // 256 = 0x0100
    assert_eq!(data[4 + 32 + 31], 0);
    // Verify end_block = 6655 = 0x19FF
    assert_eq!(data[4 + 64 + 30], 0x19);
    assert_eq!(data[4 + 64 + 31], 0xFF);
}

#[test]
fn commit_state_encoding_with_real_data() {
    let record = vec![0xAB; 100]; // Simulated state sync record
    let data = system_calls::encode_commit_state(1_700_000_000, &record);

    // Selector for commitState(uint256,bytes)
    assert_eq!(&data[..4], &hex::decode("19494a17").unwrap());

    // sync_time (big-endian u256)
    let time_bytes = &data[4..36];
    let mut expected = [0u8; 32];
    expected[24..].copy_from_slice(&1_700_000_000u64.to_be_bytes());
    assert_eq!(time_bytes, &expected);

    // offset to bytes data = 64
    let offset_bytes = &data[36..68];
    let mut expected_offset = [0u8; 32];
    expected_offset[31] = 64;
    assert_eq!(offset_bytes, &expected_offset);

    // bytes length = 100
    let len_bytes = &data[68..100];
    let mut expected_len = [0u8; 32];
    expected_len[31] = 100;
    assert_eq!(len_bytes, &expected_len);
}

#[test]
fn last_state_id_selector() {
    let data = system_calls::encode_last_state_id();
    assert_eq!(data, hex::decode("5407ca67").unwrap());
}

#[test]
fn get_current_span_selector() {
    let data = system_calls::encode_get_current_span();
    assert_eq!(data, hex::decode("af26aa96").unwrap());
}

// ====================================================================
// 5. Fork choice: compare_td
// ====================================================================

#[test]
fn fork_choice_higher_td_wins() {
    let hash = H256::zero();
    assert_eq!(
        BorEngine::compare_td(&U256::from(200), 10, &hash, &U256::from(100), 10, &hash),
        Ordering::Greater
    );
}

#[test]
fn fork_choice_equal_td_higher_number_wins() {
    let td = U256::from(100);
    let hash = H256::zero();
    assert_eq!(
        BorEngine::compare_td(&td, 20, &hash, &td, 10, &hash),
        Ordering::Greater
    );
}

#[test]
fn fork_choice_equal_td_equal_number_lower_hash_wins() {
    let td = U256::from(100);
    let hash_high = H256::from_low_u64_be(0xFF);
    let hash_low = H256::from_low_u64_be(0x01);
    // hash_low < hash_high, so the chain with hash_low wins
    assert_eq!(
        BorEngine::compare_td(&td, 10, &hash_low, &td, 10, &hash_high),
        Ordering::Greater
    );
    assert_eq!(
        BorEngine::compare_td(&td, 10, &hash_high, &td, 10, &hash_low),
        Ordering::Less
    );
}

#[test]
fn fork_choice_competing_chains_simulation() {
    // Simulate two competing chain tips:
    // Chain A: 100 blocks, each difficulty 3 (3 validators, all in-turn) = TD 300
    // Chain B: 100 blocks, mixed difficulties = TD 250
    let td_a = U256::from(300);
    let td_b = U256::from(250);
    let hash_a = H256::from_low_u64_be(0xA);
    let hash_b = H256::from_low_u64_be(0xB);

    // Chain A wins on TD
    assert_eq!(
        BorEngine::compare_td(&td_a, 100, &hash_a, &td_b, 100, &hash_b),
        Ordering::Greater
    );

    // If TD was equal, Chain A (lower hash) wins
    assert_eq!(
        BorEngine::compare_td(&td_b, 100, &hash_a, &td_b, 100, &hash_b),
        Ordering::Greater
    );
}

// ====================================================================
// 6. Genesis block construction
// ====================================================================

#[test]
fn polygon_mainnet_genesis_block_has_no_post_merge_fields() {
    let genesis = polygon_mainnet_genesis();
    let block = polygon_genesis_block(&genesis);
    let header = &block.header;

    assert_eq!(header.number, 0);
    assert_eq!(header.difficulty, U256::from(1));
    assert_eq!(header.coinbase, Address::zero());
    assert_eq!(header.nonce, 0);
    assert!(header.withdrawals_root.is_none());
    assert!(header.blob_gas_used.is_none());
    assert!(header.excess_blob_gas.is_none());
    assert!(header.parent_beacon_block_root.is_none());
    assert!(header.requests_hash.is_none());
}

#[test]
fn amoy_genesis_block_has_no_post_merge_fields() {
    let genesis = amoy_genesis();
    let block = polygon_genesis_block(&genesis);
    let header = &block.header;

    assert_eq!(header.number, 0);
    assert_eq!(header.difficulty, U256::from(1));
    assert!(header.withdrawals_root.is_none());
    assert!(header.blob_gas_used.is_none());
    assert!(header.excess_blob_gas.is_none());
    assert!(header.parent_beacon_block_root.is_none());
    assert!(header.requests_hash.is_none());
}

#[test]
fn genesis_block_validate_as_parent_for_block_1() {
    let genesis = polygon_mainnet_genesis();
    let genesis_block = polygon_genesis_block(&genesis);
    let genesis_header = &genesis_block.header;

    // Build a mock block 1 referencing genesis as parent
    let block1 = BlockHeader {
        parent_hash: genesis_header.hash(),
        number: 1,
        timestamp: genesis_header.timestamp + 2,
        difficulty: U256::from(1),
        coinbase: Address::zero(),
        ommers_hash: *DEFAULT_OMMERS_HASH,
        gas_limit: 10_000_000,
        gas_used: 0,
        base_fee_per_gas: None, // London not active at block 1 on mainnet
        extra_data: Bytes::from(vec![0u8; 97]),
        ..Default::default()
    };

    // Structural validation should pass
    assert!(validate_bor_header(&block1, genesis_header).is_ok());
}

// ====================================================================
// 7. End-to-end pipeline: genesis → block 1 → ... → sprint boundary
// ====================================================================

#[test]
fn pipeline_genesis_through_sprint_boundary() {
    let config = test_config(); // sprint=16

    // Start from a genesis-like parent at block 0
    let mut parent = make_parent_at(0, 1_700_000_000);

    // Build a chain up to block 17 (past first sprint boundary at 16)
    for block_num in 1..=17 {
        let child = make_child_of(&parent);
        assert_eq!(child.number, block_num);

        // Validate structural checks
        assert!(
            validate_bor_header(&child, &parent).is_ok(),
            "block {block_num} should pass validation"
        );

        // Check sprint boundary
        if config.is_sprint_start(block_num) {
            assert!(
                block_num % 16 == 0,
                "sprint start should be at multiples of 16, got {block_num}"
            );
        }

        parent = child;
    }

    // Block 16 should have been a sprint start
    assert!(config.is_sprint_start(16));
    // Block 15 should have been a sprint end
    assert!(config.is_sprint_end(15));
}

#[test]
fn pipeline_difficulty_across_validator_rotation() {
    // Simulate: 3 validators, proposer changes after snapshot update
    let addr_a = Address::from_low_u64_be(1);
    let addr_b = Address::from_low_u64_be(2);
    let addr_c = Address::from_low_u64_be(3);

    let validators_v1 = vec![
        make_validator_with_priority(1, 10, 0),
        make_validator_with_priority(2, 10, 100), // proposer
        make_validator_with_priority(3, 10, 50),
    ];

    let mut snap = Snapshot::new(0, H256::zero(), validators_v1);

    // Phase 1: B is proposer
    assert_eq!(snap.expected_difficulty(&addr_b), Some(3)); // in-turn
    assert_eq!(snap.expected_difficulty(&addr_c), Some(2));
    assert_eq!(snap.expected_difficulty(&addr_a), Some(1)); // farthest

    // Simulate span change: new validator set where C has highest priority
    let validators_v2 = vec![
        make_validator_with_priority(1, 10, 0),
        make_validator_with_priority(2, 10, 50),
        make_validator_with_priority(3, 10, 200), // now proposer
    ];
    snap.update_validator_set(validators_v2);

    // Phase 2: C is now proposer
    assert_eq!(snap.expected_difficulty(&addr_c), Some(3)); // in-turn
    assert_eq!(snap.expected_difficulty(&addr_a), Some(2));
    assert_eq!(snap.expected_difficulty(&addr_b), Some(1)); // farthest
}

// ====================================================================
// 8. Milestone reorg protection
// ====================================================================

#[test]
fn milestone_reorg_protection_integration() {
    use ethrex_polygon::heimdall::Milestone;

    let engine = BorEngine::new(test_config(), "http://localhost:1317");

    // No milestone: all reorgs allowed
    assert!(engine.is_reorg_allowed(0));
    assert!(engine.check_reorg_allowed(0).is_ok());

    // Set milestone covering blocks 50..100
    engine.set_milestone(Milestone {
        id: 1,
        start_block: 50,
        end_block: 100,
        hash: H256::from_low_u64_be(0x1),
    });

    // Can't revert to block 50 (within milestone)
    assert!(!engine.is_reorg_allowed(50));
    assert!(engine.check_reorg_allowed(50).is_err());

    // Can't revert to block 100 (boundary, strictly greater required)
    assert!(!engine.is_reorg_allowed(100));

    // Can revert to block 101 (past milestone)
    assert!(engine.is_reorg_allowed(101));
    assert!(engine.check_reorg_allowed(101).is_ok());

    // Update milestone to cover more blocks
    engine.set_milestone(Milestone {
        id: 2,
        start_block: 101,
        end_block: 200,
        hash: H256::from_low_u64_be(0x2),
    });

    // Now can't revert to 150
    assert!(!engine.is_reorg_allowed(150));
    // But can revert to 201
    assert!(engine.is_reorg_allowed(201));
}

// ====================================================================
// 9. Snapshot cache integration
// ====================================================================

#[test]
fn snapshot_cache_stores_and_retrieves() {
    use ethrex_polygon::consensus::snapshot::SnapshotCache;

    let cache = SnapshotCache::new();

    // Insert snapshots for multiple blocks
    for i in 0..10 {
        let hash = H256::from_low_u64_be(i);
        let snap = Snapshot::new(i, hash, vec![make_validator(1, 10)]);
        cache.insert(snap);
    }

    // All should be retrievable
    for i in 0..10 {
        let hash = H256::from_low_u64_be(i);
        let snap = cache.get(&hash).expect("snapshot should be cached");
        assert_eq!(snap.number, i);
    }
}

// ====================================================================
// 10. BorConfig fork activation
// ====================================================================

#[test]
fn polygon_fork_schedule_mainnet() {
    use ethrex_common::types::Fork;

    let config = polygon_mainnet_bor_config();

    // Pre-Jaipur returns Prague (all EVM forks active)
    assert_eq!(config.get_polygon_fork(0), Fork::Prague);

    // Exact fork boundaries
    assert_eq!(config.get_polygon_fork(23_850_000), Fork::Jaipur);
    assert_eq!(config.get_polygon_fork(38_189_056), Fork::Delhi);
    assert_eq!(config.get_polygon_fork(44_934_656), Fork::Indore);
    assert_eq!(config.get_polygon_fork(62_278_656), Fork::Ahmedabad);
    assert_eq!(config.get_polygon_fork(73_440_256), Fork::Bhilai);
    assert_eq!(config.get_polygon_fork(77_414_656), Fork::Rio);

    // Madhugiri and MadhugiriPro at same block → MadhugiriPro wins
    assert_eq!(config.get_polygon_fork(80_084_800), Fork::MadhugiriPro);
    assert_eq!(config.get_polygon_fork(81_424_000), Fork::Dandeli);

    // Lisovo and LisovoPro at same block → LisovoPro wins
    assert_eq!(config.get_polygon_fork(83_756_500), Fork::LisovoPro);
}

#[test]
fn amoy_fork_schedule() {
    use ethrex_common::types::Fork;

    let config = amoy_bor_config();

    // Amoy: Jaipur/Delhi/Indore all at block 73100
    assert_eq!(config.get_polygon_fork(0), Fork::Prague); // pre-Jaipur
    assert_eq!(config.get_polygon_fork(73_100), Fork::Indore); // Jaipur+Delhi+Indore at same block

    // Check a few Amoy-specific boundaries
    assert_eq!(config.get_polygon_fork(11_865_856), Fork::Ahmedabad);
    assert_eq!(config.get_polygon_fork(22_765_056), Fork::Bhilai);
    assert_eq!(config.get_polygon_fork(26_272_256), Fork::Rio);
}
