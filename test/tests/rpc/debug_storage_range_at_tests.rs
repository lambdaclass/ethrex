use ethrex_common::{Address, H256, U256, utils::keccak};
use serde_json::{Value, json};

use super::helpers::{rpc_call, rpc_call_expect_err, setup_block_with_storage_contract};

/// Hashed slot for a u256 slot index. Storage-trie entries are keyed by
/// `keccak256(slot)`, so this is how callers locate a known slot in the
/// `nextKey`-style response.
fn hashed_slot(slot: u64) -> H256 {
    let bytes = U256::from(slot).to_big_endian();
    keccak(bytes)
}

async fn call_range(
    store: &ethrex_storage::Store,
    block: Value,
    contract: Address,
    start_key: H256,
    max: u64,
) -> Value {
    rpc_call(
        store,
        "debug_storageRangeAt",
        vec![
            block,
            json!(0),
            json!(format!("{contract:#x}")),
            json!(format!("{start_key:#x}")),
            json!(max),
        ],
    )
    .await
}

#[tokio::test]
async fn storage_range_returns_contract_slots() {
    let env = setup_block_with_storage_contract().await;
    let block_hash = env.block.hash();

    let result = call_range(
        &env.store,
        json!(format!("{block_hash:#x}")),
        env.contract,
        H256::zero(),
        1_000,
    )
    .await;

    let obj = result.as_object().expect("response should be an object");
    let storage = obj["storage"]
        .as_object()
        .expect("storage should be an object");
    assert_eq!(storage.len(), 3, "deployed contract has 3 storage slots");

    // Verify each known slot appears under its hashed key with the expected
    // value. The deploy code wrote slot 0 = 0x11, slot 1 = 0x22, slot 2 = 0x33.
    for (slot_idx, expected_value) in [(0u64, 0x11u8), (1, 0x22), (2, 0x33)] {
        let key = format!("{:#x}", hashed_slot(slot_idx));
        let entry = storage
            .get(&key)
            .or_else(|| {
                storage
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(&key))
                    .map(|(_, v)| v)
            })
            .unwrap_or_else(|| panic!("slot {slot_idx} (hashed: {key}) missing from response"));
        assert!(
            entry["key"].is_null(),
            "preimage is not stored, key must be null"
        );
        let value = entry["value"].as_str().unwrap();
        let v: U256 = U256::from_str_radix(value.trim_start_matches("0x"), 16).unwrap();
        assert_eq!(
            v,
            U256::from(expected_value),
            "slot {slot_idx} should be 0x{expected_value:02x}"
        );
    }
    // Iteration complete — nextKey absent or null.
    assert!(
        obj.get("nextKey").is_none() || obj["nextKey"].is_null(),
        "complete iteration must have null nextKey"
    );
}

#[tokio::test]
async fn storage_range_paginates_via_next_key() {
    let env = setup_block_with_storage_contract().await;
    let block_hash = env.block.hash();

    // Three slots in trie-order. Request one at a time and walk via nextKey.
    let mut cursor = H256::zero();
    let mut seen = Vec::new();
    for _ in 0..3 {
        let page = call_range(
            &env.store,
            json!(format!("{block_hash:#x}")),
            env.contract,
            cursor,
            1,
        )
        .await;
        let storage = page["storage"].as_object().unwrap();
        assert_eq!(storage.len(), 1, "max=1 page returns exactly one slot");
        let key = storage.keys().next().unwrap().clone();
        assert!(!seen.contains(&key), "page must not repeat a previous slot");
        seen.push(key);
        let next = &page["nextKey"];
        if next.is_null() {
            break;
        }
        cursor = next.as_str().unwrap().parse().unwrap();
    }
    assert_eq!(seen.len(), 3, "should have walked through all three slots");

    // One more page from the post-last cursor must yield empty + null nextKey.
    let final_page = call_range(
        &env.store,
        json!(format!("{block_hash:#x}")),
        env.contract,
        // Walk from the last seen key + 1 to be safe; use zero-cursor + max
        // (effectively "give me all remaining" — already past).
        H256::repeat_byte(0xFF),
        1_000,
    )
    .await;
    assert!(final_page["storage"].as_object().unwrap().is_empty());
    assert!(
        final_page["nextKey"].is_null(),
        "exhausted iteration must have null nextKey"
    );
}

#[tokio::test]
async fn storage_range_unknown_address_returns_empty() {
    let env = setup_block_with_storage_contract().await;
    let block_hash = env.block.hash();

    // Query a random non-existent account.
    let result = call_range(
        &env.store,
        json!(format!("{block_hash:#x}")),
        Address::from_low_u64_be(0xDEAD_BEEF),
        H256::zero(),
        1_000,
    )
    .await;

    let obj = result.as_object().expect("response should be an object");
    assert!(
        obj["storage"].as_object().unwrap().is_empty(),
        "unknown account must return empty storage"
    );
    assert!(
        obj["nextKey"].is_null(),
        "unknown account must have null nextKey"
    );
}

#[tokio::test]
async fn storage_range_eoa_returns_empty() {
    // The sender is an EOA — has no storage trie. Should behave identically
    // to "unknown address": empty + null nextKey, no error.
    let env = setup_block_with_storage_contract().await;
    let block_hash = env.block.hash();

    let result = call_range(
        &env.store,
        json!(format!("{block_hash:#x}")),
        env.sender,
        H256::zero(),
        1_000,
    )
    .await;

    let obj = result.as_object().expect("response should be an object");
    assert!(obj["storage"].as_object().unwrap().is_empty());
    assert!(obj["nextKey"].is_null());
}

#[tokio::test]
async fn storage_range_by_block_number() {
    let env = setup_block_with_storage_contract().await;

    let result = call_range(
        &env.store,
        json!(format!("{:#x}", env.block.header.number)),
        env.contract,
        H256::zero(),
        1_000,
    )
    .await;
    let storage = result["storage"].as_object().unwrap();
    assert_eq!(
        storage.len(),
        3,
        "block-number form must resolve the same state"
    );
}

#[tokio::test]
async fn storage_range_invalid_block_hash_errors() {
    let env = setup_block_with_storage_contract().await;

    let err = rpc_call_expect_err(
        &env.store,
        "debug_storageRangeAt",
        vec![
            json!(format!("{:#x}", H256::from_low_u64_be(0xdeadbeef))),
            json!(0),
            json!(format!("{:#x}", env.contract)),
            json!(format!("{:#x}", H256::zero())),
            json!(1_u64),
        ],
    )
    .await;
    let msg = format!("{err:?}");
    assert!(
        msg.contains("Block not found"),
        "expected block-not-found error, got: {msg}"
    );
}
