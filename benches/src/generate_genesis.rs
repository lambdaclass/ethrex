//! Genesis Generator
//!
//! Generates a genesis.json file with a large number of accounts (default: 1M)
//! and saves the corresponding private keys to a text file.
//!
//! Usage:
//!   cargo run -p ethrex-benches --bin generate_genesis --release -- [OPTIONS]
//!
//! Options:
//!   --accounts <N>       Number of accounts to generate (default: 1000000)
//!   --balance <WEI>      Balance per account in wei (default: 10 ETH)
//!   --output <PATH>      Output directory (default: current directory)
//!   --chain-id <ID>      Chain ID (default: 1337)

use bytes::Bytes;
use ethereum_types::{H160, H256, U256};
use ethrex_common::types::{ChainConfig, Genesis, GenesisAccount};
use ethrex_crypto::keccak::keccak_hash;
use ethrex_l2_rpc::signer::{LocalSigner, Signer};
use secp256k1::SecretKey;
use serde_json;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Instant;

/// Hardcoded seed for reproducible key generation
const SEED: &[u8; 32] = b"ethrex_genesis_generator_seed!X!";

/// Default number of accounts
const DEFAULT_NUM_ACCOUNTS: usize = 1_000_000;

/// Default balance per account (10 ETH in wei)
const DEFAULT_BALANCE: &str = "8ac7230489e80000";

/// Default chain ID
const DEFAULT_CHAIN_ID: u64 = 1337;

/// Generate a deterministic private key from an index
fn generate_private_key(index: usize) -> SecretKey {
    let mut data = [0u8; 32];
    data[..32].copy_from_slice(SEED);

    // Mix in the index
    let index_bytes = (index as u64).to_le_bytes();
    for (i, b) in index_bytes.iter().enumerate() {
        data[24 + i] ^= b;
    }

    // Hash to get the actual key
    let hash = keccak_hash(&data);
    SecretKey::from_slice(&hash).expect("valid secret key")
}

/// Derive address from private key
fn derive_address(sk: &SecretKey) -> H160 {
    let signer = Signer::Local(LocalSigner::new(*sk));
    signer.address()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Parse arguments
    let mut num_accounts = DEFAULT_NUM_ACCOUNTS;
    let mut balance_hex = DEFAULT_BALANCE.to_string();
    let mut output_dir = PathBuf::from(".");
    let mut chain_id = DEFAULT_CHAIN_ID;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--accounts" => {
                i += 1;
                num_accounts = args[i].parse().expect("Invalid accounts number");
            }
            "--balance" => {
                i += 1;
                balance_hex = args[i].clone();
            }
            "--output" => {
                i += 1;
                output_dir = PathBuf::from(&args[i]);
            }
            "--chain-id" => {
                i += 1;
                chain_id = args[i].parse().expect("Invalid chain ID");
            }
            "--help" | "-h" => {
                println!("Genesis Generator - Creates genesis.json with many accounts");
                println!();
                println!("Usage: generate_genesis [OPTIONS]");
                println!();
                println!("Options:");
                println!("  --accounts <N>    Number of accounts (default: 1000000)");
                println!("  --balance <HEX>   Balance in wei as hex (default: 8ac7230489e80000 = 10 ETH)");
                println!("  --output <PATH>   Output directory (default: .)");
                println!("  --chain-id <ID>   Chain ID (default: 1337)");
                println!("  --help            Show this help");
                return;
            }
            _ => {
                eprintln!("Unknown argument: {}", args[i]);
                std::process::exit(1);
            }
        }
        i += 1;
    }

    println!("=== Genesis Generator ===\n");
    println!("Configuration:");
    println!("  Accounts: {}", num_accounts);
    println!("  Balance: 0x{} wei", balance_hex);
    println!("  Chain ID: {}", chain_id);
    println!("  Output: {}", output_dir.display());
    println!();

    // Create output directory if needed
    std::fs::create_dir_all(&output_dir).expect("Failed to create output directory");

    let genesis_path = output_dir.join("genesis.json");
    let keys_path = output_dir.join("private_keys.txt");

    // Generate accounts
    println!("Generating {} accounts...", num_accounts);
    let start = Instant::now();

    let balance = U256::from_str_radix(&balance_hex, 16).expect("Invalid balance hex");
    let mut alloc: BTreeMap<H160, GenesisAccount> = BTreeMap::new();

    // Open keys file for writing
    let keys_file = File::create(&keys_path).expect("Failed to create keys file");
    let mut keys_writer = BufWriter::new(keys_file);

    // Progress tracking
    let progress_interval = num_accounts / 10;

    for i in 0..num_accounts {
        let sk = generate_private_key(i);
        let addr = derive_address(&sk);

        // Write private key to file
        writeln!(keys_writer, "0x{}", hex::encode(sk.secret_bytes())).expect("Failed to write key");

        // Add to alloc
        alloc.insert(
            addr,
            GenesisAccount {
                code: Bytes::new(),
                storage: BTreeMap::new(),
                balance,
                nonce: 0,
            },
        );

        // Progress update
        if progress_interval > 0 && (i + 1) % progress_interval == 0 {
            println!("  Progress: {}%", ((i + 1) * 100) / num_accounts);
        }
    }

    keys_writer.flush().expect("Failed to flush keys file");
    println!("Generated {} accounts in {:?}", num_accounts, start.elapsed());

    // Add system contracts for post-merge operation
    println!("Adding system contracts...");
    add_system_contracts(&mut alloc);

    // Create genesis
    let genesis = Genesis {
        config: ChainConfig {
            chain_id,
            homestead_block: Some(0),
            eip150_block: Some(0),
            eip155_block: Some(0),
            eip158_block: Some(0),
            byzantium_block: Some(0),
            constantinople_block: Some(0),
            petersburg_block: Some(0),
            istanbul_block: Some(0),
            berlin_block: Some(0),
            london_block: Some(0),
            merge_netsplit_block: Some(0),
            terminal_total_difficulty: Some(0),
            terminal_total_difficulty_passed: true,
            shanghai_time: Some(0),
            cancun_time: Some(0),
            prague_time: None, // Disable Prague for simpler benchmarking
            deposit_contract_address: H160::from_str("0x00000000219ab540356cbb839cbe05303d7705fa")
                .unwrap(),
            ..Default::default()
        },
        alloc,
        coinbase: H160::zero(),
        difficulty: U256::from(1),
        extra_data: Bytes::new(),
        gas_limit: 30_000_000,
        nonce: 0x1234,
        mix_hash: H256::zero(),
        timestamp: 1700000000, // Fixed timestamp for reproducibility
        base_fee_per_gas: Some(1_000_000_000), // 1 gwei
        blob_gas_used: Some(0),
        excess_blob_gas: Some(0),
        requests_hash: None,
    };

    // Write genesis to file
    println!("Writing genesis.json...");
    let genesis_file = File::create(&genesis_path).expect("Failed to create genesis file");
    serde_json::to_writer_pretty(genesis_file, &genesis).expect("Failed to write genesis");

    println!("\n=== Complete ===\n");
    println!("Generated files:");
    println!("  Genesis: {} ({} accounts)", genesis_path.display(), num_accounts);
    println!("  Keys: {}", keys_path.display());

    // Show file sizes
    let genesis_size = std::fs::metadata(&genesis_path).map(|m| m.len()).unwrap_or(0);
    let keys_size = std::fs::metadata(&keys_path).map(|m| m.len()).unwrap_or(0);
    println!("\nFile sizes:");
    println!("  Genesis: {:.2} MB", genesis_size as f64 / 1_000_000.0);
    println!("  Keys: {:.2} MB", keys_size as f64 / 1_000_000.0);
}

/// Add system contracts required for post-merge operation (Cancun level)
fn add_system_contracts(alloc: &mut BTreeMap<H160, GenesisAccount>) {
    // Beacon roots contract (EIP-4788) - Required for Cancun
    let beacon_roots_addr = H160::from_str("0x000F3df6D732807Ef1319fB7B8bB8522d0Beac02").unwrap();
    alloc.insert(
        beacon_roots_addr,
        GenesisAccount {
            code: Bytes::from(hex::decode("3373fffffffffffffffffffffffffffffffffffffffe14604d57602036146024575f5ffd5b5f35801560495762001fff810690815414603c575f5ffd5b62001fff01545f5260205ff35b5f5ffd5b62001fff42064281555f359062001fff015500").unwrap()),
            storage: BTreeMap::new(),
            balance: U256::zero(),
            nonce: 1,
        },
    );

    // Deposit contract - Required for post-merge
    let deposit_addr = H160::from_str("0x00000000219ab540356cbb839cbe05303d7705fa").unwrap();
    alloc.insert(
        deposit_addr,
        GenesisAccount {
            code: Bytes::from(hex::decode("60806040526004361061003f5760003560e01c806301ffc9a71461004457806322895118146100a4578063621fd130146101ba578063c5f2892f14610244575b600080fd5b34801561005057600080fd5b506100906004803603602081101561006757600080fd5b50357fffffffff000000000000000000000000000000000000000000000000000000001661026b565b604080519115158252519081900360200190f35b6101b8600480360360808110156100ba57600080fd5b8101906020810181356401000000008111156100d557600080fd5b8201836020820111156100e757600080fd5b8035906020019184600183028401116401000000008311171561010957600080fd5b91939092909160208101903564010000000081111561012757600080fd5b82018360208201111561013957600080fd5b8035906020019184600183028401116401000000008311171561015b57600080fd5b91939092909160208101903564010000000081111561017957600080fd5b82018360208201111561018b57600080fd5b803590602001918460018302840111640100000000831117156101ad57600080fd5b919350915035610304565b005b3480156101c657600080fd5b506101cf6110b5565b6040805160208082528351818301528351919283929083019185019080838360005b838110156102095781810151838201526020016101f1565b50505050905090810190601f1680156102365780820380516001836020036101000a031916815260200191505b509250505060405180910390f35b34801561025057600080fd5b506102596110c7565b60408051918252519081900360200190f35b60007fffffffff0000000000000000000000000000000000000000000000000000000082167f01ffc9a70000000000000000000000000000000000000000000000000000000014806102fe57507fffffffff0000000000000000000000000000000000000000000000000000000082167f8564090700000000000000000000000000000000000000000000000000000000145b92915050565b5050505050505050505050505050565b").unwrap()),
            storage: BTreeMap::new(),
            balance: U256::zero(),
            nonce: 0,
        },
    );

    // Note: EIP-7002 (withdrawal request) and EIP-7251 (consolidation request) contracts
    // are Prague-specific and are omitted since we're using Cancun-level genesis.
}
