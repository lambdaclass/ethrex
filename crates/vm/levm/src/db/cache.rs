use crate::{call_frame::CallFrame, Account};
use ethrex_common::Address;
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
    call_frame: &mut Option<&mut CallFrame>,
) -> Option<&'cache mut Account> {
    let account_option = cached_accounts.get_mut(address);

    // insert account_option cloned into call_frame backup if not already there
    if let Some(call_frame) = call_frame {
        if !call_frame.backup.contains_key(address) {
            if let Some(account) = account_option.as_ref() {
                call_frame.backup.insert(*address, Some((*account).clone()));
            } else {
                call_frame.backup.insert(*address, None);
            }
        }
    }

    account_option
}

pub fn insert_account(
    cached_accounts: &mut CacheDB,
    address: Address,
    account: Account,
) -> Option<Account> {
    cached_accounts.insert(address, account.clone())
}

pub fn remove_account(
    cached_accounts: &mut CacheDB,
    address: &Address,
    call_frame: &mut Option<&mut CallFrame>,
) -> Option<Account> {
    let account_option = cached_accounts.remove(address);

    // insert account_option cloned into call_frame backup if not already there
    if let Some(call_frame) = call_frame {
        if !call_frame.backup.contains_key(address) {
            if let Some(account) = account_option.as_ref() {
                call_frame.backup.insert(*address, Some((*account).clone()));
            } else {
                call_frame.backup.insert(*address, None);
            }
        }
    }

    account_option
}

pub fn is_account_cached(cached_accounts: &CacheDB, address: &Address) -> bool {
    cached_accounts.contains_key(address)
}
