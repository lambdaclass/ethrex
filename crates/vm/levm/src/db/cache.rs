use ethrex_common::{types::Account, Address};
use std::{collections::HashMap, sync::Arc};

pub type CacheDB = HashMap<Address, Arc<Account>>;

pub fn get_account(
    cached_accounts: &CacheDB,
    address: &Address,
) -> Option<Arc<Account>> {
    cached_accounts.get(address).cloned()
}

pub fn get_account_mut<'cache>(
    cached_accounts: &'cache mut CacheDB,
    address: &Address,
) -> Option<&'cache mut Account> {
    cached_accounts.get_mut(address).and_then(Arc::get_mut)
}

/// Inserts an account (which will be wrapped in an Arc) into the cache.
/// Returns the previous Arc<Account> if one existed for this address.
pub fn insert_account(
    cached_accounts: &mut CacheDB,
    address: Address,
    account: Account,
) -> Option<Arc<Account>> {
    cached_accounts.insert(address, Arc::new(account))
}

/// Inserts an Arc<Account> directly into the cache.
/// Returns the previous Arc<Account> if one existed for this address.
pub fn insert_arc_account(
    cached_accounts: &mut CacheDB,
    address: Address,
    account_arc: Arc<Account>,
) -> Option<Arc<Account>> {
    cached_accounts.insert(address, account_arc)
}

pub fn remove_account(cached_accounts: &mut CacheDB, address: &Address) -> Option<Arc<Account>> {
    cached_accounts.remove(address)
}

pub fn is_account_cached(cached_accounts: &CacheDB, address: &Address) -> bool {
    cached_accounts.contains_key(address)
}

/// Gets a mutable reference to an Account from the cache, performing copy-on-write if necessary using `Arc::make_mut`.
/// Returns `None` if the address is not found in the cache.
pub fn get_or_make_mut_account<'cache>(
    cached_accounts: &'cache mut CacheDB,
    address: &Address,
) -> Option<&'cache mut Account> {
    cached_accounts.get_mut(address).map(|arc_instance_in_map| {
        // Arc::make_mut will:
        // If strong_count == 1, return a mutable ref to the existing data.
        // If strong_count > 1, clone Account data, create a new Arc with it
        Arc::make_mut(arc_instance_in_map)
    })
}
