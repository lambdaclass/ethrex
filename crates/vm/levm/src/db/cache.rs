use crate::Account;
use ethrex_core::Address;
use std::collections::HashMap;

pub type CacheDB = HashMap<Address, Account>;

pub fn get_account<'cache>(
    cached_accounts: &'cache CacheDB,
    address: &Address,
) -> Option<&'cache Account> {
    cached_accounts.get(address)
}

pub fn get_account_mut<'cache>(
    cached_accounts: &'cache mut CacheDB,
    address: &Address,
) -> Option<&'cache mut Account> {
    cached_accounts.get_mut(address)
}

pub fn insert_account(
    cached_accounts: &mut CacheDB,
    address: Address,
    account: Account,
) -> Option<Account> {
    cached_accounts.insert(address, account)
}

pub fn remove_account(cached_accounts: &mut CacheDB, address: &Address) -> Option<Account> {
    cached_accounts.remove(address)
}

pub fn is_account_cached(cached_accounts: &CacheDB, address: &Address) -> bool {
    cached_accounts.contains_key(address)
}
