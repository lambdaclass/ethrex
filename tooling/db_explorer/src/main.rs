use std::path::PathBuf;

use clap::{Parser, Subcommand};
use ethrex_db_explorer::DbExplorer;

#[derive(Parser)]
#[command(name = "db-explorer", about = "Explore an ethrex mainnet database")]
struct Cli {
    /// Path to the ethrex data directory
    #[arg(long, env = "ETHREX_DATADIR")]
    datadir: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show database info (latest block, earliest block, chain id)
    Info,

    /// Show a block header
    Header {
        /// Block number
        number: u64,
    },

    /// Show transaction statistics for a block or range
    TxStats {
        /// Start block number
        from: u64,
        /// End block number (inclusive). If omitted, only `from` is analyzed.
        to: Option<u64>,
    },

    /// Count transactions of each type in a range
    TxTypes {
        /// Start block number
        from: u64,
        /// End block number (inclusive)
        to: u64,
    },

    /// Show account state at a given block
    Account {
        /// Account address (0x-prefixed hex)
        address: String,
        /// Block number
        #[arg(long)]
        block: u64,
    },

    /// Compute state statistics (account count, storage slot distribution)
    StateStats {
        /// Block number (defaults to latest)
        #[arg(long)]
        block: Option<u64>,
        /// Stop after this many accounts (for sampling)
        #[arg(long)]
        max_accounts: Option<usize>,
        /// Print progress every N accounts
        #[arg(long, default_value = "100000")]
        report_every: usize,
        /// Count storage slots per account (very slow — prefix-scans the storage FKV)
        #[arg(long, default_value = "false")]
        count_slots: bool,
    },

    /// Diagnose state trie availability for a block
    DiagState {
        /// Block number (defaults to latest)
        #[arg(long)]
        block: Option<u64>,
    },

    /// Diagnose storage FKV table: peek at raw keys
    DiagStorage {
        /// Optional: account address to check (0x-prefixed hex).
        /// If omitted, shows first entries from the table.
        address: Option<String>,
    },

    /// Count storage slots for a specific account
    SlotCount {
        /// Account address (0x-prefixed hex)
        address: String,
        /// Block number (defaults to latest)
        #[arg(long)]
        block: Option<u64>,
    },
}

fn main() {
    let cli = Cli::parse();

    let db = match DbExplorer::open(&cli.datadir) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Failed to open database at {:?}: {e}", cli.datadir);
            std::process::exit(1);
        }
    };

    match cli.command {
        Command::Info => {
            let latest = db.latest_block_number().expect("Failed to get latest block");
            let earliest = db
                .earliest_block_number()
                .expect("Failed to get earliest block");
            println!("Database: {:?}", cli.datadir);
            println!("Earliest block: {earliest}");
            println!("Latest block:   {latest}");
        }
        Command::Header { number } => match db.header(number) {
            Ok(Some(header)) => {
                println!("{}", serde_json::to_string_pretty(&header).expect("Failed to serialize header"));
            }
            Ok(None) => {
                eprintln!("Block {number} not found");
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        },
        Command::TxStats { from, to } => {
            let end = to.unwrap_or(from);
            if from == end {
                match db.block_tx_stats(from) {
                    Ok(Some(stats)) => print!("{stats}"),
                    Ok(None) => {
                        eprintln!("Block {from} not found");
                        std::process::exit(1);
                    }
                    Err(e) => {
                        eprintln!("Error: {e}");
                        std::process::exit(1);
                    }
                }
            } else {
                match db.range_tx_stats(from..=end) {
                    Ok(stats) => print!("{stats}"),
                    Err(e) => {
                        eprintln!("Error: {e}");
                        std::process::exit(1);
                    }
                }
            }
        }
        Command::TxTypes { from, to } => match db.range_tx_stats(from..=to) {
            Ok(stats) => {
                println!("Transaction type breakdown ({from}..={to}):");
                println!("  Legacy:  {}", stats.legacy_count);
                println!("  EIP2930: {}", stats.eip2930_count);
                println!("  EIP1559: {}", stats.eip1559_count);
                println!("  EIP4844: {}", stats.eip4844_count);
                println!("  EIP7702: {}", stats.eip7702_count);
                println!("  Other:   {}", stats.other_count);
                println!("  Total:   {}", stats.total_txs);
            }
            Err(e) => {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        },
        Command::Account { address, block } => {
            let address = address
                .parse()
                .expect("Invalid address format (expected 0x-prefixed hex)");
            match db.account_state(block, address) {
                Ok(Some(state)) => {
                    println!("Account {address} at block {block}:");
                    println!("  Nonce:        {}", state.nonce);
                    println!("  Balance:      {}", state.balance);
                    println!("  Storage root: {:?}", state.storage_root);
                    println!("  Code hash:    {:?}", state.code_hash);
                }
                Ok(None) => {
                    eprintln!("Account not found at block {block}");
                    std::process::exit(1);
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }
        Command::StateStats {
            block,
            max_accounts,
            report_every,
            count_slots,
        } => {
            let block_number = block.unwrap_or_else(|| {
                db.latest_block_number().expect("Failed to get latest block")
            });
            println!(
                "Computing state stats at block {block_number}{}...",
                if count_slots { " (with slot counting)" } else { "" }
            );
            match db.state_stats(block_number, count_slots, max_accounts, report_every, |stats| {
                eprint!(
                    "\r  {} accounts scanned, {} slots so far...",
                    stats.total_accounts, stats.total_slots
                );
            }) {
                Ok(stats) => {
                    eprintln!(); // newline after progress
                    print!("{stats}");
                }
                Err(e) => {
                    eprintln!("\nError: {e}");
                    std::process::exit(1);
                }
            }
        }
        Command::DiagState { block } => {
            let block_number = block.unwrap_or_else(|| {
                db.latest_block_number().expect("Failed to get latest block")
            });
            let header = db
                .header(block_number)
                .expect("Failed to get header")
                .expect("Block not found");
            let state_root = header.state_root;
            println!("Block:      {block_number}");
            println!("State root: {state_root:?}");

            match db.has_state_root(state_root) {
                Ok(true) => println!("has_state_root: YES"),
                Ok(false) => println!("has_state_root: NO"),
                Err(e) => println!("has_state_root: ERROR ({e})"),
            }

            // Try iterating with the FKV table (first 5 accounts)
            match db.store().iter_fkv_accounts() {
                Ok(fkv) => {
                    let mut count = 0;
                    let _ = fkv.for_each(|addr, state| {
                        count += 1;
                        println!(
                            "  FKV account #{count}: {:?} nonce={} balance={}",
                            addr, state.nonce, state.balance
                        );
                        count < 5
                    });
                    println!("iter_fkv_accounts: got {count} accounts (limited to 5)");
                }
                Err(e) => println!("iter_fkv_accounts: ERROR ({e})"),
            }

            // Try a direct account lookup to see if state works at all
            // Use address 0x0000...0000 as a simple test
            match db.account_state(block_number, ethereum_types::Address::zero()) {
                Ok(Some(state)) => println!(
                    "account_state(0x0): nonce={} balance={}",
                    state.nonce, state.balance
                ),
                Ok(None) => println!("account_state(0x0): not found"),
                Err(e) => println!("account_state(0x0): ERROR ({e})"),
            }
        }
        Command::DiagStorage { address } => {
            use ethrex_storage::api::tables::{ACCOUNT_FLATKEYVALUE, STORAGE_FLATKEYVALUE};

            // First, peek at raw STORAGE_FLATKEYVALUE entries
            let prefix = if let Some(addr_str) = &address {
                let addr: ethereum_types::Address = addr_str
                    .parse()
                    .expect("Invalid address format");
                let hashed = ethrex_storage::hash_address(&addr);
                let mut p = Vec::with_capacity(65);
                for byte in &hashed {
                    p.push(byte >> 4);
                    p.push(byte & 0x0f);
                }
                p.push(17); // separator
                println!("Looking up storage for address {addr}");
                println!("Hashed address: {:?}", ethereum_types::H256::from_slice(&hashed));
                println!("Prefix ({} bytes): {:?}", p.len(), &p[..std::cmp::min(10, p.len())]);
                p
            } else {
                println!("Peeking at first entries in STORAGE_FLATKEYVALUE...");
                vec![]
            };

            match db.store().peek_table(STORAGE_FLATKEYVALUE, &prefix, 5) {
                Ok(entries) => {
                    println!("STORAGE_FLATKEYVALUE: {} entries found (showing up to 5)", entries.len());
                    for (i, (key, vlen)) in entries.iter().enumerate() {
                        println!(
                            "  #{}: key_len={}, value_len={}, key_prefix={:?}...",
                            i + 1,
                            key.len(),
                            vlen,
                            &key[..std::cmp::min(20, key.len())]
                        );
                    }
                }
                Err(e) => println!("ERROR: {e}"),
            }

            // Also peek at ACCOUNT_FLATKEYVALUE for comparison
            println!("\nPeeking at first 3 entries in ACCOUNT_FLATKEYVALUE...");
            match db.store().peek_table(ACCOUNT_FLATKEYVALUE, &[], 3) {
                Ok(entries) => {
                    println!("ACCOUNT_FLATKEYVALUE: {} entries found", entries.len());
                    for (i, (key, vlen)) in entries.iter().enumerate() {
                        println!(
                            "  #{}: key_len={}, value_len={}, key_prefix={:?}...",
                            i + 1,
                            key.len(),
                            vlen,
                            &key[..std::cmp::min(20, key.len())]
                        );
                    }
                }
                Err(e) => println!("ERROR: {e}"),
            }
        }
        Command::SlotCount { address, block } => {
            let block_number = block.unwrap_or_else(|| {
                db.latest_block_number().expect("Failed to get latest block")
            });
            let address: ethereum_types::Address = address
                .parse()
                .expect("Invalid address format (expected 0x-prefixed hex)");
            let hashed = ethereum_types::H256::from_slice(&ethrex_storage::hash_address(&address));
            match db.storage_slot_count(hashed) {
                Ok(count) => {
                    println!("Account {address} at block {block_number}: {count} storage slots");
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }
    }
}
