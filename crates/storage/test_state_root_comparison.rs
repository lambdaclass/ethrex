/// Test to compare state root computation between legacy trie and ethrex_db
///
/// This test creates identical account data and inserts it into both implementations
/// to verify they produce the same state root hash.

#[cfg(test)]
mod state_root_comparison {
    use ethrex_common::{Address, H256, U256};
    use ethrex_common::types::AccountState;
    use ethrex_rlp::encode::RLPEncode;
    use ethrex_trie::{Trie, EMPTY_TRIE_HASH};
    use ethrex_db::store::{StateTrie, AccountData, StorageTrie};
    use ethrex_db::merkle::EMPTY_ROOT;
    use std::str::FromStr;
    use crate::Store;

    fn keccak_hash(data: &[u8]) -> [u8; 32] {
        use ethrex_crypto::keccak::keccak_hash as keccak;
        keccak(data)
    }

    fn hash_address(address: &Address) -> H256 {
        H256::from(keccak_hash(&address.to_fixed_bytes()))
    }

    #[test]
    fn test_simple_single_account_state_root() {
        println!("\n=== Test: Simple Single Account ===");

        // Create a simple account
        let address = Address::from_str("0x1000000000000000000000000000000000000001").unwrap();
        let nonce = 1u64;
        let balance = U256::from(1000);
        let code_hash = H256::from_slice(&keccak_hash(&[])); // Empty code
        let storage_root = *EMPTY_TRIE_HASH;

        println!("Account:");
        println!("  Address:      {}", address);
        println!("  Nonce:        {}", nonce);
        println!("  Balance:      {}", balance);
        println!("  Code Hash:    {}", code_hash);
        println!("  Storage Root: {}", storage_root);

        // === LEGACY TRIE ===
        println!("\n--- Legacy Trie ---");
        use crate::backend::in_memory::InMemoryBackend;
        use crate::trie::BackendTrieDB;
        let backend = std::sync::Arc::new(InMemoryBackend::open().unwrap());
        let trie_db = Box::new(BackendTrieDB::new_for_accounts(backend, Vec::new()).unwrap());
        let mut legacy_trie = Trie::new(trie_db);

        let account_state = AccountState {
            nonce,
            balance,
            code_hash,
            storage_root,
        };

        let hashed_address = hash_address(&address);
        let encoded_account = account_state.encode_to_vec();

        println!("Hashed Address: {}", hashed_address);
        println!("Encoded Account (legacy): {}", hex::encode(&encoded_account));
        println!("Encoded Length: {} bytes", encoded_account.len());

        legacy_trie.insert(hashed_address.as_bytes().to_vec(), encoded_account.clone()).unwrap();
        let (legacy_state_root, _) = legacy_trie.collect_changes_since_last_hash();

        println!("Legacy State Root: {}", legacy_state_root);

        // === ETHREX_DB ===
        println!("\n--- ethrex_db ---");
        let mut ethrex_db_trie = StateTrie::new();

        let address_bytes: [u8; 20] = address.to_fixed_bytes();
        let balance_bytes: [u8; 32] = ethrex_common::utils::u256_to_big_endian(balance);

        let account_data = AccountData {
            nonce,
            balance: balance_bytes,
            storage_root: EMPTY_ROOT,
            code_hash: code_hash.to_fixed_bytes(),
        };

        let encoded_ethrex_db = account_data.encode();
        println!("Encoded Account (ethrex_db): {}", hex::encode(&encoded_ethrex_db));
        println!("Encoded Length: {} bytes", encoded_ethrex_db.len());

        ethrex_db_trie.set_account(&address_bytes, account_data);
        let ethrex_db_state_root = ethrex_db_trie.root_hash();
        let ethrex_db_state_root_h256 = H256::from(ethrex_db_state_root);

        println!("ethrex_db State Root: {}", ethrex_db_state_root_h256);

        // === COMPARISON ===
        println!("\n--- Comparison ---");
        println!("Encodings match: {}", encoded_account == encoded_ethrex_db);
        println!("State roots match: {}", legacy_state_root == ethrex_db_state_root_h256);

        if encoded_account != encoded_ethrex_db {
            println!("\nENCODING MISMATCH!");
            println!("Legacy bytes:     {:?}", encoded_account);
            println!("ethrex_db bytes:  {:?}", encoded_ethrex_db);
        }

        if legacy_state_root != ethrex_db_state_root_h256 {
            println!("\nSTATE ROOT MISMATCH!");
            println!("This indicates a difference in how the Merkle trie is structured or computed.");
        }

        assert_eq!(encoded_account, encoded_ethrex_db, "Account encodings must match");
        assert_eq!(legacy_state_root, ethrex_db_state_root_h256, "State roots must match");
    }

    #[test]
    fn test_account_with_storage_state_root() {
        println!("\n=== Test: Account with Storage ===");

        // Create an account with storage
        let address = Address::from_str("0x2000000000000000000000000000000000000002").unwrap();
        let nonce = 5u64;
        let balance = U256::from(5000);
        let code_hash = H256::from_slice(&keccak_hash(b"some code"));

        // Storage slots
        let storage_slot_1 = H256::from_low_u64_be(1);
        let storage_value_1 = U256::from(100);
        let storage_slot_2 = H256::from_low_u64_be(2);
        let storage_value_2 = U256::from(200);

        println!("Account:");
        println!("  Address: {}", address);
        println!("  Nonce:   {}", nonce);
        println!("  Balance: {}", balance);
        println!("Storage:");
        println!("  Slot 1: {} -> {}", storage_slot_1, storage_value_1);
        println!("  Slot 2: {} -> {}", storage_slot_2, storage_value_2);

        // === COMPUTE STORAGE ROOT (shared) ===
        println!("\n--- Computing Storage Root ---");
        let mut storage_trie = StorageTrie::new();

        let slot_1_hash = H256::from(keccak_hash(&storage_slot_1.to_fixed_bytes()));
        let slot_2_hash = H256::from(keccak_hash(&storage_slot_2.to_fixed_bytes()));

        let value_1_bytes = ethrex_common::utils::u256_to_big_endian(storage_value_1);
        let value_2_bytes = ethrex_common::utils::u256_to_big_endian(storage_value_2);

        storage_trie.set(&slot_1_hash.to_fixed_bytes(), value_1_bytes);
        storage_trie.set(&slot_2_hash.to_fixed_bytes(), value_2_bytes);

        let storage_root = storage_trie.root_hash();
        let storage_root_h256 = H256::from(storage_root);

        println!("Computed Storage Root: {}", storage_root_h256);

        // === LEGACY TRIE ===
        println!("\n--- Legacy Trie ---");
        use crate::backend::in_memory::InMemoryBackend;
        use crate::trie::BackendTrieDB;
        let backend = std::sync::Arc::new(InMemoryBackend::open().unwrap());
        let trie_db = Box::new(BackendTrieDB::new_for_accounts(backend, Vec::new()).unwrap());
        let mut legacy_trie = Trie::new(trie_db);

        let account_state = AccountState {
            nonce,
            balance,
            code_hash,
            storage_root: storage_root_h256,
        };

        let hashed_address = hash_address(&address);
        let encoded_account = account_state.encode_to_vec();

        println!("Encoded Account (legacy): {}", hex::encode(&encoded_account));

        legacy_trie.insert(hashed_address.as_bytes().to_vec(), encoded_account.clone()).unwrap();
        let (legacy_state_root, _) = legacy_trie.collect_changes_since_last_hash();

        println!("Legacy State Root: {}", legacy_state_root);

        // === ETHREX_DB ===
        println!("\n--- ethrex_db ---");
        let mut ethrex_db_trie = StateTrie::new();

        let address_bytes: [u8; 20] = address.to_fixed_bytes();
        let balance_bytes: [u8; 32] = ethrex_common::utils::u256_to_big_endian(balance);

        let account_data = AccountData {
            nonce,
            balance: balance_bytes,
            storage_root,
            code_hash: code_hash.to_fixed_bytes(),
        };

        let encoded_ethrex_db = account_data.encode();
        println!("Encoded Account (ethrex_db): {}", hex::encode(&encoded_ethrex_db));

        ethrex_db_trie.set_account(&address_bytes, account_data);
        let ethrex_db_state_root = ethrex_db_trie.root_hash();
        let ethrex_db_state_root_h256 = H256::from(ethrex_db_state_root);

        println!("ethrex_db State Root: {}", ethrex_db_state_root_h256);

        // === COMPARISON ===
        println!("\n--- Comparison ---");
        println!("Encodings match: {}", encoded_account == encoded_ethrex_db);
        println!("State roots match: {}", legacy_state_root == ethrex_db_state_root_h256);

        if encoded_account != encoded_ethrex_db {
            println!("\nENCODING MISMATCH!");
        }

        if legacy_state_root != ethrex_db_state_root_h256 {
            println!("\nSTATE ROOT MISMATCH!");
        }

        assert_eq!(encoded_account, encoded_ethrex_db, "Account encodings must match");
        assert_eq!(legacy_state_root, ethrex_db_state_root_h256, "State roots must match");
    }

    #[test]
    fn test_two_accounts_state_root() {
        println!("\n=== Test: Two Accounts ===");

        // Account 1
        let address1 = Address::from_str("0x1000000000000000000000000000000000000001").unwrap();
        let nonce1 = 1u64;
        let balance1 = U256::from(1000);
        let code_hash1 = H256::from_slice(&keccak_hash(&[]));

        // Account 2
        let address2 = Address::from_str("0x2000000000000000000000000000000000000002").unwrap();
        let nonce2 = 2u64;
        let balance2 = U256::from(2000);
        let code_hash2 = H256::from_slice(&keccak_hash(b"code"));

        println!("Account 1: {}, nonce={}, balance={}", address1, nonce1, balance1);
        println!("Account 2: {}, nonce={}, balance={}", address2, nonce2, balance2);

        // === LEGACY TRIE ===
        println!("\n--- Legacy Trie ---");
        use crate::backend::in_memory::InMemoryBackend;
        use crate::trie::BackendTrieDB;
        let backend = std::sync::Arc::new(InMemoryBackend::open().unwrap());
        let trie_db = Box::new(BackendTrieDB::new_for_accounts(backend, Vec::new()).unwrap());
        let mut legacy_trie = Trie::new(trie_db);

        // Insert account 1
        let account_state1 = AccountState {
            nonce: nonce1,
            balance: balance1,
            code_hash: code_hash1,
            storage_root: *EMPTY_TRIE_HASH,
        };
        let hashed_address1 = hash_address(&address1);
        legacy_trie.insert(hashed_address1.as_bytes().to_vec(), account_state1.encode_to_vec()).unwrap();

        // Insert account 2
        let account_state2 = AccountState {
            nonce: nonce2,
            balance: balance2,
            code_hash: code_hash2,
            storage_root: *EMPTY_TRIE_HASH,
        };
        let hashed_address2 = hash_address(&address2);
        legacy_trie.insert(hashed_address2.as_bytes().to_vec(), account_state2.encode_to_vec()).unwrap();

        let (legacy_state_root, _) = legacy_trie.collect_changes_since_last_hash();
        println!("Legacy State Root: {}", legacy_state_root);

        // === ETHREX_DB ===
        println!("\n--- ethrex_db ---");
        let mut ethrex_db_trie = StateTrie::new();

        // Insert account 1
        let account_data1 = AccountData {
            nonce: nonce1,
            balance: ethrex_common::utils::u256_to_big_endian(balance1),
            storage_root: EMPTY_ROOT,
            code_hash: code_hash1.to_fixed_bytes(),
        };
        ethrex_db_trie.set_account(&address1.to_fixed_bytes(), account_data1);

        // Insert account 2
        let account_data2 = AccountData {
            nonce: nonce2,
            balance: ethrex_common::utils::u256_to_big_endian(balance2),
            storage_root: EMPTY_ROOT,
            code_hash: code_hash2.to_fixed_bytes(),
        };
        ethrex_db_trie.set_account(&address2.to_fixed_bytes(), account_data2);

        let ethrex_db_state_root = ethrex_db_trie.root_hash();
        let ethrex_db_state_root_h256 = H256::from(ethrex_db_state_root);
        println!("ethrex_db State Root: {}", ethrex_db_state_root_h256);

        // === COMPARISON ===
        println!("\n--- Comparison ---");
        println!("State roots match: {}", legacy_state_root == ethrex_db_state_root_h256);

        if legacy_state_root != ethrex_db_state_root_h256 {
            println!("\nSTATE ROOT MISMATCH!");
        }

        assert_eq!(legacy_state_root, ethrex_db_state_root_h256, "State roots must match for two accounts");
    }
}
