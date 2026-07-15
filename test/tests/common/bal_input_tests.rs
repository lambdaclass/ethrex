//! Supplied-BAL input hardening on the `ethrex-common` side (companion to the
//! RPC/decode hardening PR): the `0x`-prefix strictness of the BAL hex
//! deserializers and the capacity-hint cap in `build_validation_index`.
//!
//! The deserializers are exercised directly via `serde_json::Value` (which
//! implements `Deserializer`), so no serde-derive wrapper is needed.

use ethrex_common::Address;
use ethrex_common::U256;
use ethrex_common::serde_utils::block_access_list::{rlp_str, rlp_str_opt};
use ethrex_common::types::block_access_list::{AccountChanges, BalanceChange, BlockAccessList};
use ethrex_rlp::encode::RLPEncode;
use serde_json::Value;

/// `0x` + RLP hex of an empty BAL (`0xc0`).
fn empty_bal_hex() -> String {
    format!(
        "0x{}",
        hex::encode(BlockAccessList::from_accounts(vec![]).encode_to_vec())
    )
}

#[test]
fn rlp_str_accepts_prefixed_and_rejects_unprefixed() {
    let hex = empty_bal_hex();
    let bal = rlp_str::deserialize(Value::String(hex.clone())).expect("prefixed BAL must decode");
    assert_eq!(bal.accounts().len(), 0);

    // Same bytes without the mandatory 0x prefix must be rejected.
    let unprefixed = hex.trim_start_matches("0x").to_string();
    assert!(
        rlp_str::deserialize(Value::String(unprefixed)).is_err(),
        "unprefixed BAL hex must be rejected"
    );
}

#[test]
fn rlp_str_opt_prefix_and_absence_handling() {
    // "0x" encodes absence → None.
    assert!(
        rlp_str_opt::deserialize(Value::String("0x".into()))
            .unwrap()
            .is_none()
    );
    // JSON null → None.
    assert!(rlp_str_opt::deserialize(Value::Null).unwrap().is_none());
    // A prefixed valid BAL → Some.
    assert!(
        rlp_str_opt::deserialize(Value::String(empty_bal_hex()))
            .unwrap()
            .is_some()
    );
    // Unprefixed (schema-invalid) → error, not silently accepted.
    assert!(
        rlp_str_opt::deserialize(Value::String("c0".into())).is_err(),
        "unprefixed BAL hex must be rejected"
    );
}

/// The capacity hints in `build_validation_index` are capped against a supplied
/// BAL's (attacker-controlled) account count, but the cap is only a
/// pre-allocation hint: a block with more accounts than the cap must still be
/// indexed in full. Binds the `PREALLOC_CAP` min() against silent truncation.
#[test]
fn build_validation_index_indexes_more_accounts_than_prealloc_cap() {
    let n = 8192usize + 5; // > PREALLOC_CAP
    let accounts: Vec<AccountChanges> = (0..n)
        .map(|i| {
            let mut a = Address::zero();
            a.0[16..20].copy_from_slice(&(i as u32).to_be_bytes());
            AccountChanges::new(a)
                .with_balance_changes(vec![BalanceChange::new(1, U256::from(i as u64 + 1))])
        })
        .collect();
    let bal = BlockAccessList::from_accounts(accounts);
    let index = bal.build_validation_index();
    assert_eq!(
        index.addr_to_idx.len(),
        n,
        "every account must be indexed despite the capacity-hint cap"
    );
}
