//! EIP-7928: Block Access List Tests
//!
//! Tests for Block Access List (BAL) recording, hash computation,
//! checkpoint/restore semantics, net-zero filtering, and RLP encoding.

use ethereum_types::H160;
use ethrex_common::U256;
use ethrex_common::constants::SYSTEM_ADDRESS;
use ethrex_common::types::block_access_list::{
    AccountChanges, BalanceChange, BlockAccessList, BlockAccessListRecorder, CodeChange,
    NonceChange, SlotChange, StorageChange,
};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};

// Test addresses (matching those in block_access_list.rs for RLP compatibility)
const ALICE: H160 = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x01,
]);
const BOB: H160 = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x02,
]);
const CHARLIE: H160 = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x03,
]);

// Additional test addresses for RLP encoding tests (matching block_access_list.rs)
const ALICE_ADDR: H160 = H160([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 10]); // 0xA
const BOB_ADDR: H160 = H160([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 11]); // 0xB
const CHARLIE_ADDR: H160 = H160([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 12]); // 0xC
const CONTRACT_ADDR: H160 = H160([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 12]); // 0xC

// ==================== BAL Hash Computation Tests ====================

#[test]
fn test_empty_bal_hash() {
    // Empty BAL should have the well-known empty hash
    let bal = BlockAccessList::new();
    let hash = bal.compute_hash();

    // The empty BAL hash is keccak256(RLP([])) = 0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347
    let expected = ethrex_common::H256::from_slice(
        &hex::decode("1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347").unwrap(),
    );
    assert_eq!(hash, expected);
}

#[test]
fn test_bal_hash_deterministic() {
    // Same BAL content should always produce the same hash
    let mut recorder1 = BlockAccessListRecorder::new();
    recorder1.set_block_access_index(1);
    recorder1.record_touched_address(ALICE);
    recorder1.record_storage_write(ALICE, U256::from(0x10), U256::from(0x42));

    let mut recorder2 = BlockAccessListRecorder::new();
    recorder2.set_block_access_index(1);
    recorder2.record_touched_address(ALICE);
    recorder2.record_storage_write(ALICE, U256::from(0x10), U256::from(0x42));

    let bal1 = recorder1.build();
    let bal2 = recorder2.build();

    assert_eq!(bal1.compute_hash(), bal2.compute_hash());
}

#[test]
fn test_bal_hash_changes_with_content() {
    // Different BAL content should produce different hashes
    let mut recorder1 = BlockAccessListRecorder::new();
    recorder1.set_block_access_index(1);
    recorder1.record_touched_address(ALICE);
    recorder1.record_storage_write(ALICE, U256::from(0x10), U256::from(0x42));

    let mut recorder2 = BlockAccessListRecorder::new();
    recorder2.set_block_access_index(1);
    recorder2.record_touched_address(ALICE);
    recorder2.record_storage_write(ALICE, U256::from(0x10), U256::from(0x43)); // Different value

    let bal1 = recorder1.build();
    let bal2 = recorder2.build();

    assert_ne!(bal1.compute_hash(), bal2.compute_hash());
}

#[test]
fn test_bal_hash_sorted_encoding() {
    // BAL hash should be the same regardless of insertion order
    // because the encoding sorts addresses and slots
    let mut recorder1 = BlockAccessListRecorder::new();
    recorder1.set_block_access_index(1);
    recorder1.record_touched_address(ALICE);
    recorder1.record_touched_address(BOB);
    recorder1.record_touched_address(CHARLIE);

    let mut recorder2 = BlockAccessListRecorder::new();
    recorder2.set_block_access_index(1);
    recorder2.record_touched_address(CHARLIE);
    recorder2.record_touched_address(ALICE);
    recorder2.record_touched_address(BOB);

    let bal1 = recorder1.build();
    let bal2 = recorder2.build();

    assert_eq!(bal1.compute_hash(), bal2.compute_hash());
}

// ==================== Net-Zero Storage Filtering Tests ====================

#[test]
fn test_storage_net_zero_filtered_within_tx() {
    // Per EIP-7928: "If a storage slot's value is changed but its post-transaction value
    // equals its pre-transaction value, then the change MUST NOT be recorded."
    let mut recorder = BlockAccessListRecorder::new();
    recorder.set_block_access_index(1);

    // Capture pre-storage value
    recorder.capture_pre_storage(ALICE, U256::from(0x10), U256::from(100));

    // Write a different value
    recorder.record_storage_write(ALICE, U256::from(0x10), U256::from(200));

    // Write back the original value
    recorder.record_storage_write(ALICE, U256::from(0x10), U256::from(100));

    // build() triggers net-zero filtering for the current transaction
    let bal = recorder.build();

    // The storage change should be filtered out as it's net-zero
    let account = &bal.accounts()[0];
    assert!(
        account.storage_changes.is_empty(),
        "Net-zero storage changes should be filtered"
    );
}

#[test]
fn test_storage_net_zero_with_intermediate_write() {
    // Even with multiple intermediate writes, net-zero should be filtered
    let mut recorder = BlockAccessListRecorder::new();
    recorder.set_block_access_index(1);

    recorder.capture_pre_storage(ALICE, U256::from(0x10), U256::from(0));

    // Multiple writes
    recorder.record_storage_write(ALICE, U256::from(0x10), U256::from(100));
    recorder.record_storage_write(ALICE, U256::from(0x10), U256::from(200));
    recorder.record_storage_write(ALICE, U256::from(0x10), U256::from(300));
    // Back to original
    recorder.record_storage_write(ALICE, U256::from(0x10), U256::from(0));

    // build() triggers net-zero filtering
    let bal = recorder.build();

    let account = &bal.accounts()[0];
    assert!(
        account.storage_changes.is_empty(),
        "Net-zero storage should be filtered even with intermediate writes"
    );
}

#[test]
fn test_storage_non_zero_change_recorded() {
    // Non-zero changes should be recorded
    let mut recorder = BlockAccessListRecorder::new();
    recorder.set_block_access_index(1);

    recorder.capture_pre_storage(ALICE, U256::from(0x10), U256::from(100));
    recorder.record_storage_write(ALICE, U256::from(0x10), U256::from(200));

    // build() triggers net-zero filtering but this is NOT net-zero
    let bal = recorder.build();

    let account = &bal.accounts()[0];
    assert_eq!(account.storage_changes.len(), 1);
    assert_eq!(account.storage_changes[0].slot, U256::from(0x10));
}

// ==================== Checkpoint/Restore Tests ====================

#[test]
fn test_checkpoint_restore_storage_writes() {
    let mut recorder = BlockAccessListRecorder::new();
    recorder.set_block_access_index(1);

    // Write before checkpoint
    recorder.record_storage_write(ALICE, U256::from(0x10), U256::from(0x01));

    let checkpoint = recorder.checkpoint();

    // Write after checkpoint (will be reverted)
    recorder.record_storage_write(ALICE, U256::from(0x20), U256::from(0x02));
    recorder.record_storage_write(BOB, U256::from(0x30), U256::from(0x03));

    recorder.restore(checkpoint);

    let bal = recorder.build();

    // Only ALICE with slot 0x10 should have a write
    // BOB should still appear as touched but with no changes
    let alice = bal.accounts().iter().find(|a| a.address == ALICE).unwrap();
    assert_eq!(alice.storage_changes.len(), 1);
    assert_eq!(alice.storage_changes[0].slot, U256::from(0x10));

    let bob = bal.accounts().iter().find(|a| a.address == BOB).unwrap();
    assert!(bob.storage_changes.is_empty());
}

#[test]
fn test_checkpoint_restore_balance_changes() {
    let mut recorder = BlockAccessListRecorder::new();
    recorder.set_block_access_index(1);

    recorder.set_initial_balance(ALICE, U256::from(1000));
    recorder.record_balance_change(ALICE, U256::from(900));

    let checkpoint = recorder.checkpoint();

    recorder.set_initial_balance(BOB, U256::from(500));
    recorder.record_balance_change(BOB, U256::from(600));

    recorder.restore(checkpoint);

    let bal = recorder.build();

    let alice = bal.accounts().iter().find(|a| a.address == ALICE).unwrap();
    assert_eq!(alice.balance_changes.len(), 1);

    let bob = bal.accounts().iter().find(|a| a.address == BOB).unwrap();
    assert!(bob.balance_changes.is_empty());
}

#[test]
fn test_checkpoint_restore_nonce_changes() {
    let mut recorder = BlockAccessListRecorder::new();
    recorder.set_block_access_index(1);

    recorder.record_nonce_change(ALICE, 1);

    let checkpoint = recorder.checkpoint();

    recorder.record_nonce_change(BOB, 1);

    recorder.restore(checkpoint);

    let bal = recorder.build();

    let alice = bal.accounts().iter().find(|a| a.address == ALICE).unwrap();
    assert_eq!(alice.nonce_changes.len(), 1);

    let bob = bal.accounts().iter().find(|a| a.address == BOB).unwrap();
    assert!(bob.nonce_changes.is_empty());
}

#[test]
fn test_checkpoint_restore_code_changes() {
    let mut recorder = BlockAccessListRecorder::new();
    recorder.set_block_access_index(1);

    recorder.record_code_change(ALICE, bytes::Bytes::from_static(&[0x60, 0x00]));

    let checkpoint = recorder.checkpoint();

    recorder.record_code_change(BOB, bytes::Bytes::from_static(&[0x60, 0x01]));

    recorder.restore(checkpoint);

    let bal = recorder.build();

    let alice = bal.accounts().iter().find(|a| a.address == ALICE).unwrap();
    assert_eq!(alice.code_changes.len(), 1);

    let bob = bal.accounts().iter().find(|a| a.address == BOB).unwrap();
    assert!(bob.code_changes.is_empty());
}

#[test]
fn test_nested_checkpoints() {
    let mut recorder = BlockAccessListRecorder::new();
    recorder.set_block_access_index(1);

    recorder.record_storage_write(ALICE, U256::from(0x10), U256::from(0x01));

    let _cp1 = recorder.checkpoint();

    recorder.record_storage_write(ALICE, U256::from(0x20), U256::from(0x02));

    let cp2 = recorder.checkpoint();

    recorder.record_storage_write(ALICE, U256::from(0x30), U256::from(0x03));

    // Restore to cp2 (only slot 0x30 reverted)
    recorder.restore(cp2);

    let bal = recorder.build();
    let alice = &bal.accounts()[0];
    assert_eq!(alice.storage_changes.len(), 2); // slots 0x10 and 0x20

    // Now test restoring to cp1 from fresh state
    let mut recorder2 = BlockAccessListRecorder::new();
    recorder2.set_block_access_index(1);
    recorder2.record_storage_write(ALICE, U256::from(0x10), U256::from(0x01));
    let cp1_fresh = recorder2.checkpoint();
    recorder2.record_storage_write(ALICE, U256::from(0x20), U256::from(0x02));
    let _cp2_fresh = recorder2.checkpoint();
    recorder2.record_storage_write(ALICE, U256::from(0x30), U256::from(0x03));

    // Restore all the way to cp1
    recorder2.restore(cp1_fresh);

    let bal2 = recorder2.build();
    let alice2 = &bal2.accounts()[0];
    assert_eq!(alice2.storage_changes.len(), 1); // only slot 0x10
}

// ==================== SYSTEM_ADDRESS Tests ====================

#[test]
fn test_system_address_filtering() {
    // SYSTEM_ADDRESS should be excluded from BAL unless it has actual state changes
    let mut recorder = BlockAccessListRecorder::new();
    recorder.set_block_access_index(0); // System contract phase

    // Just touch SYSTEM_ADDRESS (no actual changes)
    recorder.record_touched_address(SYSTEM_ADDRESS);

    let bal = recorder.build();

    // SYSTEM_ADDRESS should not appear
    assert!(bal.is_empty());
}

#[test]
fn test_system_address_with_storage_change() {
    let mut recorder = BlockAccessListRecorder::new();
    recorder.set_block_access_index(0);

    // Actual storage change for SYSTEM_ADDRESS
    recorder.record_storage_write(SYSTEM_ADDRESS, U256::from(0x10), U256::from(0x42));

    let bal = recorder.build();

    // SYSTEM_ADDRESS should appear because it has actual state changes
    assert_eq!(bal.accounts().len(), 1);
    assert_eq!(bal.accounts()[0].address, SYSTEM_ADDRESS);
}

// ==================== Block Access Index Tests ====================

#[test]
fn test_block_access_index_semantics() {
    // Index 0: pre-execution (system contracts)
    // Index 1..n: transaction indices
    // Index n+1: post-execution (withdrawals)

    let mut recorder = BlockAccessListRecorder::new();

    // Pre-execution phase (index 0)
    recorder.set_block_access_index(0);
    recorder.record_storage_write(ALICE, U256::from(0x01), U256::from(0x10));

    // Transaction 1 (index 1)
    recorder.set_block_access_index(1);
    recorder.record_storage_write(ALICE, U256::from(0x02), U256::from(0x20));

    // Transaction 2 (index 2)
    recorder.set_block_access_index(2);
    recorder.record_storage_write(ALICE, U256::from(0x03), U256::from(0x30));

    // Post-execution/withdrawals (index 3 for 2 txs)
    recorder.set_block_access_index(3);
    recorder.record_balance_change(BOB, U256::from(1000));
    recorder.set_initial_balance(BOB, U256::from(0));

    let bal = recorder.build();

    let alice = bal.accounts().iter().find(|a| a.address == ALICE).unwrap();
    assert_eq!(alice.storage_changes.len(), 3);

    // Verify indices are correctly assigned
    let indices: Vec<u16> = alice
        .storage_changes
        .iter()
        .flat_map(|s| s.slot_changes.iter().map(|c| c.block_access_index))
        .collect();
    assert!(indices.contains(&0)); // pre-exec
    assert!(indices.contains(&1)); // tx 1
    assert!(indices.contains(&2)); // tx 2
}

// ==================== RLP Encoding/Decoding Tests ====================

#[test]
fn test_bal_rlp_roundtrip() {
    use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};

    let mut recorder = BlockAccessListRecorder::new();
    recorder.set_block_access_index(1);
    recorder.record_touched_address(ALICE);
    recorder.record_storage_write(ALICE, U256::from(0x10), U256::from(0x42));
    recorder.set_initial_balance(ALICE, U256::from(1000));
    recorder.record_balance_change(ALICE, U256::from(900));
    recorder.record_nonce_change(ALICE, 1);
    recorder.record_code_change(ALICE, bytes::Bytes::from_static(&[0x60, 0x00]));

    let original = recorder.build();
    let encoded = original.encode_to_vec();
    let decoded = BlockAccessList::decode(&encoded).expect("Failed to decode BAL");

    assert_eq!(original, decoded);
    assert_eq!(original.compute_hash(), decoded.compute_hash());
}

#[test]
fn test_storage_change_rlp_roundtrip() {
    use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};

    let change = StorageChange::new(5, U256::from(0x12345));
    let encoded = change.encode_to_vec();
    let decoded = StorageChange::decode(&encoded).expect("Failed to decode StorageChange");

    assert_eq!(change, decoded);
}

#[test]
fn test_balance_change_rlp_roundtrip() {
    use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};

    let change = BalanceChange::new(3, U256::from(1000));
    let encoded = change.encode_to_vec();
    let decoded = BalanceChange::decode(&encoded).expect("Failed to decode BalanceChange");

    assert_eq!(change, decoded);
}

#[test]
fn test_nonce_change_rlp_roundtrip() {
    use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};

    let change = NonceChange::new(2, 42);
    let encoded = change.encode_to_vec();
    let decoded = NonceChange::decode(&encoded).expect("Failed to decode NonceChange");

    assert_eq!(change, decoded);
}

#[test]
fn test_code_change_rlp_roundtrip() {
    let change = CodeChange::new(1, bytes::Bytes::from_static(&[0x60, 0x00, 0x60, 0x00]));
    let encoded = change.encode_to_vec();
    let decoded = CodeChange::decode(&encoded).expect("Failed to decode CodeChange");

    assert_eq!(change, decoded);
}

// ==================== RLP Encoding Hex Validation Tests ====================
// These tests verify specific RLP hex encodings for cross-implementation compatibility

#[test]
fn test_encode_decode_empty_list_validation() {
    let actual_bal = BlockAccessList::from_accounts(vec![AccountChanges::new(ALICE_ADDR)]);

    let mut buf = Vec::new();
    actual_bal.encode(&mut buf);

    let encoded_rlp = hex::encode(&buf);
    assert_eq!(
        &encoded_rlp,
        "dbda94000000000000000000000000000000000000000ac0c0c0c0c0"
    );

    let decoded_bal = BlockAccessList::decode(&buf).unwrap();
    assert_eq!(decoded_bal, actual_bal);
}

#[test]
fn test_encode_decode_partial_validation() {
    let actual_bal = BlockAccessList::from_accounts(vec![
        AccountChanges::new(ALICE_ADDR)
            .with_storage_reads(vec![U256::from(1), U256::from(2)])
            .with_balance_changes(vec![BalanceChange::new(1, U256::from(100))])
            .with_nonce_changes(vec![NonceChange::new(1, 1)]),
    ]);

    let mut buf = Vec::new();
    actual_bal.encode(&mut buf);

    let encoded_rlp = hex::encode(&buf);
    assert_eq!(
        &encoded_rlp,
        "e3e294000000000000000000000000000000000000000ac0c20102c3c20164c3c20101c0"
    );

    let decoded_bal = BlockAccessList::decode(&buf).unwrap();
    assert_eq!(decoded_bal, actual_bal);
}

#[test]
fn test_storage_changes_validation() {
    let actual_bal = BlockAccessList::from_accounts(vec![
        AccountChanges::new(CONTRACT_ADDR).with_storage_changes(vec![SlotChange::with_changes(
            U256::from(0x1),
            vec![StorageChange::new(1, U256::from(0x42))],
        )]),
    ]);

    let mut buf = Vec::new();
    actual_bal.encode(&mut buf);

    let encoded_rlp = hex::encode(buf);
    assert_eq!(
        &encoded_rlp,
        "e1e094000000000000000000000000000000000000000cc6c501c3c20142c0c0c0c0"
    );
}

#[test]
fn test_expected_addresses_auto_sorted() {
    let actual_bal = BlockAccessList::from_accounts(vec![
        AccountChanges::new(CHARLIE_ADDR),
        AccountChanges::new(ALICE_ADDR),
        AccountChanges::new(BOB_ADDR),
    ]);

    let mut buf = Vec::new();
    actual_bal.encode(&mut buf);

    let encoded_rlp = hex::encode(buf);
    assert_eq!(
        &encoded_rlp,
        "f851da94000000000000000000000000000000000000000ac0c0c0c0c0da94000000000000000000000000000000000000000bc0c0c0c0c0da94000000000000000000000000000000000000000cc0c0c0c0c0"
    );
}

#[test]
fn test_expected_storage_slots_ordering_correct_order_should_pass() {
    let actual_bal =
        BlockAccessList::from_accounts(vec![AccountChanges::new(ALICE_ADDR).with_storage_changes(
            vec![
                SlotChange::new(U256::from(0x02)),
                SlotChange::new(U256::from(0x01)),
                SlotChange::new(U256::from(0x03)),
            ],
        )]);

    let mut buf = Vec::new();
    actual_bal.encode(&mut buf);

    let encoded_rlp = hex::encode(&buf);
    assert_eq!(
        &encoded_rlp,
        "e4e394000000000000000000000000000000000000000ac9c201c0c202c0c203c0c0c0c0c0"
    );
}

#[test]
fn test_expected_storage_reads_ordering_correct_order_should_pass() {
    let actual_bal = BlockAccessList::from_accounts(vec![
        AccountChanges::new(ALICE_ADDR).with_storage_reads(vec![
            U256::from(0x02),
            U256::from(0x01),
            U256::from(0x03),
        ]),
    ]);

    let mut buf = Vec::new();
    actual_bal.encode(&mut buf);

    let encoded_rlp = hex::encode(buf);
    assert_eq!(
        &encoded_rlp,
        "dedd94000000000000000000000000000000000000000ac0c3010203c0c0c0"
    );
}

#[test]
fn test_expected_tx_indices_ordering_correct_order_should_pass() {
    let actual_bal =
        BlockAccessList::from_accounts(vec![AccountChanges::new(ALICE_ADDR).with_nonce_changes(
            vec![
                NonceChange::new(2, 2),
                NonceChange::new(3, 3),
                NonceChange::new(1, 1),
            ],
        )]);

    let mut buf = Vec::new();
    actual_bal.encode(&mut buf);

    let encoded_rlp = hex::encode(buf);
    assert_eq!(
        &encoded_rlp,
        "e4e394000000000000000000000000000000000000000ac0c0c0c9c20101c20202c20303c0"
    );
}

#[test]
fn test_decode_storage_slots_ordering_correct_order_should_pass() {
    let actual_bal =
        BlockAccessList::from_accounts(vec![AccountChanges::new(ALICE_ADDR).with_storage_changes(
            vec![
                SlotChange::new(U256::from(0x01)),
                SlotChange::new(U256::from(0x02)),
                SlotChange::new(U256::from(0x03)),
            ],
        )]);

    let encoded_rlp: Vec<u8> =
        hex::decode("e4e394000000000000000000000000000000000000000ac9c201c0c202c0c203c0c0c0c0c0")
            .unwrap();

    let decoded_bal = BlockAccessList::decode(&encoded_rlp).unwrap();
    assert_eq!(decoded_bal, actual_bal);
}

// ==================== BlockAccessListRecorder Tests ====================

#[test]
fn test_recorder_empty_build() {
    let recorder = BlockAccessListRecorder::new();
    let bal = recorder.build();
    assert!(bal.is_empty());
}

#[test]
fn test_recorder_touched_address_only() {
    let mut recorder = BlockAccessListRecorder::new();
    recorder.record_touched_address(ALICE_ADDR);
    let bal = recorder.build();

    assert_eq!(bal.accounts().len(), 1);
    let account = &bal.accounts()[0];
    assert_eq!(account.address, ALICE_ADDR);
    // Account with no changes should still appear (per EIP-7928)
    assert!(account.storage_changes.is_empty());
    assert!(account.balance_changes.is_empty());
}

#[test]
fn test_recorder_storage_read_then_write_becomes_write() {
    let mut recorder = BlockAccessListRecorder::new();
    recorder.set_block_access_index(1);

    // First read a slot
    recorder.record_storage_read(ALICE_ADDR, U256::from(0x10));
    // Then write to the same slot
    recorder.record_storage_write(ALICE_ADDR, U256::from(0x10), U256::from(0x42));

    let bal = recorder.build();

    assert_eq!(bal.accounts().len(), 1);
    let account = &bal.accounts()[0];
    // The slot should appear in writes, not reads
    assert_eq!(account.storage_changes.len(), 1);
    assert!(account.storage_reads.is_empty());
    assert_eq!(account.storage_changes[0].slot, U256::from(0x10));
}

#[test]
fn test_recorder_storage_read_only() {
    let mut recorder = BlockAccessListRecorder::new();
    recorder.set_block_access_index(1);

    recorder.record_storage_read(ALICE_ADDR, U256::from(0x10));
    recorder.record_storage_read(ALICE_ADDR, U256::from(0x20));

    let bal = recorder.build();

    assert_eq!(bal.accounts().len(), 1);
    let account = &bal.accounts()[0];
    assert!(account.storage_changes.is_empty());
    assert_eq!(account.storage_reads.len(), 2);
}

#[test]
fn test_recorder_multiple_writes_same_slot() {
    let mut recorder = BlockAccessListRecorder::new();
    recorder.set_block_access_index(1);
    recorder.record_storage_write(ALICE_ADDR, U256::from(0x10), U256::from(0x01));
    recorder.set_block_access_index(2);
    recorder.record_storage_write(ALICE_ADDR, U256::from(0x10), U256::from(0x02));

    let bal = recorder.build();

    let account = &bal.accounts()[0];
    assert_eq!(account.storage_changes.len(), 1);
    let slot_change = &account.storage_changes[0];
    // Should have two changes with different indices
    assert_eq!(slot_change.slot_changes.len(), 2);
}

#[test]
fn test_recorder_balance_roundtrip_filtered_within_tx() {
    // Per EIP-7928: "If an account's balance changes during a transaction, but its
    // post-transaction balance is equal to its pre-transaction balance, then the
    // change MUST NOT be recorded."
    let mut recorder = BlockAccessListRecorder::new();
    recorder.set_block_access_index(1);

    // Set initial balance
    recorder.set_initial_balance(ALICE_ADDR, U256::from(1000));
    // Record changes within the SAME transaction that round-trip
    recorder.record_balance_change(ALICE_ADDR, U256::from(500)); // decrease
    recorder.record_balance_change(ALICE_ADDR, U256::from(1000)); // back to initial

    let bal = recorder.build();

    let account = &bal.accounts()[0];
    // Balance round-tripped within same TX, so balance_changes should be empty
    assert!(account.balance_changes.is_empty());
}

#[test]
fn test_recorder_balance_changes_across_txs_not_filtered() {
    // Per EIP-7928: Per-transaction filtering means changes across different
    // transactions are evaluated independently.
    let mut recorder = BlockAccessListRecorder::new();
    recorder.set_block_access_index(1);

    // Set initial balance for TX 1
    recorder.set_initial_balance(ALICE_ADDR, U256::from(1000));
    // TX 1: decrease to 500 (NOT round-trip: 1000 -> 500)
    recorder.record_balance_change(ALICE_ADDR, U256::from(500));

    // TX 2: increase back to 1000 (NOT round-trip: 500 -> 1000)
    recorder.set_block_access_index(2);
    recorder.record_balance_change(ALICE_ADDR, U256::from(1000));

    let bal = recorder.build();

    let account = &bal.accounts()[0];
    // Both transactions have actual balance changes (not round-trips within their tx)
    // TX 1: 1000 -> 500, TX 2: 500 -> 1000
    assert_eq!(account.balance_changes.len(), 2);
}

#[test]
fn test_recorder_balance_change_recorded() {
    let mut recorder = BlockAccessListRecorder::new();
    recorder.set_block_access_index(1);

    recorder.set_initial_balance(ALICE_ADDR, U256::from(1000));
    recorder.record_balance_change(ALICE_ADDR, U256::from(500));

    let bal = recorder.build();

    let account = &bal.accounts()[0];
    // Balance changed to different value, should be recorded
    assert_eq!(account.balance_changes.len(), 1);
    assert_eq!(account.balance_changes[0].post_balance, U256::from(500));
}

#[test]
fn test_recorder_nonce_change() {
    let mut recorder = BlockAccessListRecorder::new();
    recorder.set_block_access_index(1);

    recorder.record_nonce_change(ALICE_ADDR, 1);

    let bal = recorder.build();

    let account = &bal.accounts()[0];
    assert_eq!(account.nonce_changes.len(), 1);
    assert_eq!(account.nonce_changes[0].post_nonce, 1);
}

#[test]
fn test_recorder_code_change() {
    let mut recorder = BlockAccessListRecorder::new();
    recorder.set_block_access_index(1);

    recorder.record_code_change(ALICE_ADDR, bytes::Bytes::from_static(&[0x60, 0x00]));

    let bal = recorder.build();

    let account = &bal.accounts()[0];
    assert_eq!(account.code_changes.len(), 1);
    assert_eq!(
        account.code_changes[0].new_code,
        &bytes::Bytes::from_static(&[0x60, 0x00])
    );
}

#[test]
fn test_recorder_system_address_excluded_when_only_touched() {
    let mut recorder = BlockAccessListRecorder::new();
    // Just touch SYSTEM_ADDRESS without actual state changes
    recorder.record_touched_address(SYSTEM_ADDRESS);

    let bal = recorder.build();
    // SYSTEM_ADDRESS should not appear if only touched
    assert!(bal.is_empty());
}

#[test]
fn test_recorder_system_address_included_with_state_change() {
    let mut recorder = BlockAccessListRecorder::new();
    recorder.set_block_access_index(1);
    // Record an actual state change for SYSTEM_ADDRESS
    recorder.record_storage_write(SYSTEM_ADDRESS, U256::from(0x10), U256::from(0x42));

    let bal = recorder.build();
    // SYSTEM_ADDRESS should appear because it has actual state changes
    assert_eq!(bal.accounts().len(), 1);
    assert_eq!(bal.accounts()[0].address, SYSTEM_ADDRESS);
}

#[test]
fn test_recorder_multiple_addresses_sorted() {
    let mut recorder = BlockAccessListRecorder::new();
    recorder.record_touched_address(CHARLIE_ADDR);
    recorder.record_touched_address(ALICE_ADDR);
    recorder.record_touched_address(BOB_ADDR);

    let bal = recorder.build();

    // Addresses should be sorted lexicographically in the encoded output
    assert_eq!(bal.accounts().len(), 3);
    // BTreeSet maintains order, so the build() returns them in sorted order
    let addresses: Vec<_> = bal.accounts().iter().map(|a| a.address).collect();
    // The set should be sorted
    let mut sorted = addresses.clone();
    sorted.sort();
    assert_eq!(addresses, sorted);
}

// ==================== EIP-7928 Execution Spec Tests ====================

#[test]
fn test_bal_self_transfer() {
    // Per EIP-7928: Self-transfers where an account sends value to itself
    // result in balance changes that round-trip within the same TX.
    let mut recorder = BlockAccessListRecorder::new();
    recorder.set_block_access_index(1);

    // Initial balance of 1000
    recorder.set_initial_balance(ALICE_ADDR, U256::from(1000));
    // Self-transfer: balance goes down then back up by same amount
    // (In a real self-transfer, the net effect is zero)
    recorder.record_balance_change(ALICE_ADDR, U256::from(1000)); // No net change

    let bal = recorder.build();

    let account = &bal.accounts()[0];
    // Self-transfer with no net balance change should result in empty balance_changes
    assert!(account.balance_changes.is_empty());
}

#[test]
fn test_bal_zero_value_transfer() {
    // Per EIP-7928: Zero-value transfers touch accounts but don't change balances.
    // Both sender and recipient must appear in BAL even with no balance changes.
    let mut recorder = BlockAccessListRecorder::new();
    recorder.set_block_access_index(1);

    // Touch both addresses (simulating a zero-value transfer)
    recorder.record_touched_address(ALICE_ADDR); // sender
    recorder.record_touched_address(BOB_ADDR); // recipient

    // Set initial balances (no actual change occurs in zero-value transfer)
    recorder.set_initial_balance(ALICE_ADDR, U256::from(1000));
    recorder.set_initial_balance(BOB_ADDR, U256::from(500));

    // Record same balances (no change)
    recorder.record_balance_change(ALICE_ADDR, U256::from(1000));
    recorder.record_balance_change(BOB_ADDR, U256::from(500));

    let bal = recorder.build();

    // Both accounts should appear (they were touched)
    assert_eq!(bal.accounts().len(), 2);
    // Neither should have balance_changes (balances unchanged)
    for account in bal.accounts() {
        assert!(account.balance_changes.is_empty());
    }
}

#[test]
fn test_bal_checkpoint_restore_preserves_touched_addresses() {
    // Per EIP-7928: "State changes from reverted calls are discarded, but all
    // accessed addresses must be included."
    let mut recorder = BlockAccessListRecorder::new();
    recorder.set_block_access_index(1);

    // Record some state before checkpoint
    recorder.record_touched_address(ALICE_ADDR);
    recorder.record_storage_write(ALICE_ADDR, U256::from(0x10), U256::from(0x01));

    // Take checkpoint (simulating entering a nested call)
    let checkpoint = recorder.checkpoint();

    // Record more state that will be reverted
    recorder.record_touched_address(BOB_ADDR);
    recorder.record_storage_write(BOB_ADDR, U256::from(0x20), U256::from(0x02));

    // Revert (simulating nested call failure)
    recorder.restore(checkpoint);

    let bal = recorder.build();

    // ALICE should have her storage write preserved
    // BOB's storage write should be reverted
    // BUT both addresses should still appear (touched_addresses persists)
    assert_eq!(bal.accounts().len(), 2);

    let alice = bal
        .accounts()
        .iter()
        .find(|a| a.address == ALICE_ADDR)
        .unwrap();
    let bob = bal
        .accounts()
        .iter()
        .find(|a| a.address == BOB_ADDR)
        .unwrap();

    // Alice's storage write survived
    assert_eq!(alice.storage_changes.len(), 1);
    // Bob's storage write was reverted
    assert!(bob.storage_changes.is_empty());
}

#[test]
fn test_bal_reverted_write_restores_read() {
    // When a slot is read, then written (which removes it from reads), then
    // the write is reverted, the slot should be restored as a read.
    let mut recorder = BlockAccessListRecorder::new();
    recorder.set_block_access_index(1);

    // Read a slot
    recorder.record_storage_read(ALICE_ADDR, U256::from(0x10));

    // Take checkpoint
    let checkpoint = recorder.checkpoint();

    // Write to the same slot (this removes it from reads and adds to writes)
    recorder.record_storage_write(ALICE_ADDR, U256::from(0x10), U256::from(0x42));

    // Revert the write
    recorder.restore(checkpoint);

    let bal = recorder.build();

    let account = &bal.accounts()[0];
    // The write was reverted, so slot should be back in reads
    assert_eq!(account.storage_reads.len(), 1);
    assert!(account.storage_reads.contains(&U256::from(0x10)));
    // And not in writes
    assert!(account.storage_changes.is_empty());
}
