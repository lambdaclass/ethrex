//! Startup banner for ethrex-dev.
//!
//! Displays ASCII art and account information on startup.

use ethrex_common::{Address, U256, types::Genesis};
use ethrex_config::networks::{LOCAL_DEVNET_GENESIS_CONTENTS, LOCAL_DEVNET_PRIVATE_KEYS};

use crate::error::BlockBuilderError;

const BANNER: &str = r#"
       _____ _____ _   _ ____  _______  __
      | ____|_   _| | | |  _ \| ____\ \/ /
      |  _|   | | | |_| | |_) |  _|  \  /
      | |___  | | |  _  |  _ <| |___ /  \
      |_____| |_| |_| |_|_| \_\_____/_/\_\

            [ Development Node ]
"#;

/// Account information for display.
pub struct AccountInfo {
    pub address: Address,
    pub private_key: String,
    pub balance: U256,
}

/// Get address from a secret key.
fn get_address_from_secret_key(secret_key_bytes: &[u8]) -> Result<Address, BlockBuilderError> {
    let secret_key = secp256k1::SecretKey::from_slice(secret_key_bytes)
        .map_err(|e| BlockBuilderError::Internal(format!("Failed to parse secret key: {e}")))?;

    let public_key = secret_key
        .public_key(secp256k1::SECP256K1)
        .serialize_uncompressed();
    let hash = ethrex_common::utils::keccak(&public_key[1..]);

    // Get the last 20 bytes of the hash
    let address_bytes: [u8; 20] = hash.as_ref()[12..32]
        .try_into()
        .map_err(|e| BlockBuilderError::Internal(format!("Failed to convert address: {e}")))?;

    Ok(Address::from(address_bytes))
}

/// Load accounts from genesis and private keys.
pub fn load_accounts(count: usize) -> Result<Vec<AccountInfo>, BlockBuilderError> {
    let genesis: Genesis = serde_json::from_str(LOCAL_DEVNET_GENESIS_CONTENTS)
        .map_err(|e| BlockBuilderError::Genesis(e.to_string()))?;

    let private_keys: Vec<String> = LOCAL_DEVNET_PRIVATE_KEYS
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_string())
        .take(count)
        .collect();

    let mut accounts = Vec::with_capacity(count);

    for pk_str in private_keys {
        let pk_hex = pk_str.strip_prefix("0x").unwrap_or(&pk_str);
        let pk_bytes = hex::decode(pk_hex).map_err(|e| {
            BlockBuilderError::Internal(format!("Failed to decode private key: {e}"))
        })?;

        let address = get_address_from_secret_key(&pk_bytes)?;

        let balance = genesis
            .alloc
            .get(&address)
            .map(|acc| acc.balance)
            .unwrap_or_default();

        accounts.push(AccountInfo {
            address,
            private_key: pk_str,
            balance,
        });
    }

    Ok(accounts)
}

/// Format balance in ETH (simplified display).
fn format_eth(balance: U256) -> String {
    let wei_per_eth = U256::from(10).pow(U256::from(18));
    let eth = balance / wei_per_eth;
    format!("{}", eth)
}

/// Display the startup banner with account information.
pub fn display_banner(host: &str, port: u16) -> Result<(), BlockBuilderError> {
    let genesis: Genesis = serde_json::from_str(LOCAL_DEVNET_GENESIS_CONTENTS)
        .map_err(|e| BlockBuilderError::Genesis(e.to_string()))?;

    let accounts = load_accounts(10)?;

    // Print banner
    println!("{}", BANNER);

    // Print network info in a compact header
    println!(
        "    Network: LocalDevnet (Chain ID: {})  |  Gas Limit: {}  |  Base Fee: 1 gwei",
        genesis.config.chain_id, genesis.gas_limit
    );
    println!();

    // Print accounts (simple format without box drawing)
    println!("    Accounts:");
    println!("    ---------");
    for (i, account) in accounts.iter().enumerate() {
        println!(
            "    [{}] {:#x}  ({} ETH)",
            i,
            account.address,
            format_eth(account.balance)
        );
    }
    println!();

    // Full private keys section
    println!("    Private Keys:");
    println!("    -------------");
    for (i, account) in accounts.iter().enumerate() {
        println!("    [{}] {}", i, account.private_key);
    }
    println!();

    println!("    RPC endpoint: http://{}:{}", host, port);
    println!("    GitHub: https://github.com/lambdaclass/ethrex");
    println!();

    Ok(())
}
