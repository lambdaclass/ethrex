//! Equivalence gate for `gen-state`'s direct flat-KV writer.
//!
//! `gen-state` writes the flat-KV index itself (deriving entries from the data
//! it inserts into the tries) instead of calling the store's background
//! `generate_flatkeyvalue()`. For that to be correct the entries must be keyed
//! exactly as the store's generator keys them. This test builds a small state,
//! runs the REAL generator as the oracle, then asserts every entry it wrote is
//! retrievable under the keys `state_bench::fkv` produces — proving the bench's
//! key encoding is byte-identical to the store's, for both accounts and storage.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use ethrex_common::types::AccountState;
use ethrex_common::{Address, H256, U256};
use ethrex_crypto::NativeCrypto;
use ethrex_crypto::keccak::keccak_hash;
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::api::tables::{ACCOUNT_FLATKEYVALUE, STORAGE_FLATKEYVALUE};
use ethrex_storage::{EngineType, Store, hash_address, hash_key};
use ethrex_trie::EMPTY_TRIE_HASH;

use state_bench::fkv::{account_fkv_key, storage_fkv_key};

/// Disk-backed scratch base so the throwaway RocksDB doesn't land on a small
/// tmpfs `/tmp`.
fn scratch_base() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        let cache = PathBuf::from(home).join(".cache").join("tmp");
        if std::fs::create_dir_all(&cache).is_ok() {
            return cache;
        }
    }
    std::env::temp_dir()
}

const NUM_ACCOUNTS: u64 = 6;
const SLOTS_PER_ACCOUNT: u64 = 8;

fn address_of(i: u64) -> Address {
    Address::from_slice(&keccak_hash(i.to_be_bytes())[..20])
}
fn slot_key(i: u64) -> H256 {
    H256(keccak_hash([b"k".as_ref(), &i.to_be_bytes()].concat()))
}
fn slot_value(i: u64) -> U256 {
    U256::from_big_endian(&keccak_hash([b"v".as_ref(), &i.to_be_bytes()].concat())) + U256::one()
}

#[tokio::test]
async fn direct_fkv_keys_match_store_generator() {
    let dir = tempfile::tempdir_in(scratch_base()).unwrap();
    let store = Store::new(dir.path().to_str().unwrap(), EngineType::RocksDB).unwrap();

    // Build storage-bearing accounts exactly as gen-state does: hashed storage
    // keys inserted into a direct storage trie, then the account inserted into
    // the state trie with the resulting storage_root.
    let mut expected_accounts: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    let mut expected_storage: Vec<(H256, Vec<u8>, Vec<u8>)> = Vec::new(); // (acct_hash, hashed_key, value_rlp)
    let mut state_trie = store.open_direct_state_trie(*EMPTY_TRIE_HASH).unwrap();

    for a in 0..NUM_ACCOUNTS {
        let address = address_of(a);
        let account_hash = H256::from_slice(&hash_address(&address));
        let mut storage_trie = store
            .open_direct_storage_trie(account_hash, *EMPTY_TRIE_HASH)
            .unwrap();
        for s in 0..SLOTS_PER_ACCOUNT {
            let idx = a * SLOTS_PER_ACCOUNT + s;
            let hashed = hash_key(&slot_key(idx));
            let value_rlp = slot_value(idx).encode_to_vec();
            storage_trie
                .insert(hashed.clone(), value_rlp.clone())
                .unwrap();
            expected_storage.push((account_hash, hashed, value_rlp));
        }
        let storage_root = storage_trie.hash(&NativeCrypto).unwrap();

        let state = AccountState {
            nonce: 1,
            balance: U256::from(a + 1),
            storage_root,
            code_hash: *ethrex_common::constants::EMPTY_KECCAK_HASH,
        };
        let hashed_address = hash_address(&address);
        let state_rlp = state.encode_to_vec();
        state_trie
            .insert(hashed_address.clone(), state_rlp.clone())
            .unwrap();
        expected_accounts.push((hashed_address, state_rlp));
    }
    // Persist the state trie root so the generator can find it.
    state_trie.hash(&NativeCrypto).unwrap();

    // Run the store's real flat-KV generator as the oracle.
    store.generate_flatkeyvalue().unwrap();
    let start = Instant::now();
    loop {
        let lw = store.last_written().unwrap();
        if !lw.is_empty() && lw.iter().all(|b| *b == 0xff) {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "flat-KV generation did not finish in time"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Every account the generator wrote must be retrievable under our key.
    for (hashed_address, state_rlp) in &expected_accounts {
        let got = store
            .read(ACCOUNT_FLATKEYVALUE, account_fkv_key(hashed_address))
            .unwrap();
        assert_eq!(
            got.as_ref(),
            Some(state_rlp),
            "account flat-KV key mismatch vs store generator"
        );
    }

    // Every storage slot the generator wrote must be retrievable under our key.
    for (account_hash, hashed_key, value_rlp) in &expected_storage {
        let got = store
            .read(
                STORAGE_FLATKEYVALUE,
                storage_fkv_key(*account_hash, hashed_key),
            )
            .unwrap();
        assert_eq!(
            got.as_ref(),
            Some(value_rlp),
            "storage flat-KV key mismatch vs store generator"
        );
    }
}
