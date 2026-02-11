//! Startup banner for `ethrex --dev`.

use ethrex_common::{Address, U256, types::Genesis};
use ethrex_config::networks::{LOCAL_DEVNET_GENESIS_CONTENTS, LOCAL_DEVNET_PRIVATE_KEYS};

use crate::error::BlockBuilderError;

const WEI_PER_ETH: u64 = 1_000_000_000_000_000_000;

// ANSI color codes
const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const RED: &str = "\x1b[91m";
const ORANGE: &str = "\x1b[38;5;208m";
const YELLOW: &str = "\x1b[93m";
const LIGHT_BLUE: &str = "\x1b[94m";
const BLUE: &str = "\x1b[34m";

struct AccountInfo {
    address: Address,
    private_key: String,
    balance: U256,
}

fn address_from_secret_key(secret_key_bytes: &[u8]) -> Result<Address, BlockBuilderError> {
    let secret_key = secp256k1::SecretKey::from_slice(secret_key_bytes)
        .map_err(|e| BlockBuilderError::Internal(format!("Failed to parse secret key: {e}")))?;

    let public_key = secret_key
        .public_key(secp256k1::SECP256K1)
        .serialize_uncompressed();
    let hash = ethrex_common::utils::keccak(&public_key[1..]);

    let address_bytes: [u8; 20] = hash.as_ref()[12..32]
        .try_into()
        .map_err(|e| BlockBuilderError::Internal(format!("Failed to convert address: {e}")))?;

    Ok(Address::from(address_bytes))
}

fn load_accounts(genesis: &Genesis, count: usize) -> Result<Vec<AccountInfo>, BlockBuilderError> {
    LOCAL_DEVNET_PRIVATE_KEYS
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_string())
        .take(count)
        .map(|pk_str| {
            let pk_hex = pk_str.strip_prefix("0x").unwrap_or(&pk_str);
            let pk_bytes = hex::decode(pk_hex).map_err(|e| {
                BlockBuilderError::Internal(format!("Failed to decode private key: {e}"))
            })?;

            let address = address_from_secret_key(&pk_bytes)?;
            let balance = genesis
                .alloc
                .get(&address)
                .map(|acc| acc.balance)
                .unwrap_or_default();

            Ok(AccountInfo {
                address,
                private_key: pk_str,
                balance,
            })
        })
        .collect()
}

/// Display the startup banner with account information.
pub fn display_banner(host: &str, port: u16, use_color: bool) -> Result<(), BlockBuilderError> {
    let genesis: Genesis = serde_json::from_str(LOCAL_DEVNET_GENESIS_CONTENTS)
        .map_err(|e| BlockBuilderError::Genesis(e.to_string()))?;

    let accounts = load_accounts(&genesis, 10)?;

    if use_color {
        print_colored_banner(&genesis, &accounts, host, port);
    } else {
        print_plain_banner(&genesis, &accounts, host, port);
    }

    Ok(())
}

fn print_colored_banner(genesis: &Genesis, accounts: &[AccountInfo], host: &str, port: u16) {
    // ASCII logo
    println!();
    println!("  {RED}{BOLD} _____ _____ _   _ ____  _______  __{RESET}");
    println!("  {RED}{BOLD}| ____|_   _| | | |  _ \\| ____\\ \\/ /{RESET}");
    println!("  {RED}{BOLD}|  _|   | | | |_| | |_) |  _|  \\  /{RESET}");
    println!("  {RED}{BOLD}| |___  | | |  _  |  _ <| |___ /  \\{RESET}");
    println!("  {RED}{BOLD}|_____| |_| |_| |_|_| \\_\\_____/_/\\_\\{RESET}");
    println!();
    println!("          {ORANGE}{BOLD}Development Node{RESET}");
    println!();

    // Info panel
    let url = format!("http://{host}:{port}");
    let info_line = format!(
        "  {BOLD}Chain ID:{RESET} {}  {BLUE}\u{00b7}{RESET}  {BOLD}Gas Limit:{RESET} {}  {BLUE}\u{00b7}{RESET}  {BOLD}Base Fee:{RESET} 1 gwei",
        genesis.config.chain_id, genesis.gas_limit
    );
    let url_line = format!("  Listening on {LIGHT_BLUE}{BOLD}{url}{RESET}");

    let box_width = 64;
    let rule: String = "\u{2500}".repeat(box_width);

    println!("{BLUE}\u{256d}{rule}\u{256e}{RESET}");
    println!("{BLUE}\u{2502}{RESET}{info_line}");
    println!("{BLUE}\u{2502}{RESET}{url_line}");
    println!("{BLUE}\u{2570}{rule}\u{256f}{RESET}");
    println!();

    // Accounts section
    let section_rule: String = "\u{2500}".repeat(box_width - 11);
    println!("{BLUE}\u{2500}\u{2500} {RESET}{BOLD}Accounts{RESET} {BLUE}{section_rule}{RESET}");
    println!();

    for (i, account) in accounts.iter().enumerate() {
        let eth = account.balance / U256::from(WEI_PER_ETH);
        println!(
            "({i}) {LIGHT_BLUE}{:#x}{RESET} ({eth} ETH)",
            account.address
        );
        println!("    {YELLOW}{}{RESET}", account.private_key);
        if i < accounts.len() - 1 {
            println!();
        }
    }

    println!();
    println!(
        "{YELLOW}{BOLD}\u{26a0} These accounts and keys are publicly known. Do not use on mainnet.{RESET}"
    );
    println!();
}

fn print_plain_banner(genesis: &Genesis, accounts: &[AccountInfo], host: &str, port: u16) {
    let box_width = 64;

    // ASCII logo
    println!();
    println!("   _____ _____ _   _ ____  _______  __");
    println!("  | ____|_   _| | | |  _ \\| ____\\ \\/ /");
    println!("  |  _|   | | | |_| | |_) |  _|  \\  /");
    println!("  | |___  | | |  _  |  _ <| |___ /  \\");
    println!("  |_____| |_| |_| |_|_| \\_\\_____/_/\\_\\");
    println!();
    println!("          Development Node");
    println!();

    // Info panel
    let url = format!("http://{host}:{port}");
    let rule: String = "─".repeat(box_width);
    println!("╭{rule}╮");
    println!(
        "│  Chain ID: {}  ·  Gas Limit: {}  ·  Base Fee: 1 gwei",
        genesis.config.chain_id, genesis.gas_limit
    );
    println!("│  Listening on {url}");
    println!("╰{rule}╯");
    println!();

    // Accounts section
    let section_rule: String = "─".repeat(box_width - 11);
    println!("── Accounts {section_rule}");
    println!();

    for (i, account) in accounts.iter().enumerate() {
        let eth = account.balance / U256::from(WEI_PER_ETH);
        println!("({i}) {:#x} ({eth} ETH)", account.address);
        println!("    {}", account.private_key);
        if i < accounts.len() - 1 {
            println!();
        }
    }

    println!();
    println!("⚠ These accounts and keys are publicly known. Do not use on mainnet.");
    println!();
}
