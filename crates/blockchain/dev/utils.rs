use std::collections::HashMap;

use ethereum_types::{H160, U256};
use ethrex_common::types::Genesis;
use ethrex_l2_sdk::get_address_from_secret_key;
use keccak_hash::H256;
use secp256k1::SecretKey;

pub const NUMBER_OF_TOP_ACCOUNTS: usize = 10;

pub fn show_rich_accounts(genesis: &Genesis, contents: &str) {
    let private_keys: Vec<String> = contents
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_string())
        .collect();

    let mut address_to_pk = HashMap::new();
    for pk in private_keys.iter() {
        let pk_str = pk.strip_prefix("0x").unwrap_or(pk);
        let Ok(pk_h256) = pk_str.parse::<H256>() else {
            return;
        };
        let pk_bytes = pk_h256.as_bytes();
        let Ok(secret_key) = SecretKey::from_slice(pk_bytes) else {
            return;
        };
        let Ok(address) = get_address_from_secret_key(&secret_key) else {
            return;
        };
        address_to_pk.insert(address, pk);
    }

    let mut top_accounts: Vec<(&H160, U256)> = genesis
        .alloc
        .iter()
        .map(|(address, account)| (address, account.balance))
        .collect();
    top_accounts.sort_by(|a, b| b.1.cmp(&a.1)); // sort by greater balance
    top_accounts.truncate(NUMBER_OF_TOP_ACCOUNTS);

    println!("Showing first {} accounts", NUMBER_OF_TOP_ACCOUNTS);
    println!("-------------------------------------------------------------------------------");
    for (address, balance) in top_accounts {
        let Some(pk) = address_to_pk.get(address) else {
            continue;
        };
        println!("Private Key: {}", pk);
        println!("Address:     {:?} (Ξ {})", address, wei_to_eth(balance));
        println!("-------------------------------------------------------------------------------");
    }
}

pub fn wei_to_eth(wei: U256) -> U256 {
    wei.checked_div(U256::from_dec_str("1000000000000000000").unwrap())
        .unwrap_or(U256::zero())
}
