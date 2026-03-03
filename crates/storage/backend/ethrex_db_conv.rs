//! Type conversions between ethrex types and ethrex-db types.
//!
//! ethrex uses `ethereum_types::{H160, H256, U256}` while ethrex-db uses
//! `primitive_types::{H256, U256}`. Both crate families wrap the same underlying
//! `uint`/`fixed-hash` types, so we convert via their `[u8; 32]` representation.

use ethrex_common::types::AccountInfo;
use ethrex_common::types::AccountState;
use ethrex_common::{Address, H256, U256};

use ethrex_db::chain::Account as DbAccount;
use ethrex_db::store::AccountData;

/// Type alias for ethrex-db's H256 to avoid ambiguity.
pub type DbH256 = primitive_types::H256;
/// Type alias for ethrex-db's U256 to avoid ambiguity.
pub type DbU256 = primitive_types::U256;

// --- Primitive type conversions (H256) ---

/// Convert ethrex H256 (`ethereum_types`) to ethrex-db H256 (`primitive_types`).
pub fn h256_to_db(h: &H256) -> DbH256 {
    primitive_types::H256(h.0)
}

/// Convert ethrex-db H256 (`primitive_types`) to ethrex H256 (`ethereum_types`).
pub fn h256_from_db(h: &DbH256) -> H256 {
    H256(h.0)
}

// --- Primitive type conversions (U256) ---

/// Convert ethrex U256 (`ethereum_types`) to ethrex-db U256 (`primitive_types`).
///
/// Uses big-endian byte serialization for safety, since the internal limb
/// layout is not guaranteed to match across crate versions.
pub fn u256_to_db(v: &U256) -> DbU256 {
    // ethereum_types::U256::to_big_endian returns [u8; 32].
    let bytes: [u8; 32] = v.to_big_endian();
    DbU256::from_big_endian(&bytes)
}

/// Convert ethrex-db U256 (`primitive_types`) to ethrex U256 (`ethereum_types`).
pub fn u256_from_db(v: &DbU256) -> U256 {
    // primitive_types::U256::to_big_endian returns [u8; 32].
    let bytes: [u8; 32] = v.to_big_endian();
    U256::from_big_endian(&bytes)
}

// --- Address conversions ---

/// Convert an Address (H160) to an H256 by left-zero-padding to 32 bytes.
pub fn address_to_h256(addr: &Address) -> DbH256 {
    let mut bytes = [0u8; 32];
    bytes[12..32].copy_from_slice(addr.as_bytes());
    primitive_types::H256(bytes)
}

/// Convert an ethrex Address (H160) to an ethrex-db H256 by keccak-hashing.
///
/// ethrex-db addresses accounts by `keccak256(address)`, matching Ethereum's
/// state trie key derivation.
pub fn address_to_db_key(addr: &Address) -> DbH256 {
    let hash = ethrex_common::utils::keccak(addr.as_bytes());
    h256_to_db(&hash)
}

// --- Account conversions ---

/// Convert an ethrex `AccountState` to an ethrex-db `Account`.
pub fn account_state_to_db(state: &AccountState) -> DbAccount {
    DbAccount {
        nonce: state.nonce,
        balance: u256_to_db(&state.balance),
        code_hash: h256_to_db(&state.code_hash),
        storage_root: h256_to_db(&state.storage_root),
    }
}

/// Convert an ethrex-db `Account` to an ethrex `AccountState`.
pub fn account_state_from_db(account: &DbAccount) -> AccountState {
    AccountState {
        nonce: account.nonce,
        balance: u256_from_db(&account.balance),
        code_hash: h256_from_db(&account.code_hash),
        storage_root: h256_from_db(&account.storage_root),
    }
}

/// Convert an ethrex `AccountInfo` (which lacks `storage_root`) to an ethrex-db
/// `Account`, using the provided `storage_root`.
pub fn account_info_to_db(info: &AccountInfo, storage_root: H256) -> DbAccount {
    DbAccount {
        nonce: info.nonce,
        balance: u256_to_db(&info.balance),
        code_hash: h256_to_db(&info.code_hash),
        storage_root: h256_to_db(&storage_root),
    }
}

/// Convert an ethrex-db `Account` to an ethrex `AccountInfo`.
///
/// The `storage_root` field is dropped since `AccountInfo` does not carry it.
pub fn account_info_from_db(account: &DbAccount) -> AccountInfo {
    AccountInfo {
        nonce: account.nonce,
        balance: u256_from_db(&account.balance),
        code_hash: h256_from_db(&account.code_hash),
    }
}

// --- AccountData conversions (raw byte representation used in trie storage) ---

/// Convert an ethrex `AccountState` to an ethrex-db `AccountData`.
///
/// `AccountData` stores balance, storage_root, and code_hash as raw `[u8; 32]`
/// arrays (big-endian for balance).
pub fn account_state_to_data(state: &AccountState) -> AccountData {
    AccountData {
        nonce: state.nonce,
        balance: state.balance.to_big_endian(),
        storage_root: state.storage_root.0,
        code_hash: state.code_hash.0,
    }
}

/// Convert an ethrex-db `AccountData` to an ethrex `AccountState`.
pub fn data_to_account_state(data: &AccountData) -> AccountState {
    AccountState {
        nonce: data.nonce,
        balance: U256::from_big_endian(&data.balance),
        storage_root: H256(data.storage_root),
        code_hash: H256(data.code_hash),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn h256_round_trip() {
        let original = H256::from_str(
            "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470",
        )
        .expect("valid hex");
        let db = h256_to_db(&original);
        let back = h256_from_db(&db);
        assert_eq!(original, back);
    }

    #[test]
    fn h256_zero_round_trip() {
        let original = H256::zero();
        let db = h256_to_db(&original);
        let back = h256_from_db(&db);
        assert_eq!(original, back);
    }

    #[test]
    fn u256_round_trip() {
        let original = U256::from(123_456_789u64);
        let db = u256_to_db(&original);
        let back = u256_from_db(&db);
        assert_eq!(original, back);
    }

    #[test]
    fn u256_zero_round_trip() {
        let original = U256::zero();
        let db = u256_to_db(&original);
        let back = u256_from_db(&db);
        assert_eq!(original, back);
    }

    #[test]
    fn u256_max_round_trip() {
        let original = U256::MAX;
        let db = u256_to_db(&original);
        let back = u256_from_db(&db);
        assert_eq!(original, back);
    }

    #[test]
    fn u256_large_value_round_trip() {
        // A value that exercises all 4 limbs.
        let original = U256::from_big_endian(&[
            0xFF, 0x00, 0xAB, 0xCD, 0x12, 0x34, 0x56, 0x78,
            0x9A, 0xBC, 0xDE, 0xF0, 0x11, 0x22, 0x33, 0x44,
            0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC,
            0xDD, 0xEE, 0xFF, 0x00, 0x01, 0x02, 0x03, 0x04,
        ]);
        let db = u256_to_db(&original);
        let back = u256_from_db(&db);
        assert_eq!(original, back);
    }

    #[test]
    fn account_state_round_trip() {
        let state = AccountState {
            nonce: 42,
            balance: U256::from(1_000_000_000u64),
            storage_root: H256::from_str(
                "56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
            )
            .expect("valid hex"),
            code_hash: H256::from_str(
                "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470",
            )
            .expect("valid hex"),
        };

        let db_account = account_state_to_db(&state);
        let back = account_state_from_db(&db_account);
        assert_eq!(state, back);
    }

    #[test]
    fn account_info_round_trip() {
        let info = AccountInfo {
            nonce: 7,
            balance: U256::from(500u64),
            code_hash: H256::from_str(
                "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470",
            )
            .expect("valid hex"),
        };

        let storage_root = H256::from_str(
            "56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
        )
        .expect("valid hex");

        let db_account = account_info_to_db(&info, storage_root);
        let back = account_info_from_db(&db_account);
        assert_eq!(info, back);
    }

    #[test]
    fn account_info_to_db_preserves_storage_root() {
        let info = AccountInfo {
            nonce: 1,
            balance: U256::from(100u64),
            code_hash: H256::zero(),
        };
        let storage_root = H256::from_str(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .expect("valid hex");

        let db_account = account_info_to_db(&info, storage_root);
        assert_eq!(h256_from_db(&db_account.storage_root), storage_root);
    }

    #[test]
    fn address_to_db_key_is_keccak() {
        // Verify that the address-to-key conversion matches keccak256 hashing.
        let addr =
            Address::from_str("0000000000000000000000000000000000000001").expect("valid address");
        let key = address_to_db_key(&addr);
        let expected = ethrex_common::utils::keccak(addr.as_bytes());
        assert_eq!(key.0, expected.0);
    }

    #[test]
    fn default_account_info_round_trip() {
        let info = AccountInfo::default();
        let storage_root = H256::zero();
        let db_account = account_info_to_db(&info, storage_root);
        let back = account_info_from_db(&db_account);
        assert_eq!(info, back);
    }

    #[test]
    fn address_to_h256_zero_pads() {
        let addr =
            Address::from_str("0000000000000000000000000000000000000001").expect("valid address");
        let h = address_to_h256(&addr);
        // First 12 bytes should be zero, last 20 bytes are the address.
        assert_eq!(&h.0[..12], &[0u8; 12]);
        assert_eq!(&h.0[12..], addr.as_bytes());
    }

    #[test]
    fn address_to_h256_known_address() {
        let addr =
            Address::from_str("d8dA6BF26964aF9D7eEd9e03E53415D37aA96045").expect("valid address");
        let h = address_to_h256(&addr);
        assert_eq!(&h.0[..12], &[0u8; 12]);
        assert_eq!(&h.0[12..], addr.as_bytes());
    }

    #[test]
    fn account_state_to_data_round_trip() {
        let state = AccountState {
            nonce: 99,
            balance: U256::from(5_000_000_000u64),
            storage_root: H256::from_str(
                "56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
            )
            .expect("valid hex"),
            code_hash: H256::from_str(
                "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470",
            )
            .expect("valid hex"),
        };

        let data = account_state_to_data(&state);
        let back = data_to_account_state(&data);
        assert_eq!(state, back);
    }

    #[test]
    fn account_data_zero_balance() {
        let state = AccountState {
            nonce: 0,
            balance: U256::zero(),
            storage_root: H256::zero(),
            code_hash: H256::zero(),
        };
        let data = account_state_to_data(&state);
        assert_eq!(data.balance, [0u8; 32]);
        let back = data_to_account_state(&data);
        assert_eq!(state, back);
    }
}
