/// Adapter module for converting between ethrex types and ethrex_db types
///
/// This module provides type conversions and helper functions to bridge
/// between the main ethrex codebase and the vendored ethrex_db storage engine.
use crate::error::StoreError;
use ethrex_common::{
    Address, H256, U256,
    types::{AccountInfo, AccountState, AccountUpdate},
};

/// Convert ethrex H256 to ethrex_db H256
/// Both use ethereum-types::H256, so this is a no-op
#[inline]
pub fn convert_h256_to_db(hash: H256) -> H256 {
    hash
}

/// Convert ethrex_db H256 to ethrex H256
/// Both use ethereum-types::H256, so this is a no-op
#[inline]
pub fn convert_h256_from_db(hash: H256) -> H256 {
    hash
}

/// Convert ethrex Address to ethrex_db Address
/// Both use ethereum-types::Address, so this is a no-op
#[inline]
pub fn convert_address_to_db(address: Address) -> Address {
    address
}

/// Convert ethrex_db Address to ethrex Address
/// Both use ethereum-types::Address, so this is a no-op
#[inline]
pub fn convert_address_from_db(address: Address) -> Address {
    address
}

/// Convert ethrex U256 to ethrex_db U256
/// Both use ethereum-types::U256, so this is a no-op
#[inline]
pub fn convert_u256_to_db(value: U256) -> U256 {
    value
}

/// Convert ethrex_db U256 to ethrex U256
/// Both use ethereum-types::U256, so this is a no-op
#[inline]
pub fn convert_u256_from_db(value: U256) -> U256 {
    value
}

/// Convert ethrex AccountState to ethrex_db AccountData format
///
/// ethrex_db expects AccountData with fields:
/// - nonce: u64
/// - balance: U256
/// - storage_root: H256
/// - code_hash: H256
pub fn account_state_to_db_account(state: &AccountState) -> (u64, U256, H256, H256) {
    (
        state.nonce,
        state.balance,
        state.storage_root,
        state.code_hash,
    )
}

/// Convert ethrex_db account data to ethrex AccountState
pub fn db_account_to_account_state(
    nonce: u64,
    balance: U256,
    storage_root: H256,
    code_hash: H256,
) -> AccountState {
    AccountState {
        nonce,
        balance,
        storage_root,
        code_hash,
    }
}

/// Convert ethrex AccountInfo to partial account data
/// Note: AccountInfo doesn't include storage_root, only code_hash
pub fn account_info_to_partial(info: &AccountInfo) -> (u64, U256, H256) {
    (info.nonce, info.balance, info.code_hash)
}

/// Convert partial account data to ethrex AccountInfo
pub fn partial_to_account_info(nonce: u64, balance: U256, code_hash: H256) -> AccountInfo {
    AccountInfo {
        nonce,
        balance,
        code_hash,
    }
}

/// Apply an AccountUpdate to an ethrex_db block
///
/// This handles:
/// - Setting account state (nonce, balance, storage_root, code_hash)
/// - Applying storage updates (insert/delete based on value)
/// - Handling zero values as deletes
///
/// # Errors
///
/// Returns StoreError if ethrex_db operations fail
pub fn apply_account_update_to_block(
    _block: &mut (), // TODO: Replace with actual ethrex_db::chain::Block type once integrated
    _update: &AccountUpdate,
) -> Result<(), StoreError> {
    // NOTE: This function signature is a placeholder.
    // It will be properly implemented in Phase 4 when we integrate the Blockchain layer.
    //
    // Planned implementation:
    // 1. Create account from update.info (nonce, balance, code_hash, storage_root)
    // 2. Call block.set_account(update.address, account)
    // 3. For each (key, value) in update.storage:
    //    - If value.is_zero(): block.remove_storage(address, key)
    //    - Else: block.set_storage(address, key, value)
    //
    // This placeholder prevents compilation errors during incremental migration.

    Ok(())
}

/// Batch apply multiple AccountUpdates to an ethrex_db block
///
/// This is more efficient than applying updates one at a time.
///
/// # Errors
///
/// Returns StoreError if any update fails to apply
pub fn apply_account_updates_batch(
    _block: &mut (), // TODO: Replace with actual ethrex_db::chain::Block type
    _updates: &[AccountUpdate],
) -> Result<(), StoreError> {
    // NOTE: Placeholder - will be implemented in Phase 4
    // Planned implementation:
    // for update in updates {
    //     apply_account_update_to_block(block, update)?;
    // }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::H256;

    #[test]
    fn test_h256_conversion() {
        let original = H256::from([1u8; 32]);
        let converted = convert_h256_to_db(original);
        let back = convert_h256_from_db(converted);
        assert_eq!(original, back);
    }

    #[test]
    fn test_address_conversion() {
        let original = Address::from([2u8; 20]);
        let converted = convert_address_to_db(original);
        let back = convert_address_from_db(converted);
        assert_eq!(original, back);
    }

    #[test]
    fn test_u256_conversion() {
        let original = U256::from(12345u64);
        let converted = convert_u256_to_db(original);
        let back = convert_u256_from_db(converted);
        assert_eq!(original, back);
    }

    #[test]
    fn test_account_state_roundtrip() {
        let state = AccountState {
            nonce: 42,
            balance: U256::from(1000000000000000000u128),
            storage_root: H256::from([3u8; 32]),
            code_hash: H256::from([4u8; 32]),
        };

        let (nonce, balance, storage_root, code_hash) = account_state_to_db_account(&state);
        let recovered = db_account_to_account_state(nonce, balance, storage_root, code_hash);

        assert_eq!(state, recovered);
    }

    #[test]
    fn test_account_info_conversion() {
        let info = AccountInfo {
            nonce: 10,
            balance: U256::from(5000000000000000000u128),
            code_hash: H256::from([5u8; 32]),
        };

        let (nonce, balance, code_hash) = account_info_to_partial(&info);
        let recovered = partial_to_account_info(nonce, balance, code_hash);

        assert_eq!(info, recovered);
    }
}
