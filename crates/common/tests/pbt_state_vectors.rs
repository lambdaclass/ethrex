//! Conformance tests for `types::pbt_state`: whole state in, the
//! spec's state root out.
//!
//! `ethrex-binary-trie`'s own `tests/spec_vectors.rs` pins the pieces —
//! key derivation, code chunking, basic-data packing, and the tree's
//! roots over raw key/value entries. None of that pins the *composition*
//! that [`apply_account_updates`] performs: which leaves an account
//! contributes, and which it deliberately does not. That is what this
//! file pins, against the `pbt_state` section of the same vendored
//! fixture, whose roots come from the spec's own `src/ethereum/state_pbt.py`.
//!
//! The fixture lives with the crate that owns the rest of it,
//! `crates/common/binary-trie/tests/vectors/`; see that crate's README
//! for how it is refreshed.

use std::collections::BTreeMap;

use ethrex_binary_trie::trie::BinaryTrie;
use ethrex_common::types::pbt_state::apply_account_updates;
use ethrex_common::types::{AccountInfo, AccountUpdate, Code};
use ethrex_common::{Address, Bytes, H256, NativeCrypto, U256};
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    pbt_state: Vec<StateCase>,
}

/// A whole state and the root it commits to.
#[derive(Deserialize)]
struct StateCase {
    name: String,
    /// Keyed by the account's 20-byte address, as hex. Order is not
    /// significant: the root is a function of the account set alone.
    accounts: BTreeMap<String, AccountSpec>,
    root: String,
}

#[derive(Deserialize)]
struct AccountSpec {
    nonce: u64,
    /// Hex string; balances can exceed a JSON-safe integer.
    balance: String,
    code: String,
    /// `keccak256(code)`, restated by the fixture because the overflow
    /// code chunk keys are content-addressed by it.
    code_hash: String,
    /// Keyed by decimal slot number, which ranges over all of `2**256`;
    /// values are 32-byte hex.
    storage: BTreeMap<String, String>,
}

fn unhex(s: &str) -> Vec<u8> {
    hex::decode(s.strip_prefix("0x").unwrap_or(s)).expect("fixture hex string")
}

fn hex_u256(s: &str) -> U256 {
    U256::from_str_radix(s.trim_start_matches("0x"), 16).expect("fixture hex integer")
}

fn load() -> Fixture {
    let fixture: Fixture = serde_json::from_str(include_str!(
        "../binary-trie/tests/vectors/binary_trie_vectors.json"
    ))
    .expect("fixture parses");
    // The fixture is vendored and refreshed from upstream, so its case
    // count is expected to grow. Assert only that the section did not
    // arrive empty — an exact count would fail every legitimate refresh,
    // but a fixture that lost the section must not pass vacuously.
    assert!(!fixture.pbt_state.is_empty(), "no pbt_state cases");
    fixture
}

/// The account updates that build `case`'s state from nothing.
///
/// These cases only ever build state up, so no update removes anything.
fn updates_for(case: &StateCase) -> Vec<AccountUpdate> {
    case.accounts
        .iter()
        .map(|(address, account)| {
            let code = Code::from_bytecode(Bytes::from(unhex(&account.code)), &NativeCrypto);
            assert_eq!(
                code.hash.as_bytes(),
                unhex(&account.code_hash).as_slice(),
                "case {}: fixture code_hash is keccak256(code)",
                case.name
            );
            AccountUpdate {
                address: Address::from_slice(&unhex(address)),
                removed: false,
                info: Some(AccountInfo {
                    code_hash: code.hash,
                    balance: hex_u256(&account.balance),
                    nonce: account.nonce,
                }),
                code: Some(code),
                added_storage: account
                    .storage
                    .iter()
                    .map(|(slot, value)| {
                        let slot = U256::from_dec_str(slot).expect("fixture decimal slot");
                        (
                            H256(slot.to_big_endian()),
                            U256::from_big_endian(&unhex(value)),
                        )
                    })
                    .collect(),
                removed_storage: false,
            }
        })
        .collect()
}

#[test]
fn pbt_state_roots_match_spec() {
    let cases = load().pbt_state;
    let mut ran = Vec::new();
    for case in &cases {
        let mut trie = BinaryTrie::new_temp();
        apply_account_updates(&mut trie, &updates_for(case))
            .unwrap_or_else(|err| panic!("case {}: updates apply: {err}", case.name));
        assert_eq!(
            trie.root().as_bytes(),
            unhex(&case.root).as_slice(),
            "state root, case {}",
            case.name
        );
        ran.push(case.name.as_str());
    }
    println!("pbt_state: {} cases passed: {}", ran.len(), ran.join(", "));
}
