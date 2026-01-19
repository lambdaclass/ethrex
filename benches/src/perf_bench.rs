//! Performance benchmark for ethrex
//!
//! This benchmark creates a large Ethereum state and measures block execution time.
//!
//! ## Initial Setup
//! - ~1000 accounts with 10 ETH each (scalable)
//! - 10 ERC20 contracts with pre-populated balances
//!
//! ## Benchmark
//! - N blocks with M transactions each
//! - Records ms per block execution and calculates mean

use bytes::Bytes;
use ethereum_types::{Address, H160, H256, U256};
use ethrex_blockchain::{Blockchain, BlockchainOptions, BlockchainType};
use ethrex_blockchain::payload::{BuildPayloadArgs, PayloadBuildResult, create_payload};
use ethrex_common::types::{
    EIP1559Transaction, Genesis, GenesisAccount, Transaction, TxKind,
};
use ethrex_crypto::keccak::keccak_hash;
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_storage::{EngineType, Store};
use once_cell::sync::OnceCell;
use secp256k1::SecretKey;
use std::collections::BTreeMap;
use std::str::FromStr;
use std::time::Instant;

/// Hardcoded seed for reproducible key generation
const SEED: &[u8; 32] = b"ethrex_perf_benchmark_seed_2024!";

/// Number of regular accounts (with ETH balance only)
const NUM_ACCOUNTS: usize = 1_000;

/// Target transactions per block (controls gas limit)
const TXS_PER_BLOCK: usize = 400;

/// Gas limit per block (TXS_PER_BLOCK * 21000 gas per simple transfer + some buffer)
const BLOCK_GAS_LIMIT: u64 = (TXS_PER_BLOCK as u64) * 21_000 + 100_000;

/// Number of blocks to execute
const NUM_BLOCKS: usize = 100;

/// Chain ID for the benchmark
const CHAIN_ID: u64 = 1337;

/// Balance for each account (1000 ETH in wei)
const ACCOUNT_BALANCE: &str = "0x3635c9adc5dea00000"; // 1000 ETH

/// Result of a benchmark run
#[derive(Debug, Clone)]
pub struct BenchmarkResult {
    pub times_ms: Vec<f64>,
    pub mean_ms: f64,
    pub total_gas_used: u64,
}

/// Generate a deterministic private key from seed and index
fn generate_private_key(index: usize) -> SecretKey {
    let mut data = [0u8; 32];
    data[..SEED.len()].copy_from_slice(SEED);

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
fn derive_address(sk: &SecretKey) -> Address {
    let signer = Signer::Local(LocalSigner::new(*sk));
    signer.address()
}

/// Generate genesis with accounts
pub fn generate_genesis() -> (Genesis, Vec<SecretKey>, Vec<Address>) {
    println!("Generating genesis with {} accounts...", NUM_ACCOUNTS);
    let start = Instant::now();

    let mut alloc: BTreeMap<Address, GenesisAccount> = BTreeMap::new();
    let mut secret_keys = Vec::with_capacity(NUM_ACCOUNTS);
    let mut account_addresses = Vec::with_capacity(NUM_ACCOUNTS);

    let balance = U256::from_str(ACCOUNT_BALANCE).unwrap();

    // Generate regular accounts
    for i in 0..NUM_ACCOUNTS {
        let sk = generate_private_key(i);
        let addr = derive_address(&sk);

        alloc.insert(addr, GenesisAccount {
            code: Bytes::new(),
            storage: BTreeMap::new(),
            balance,
            nonce: 0,
        });

        secret_keys.push(sk);
        account_addresses.push(addr);
    }

    // Add system contracts required for post-merge
    add_system_contracts(&mut alloc);

    let genesis = Genesis {
        config: ethrex_common::types::ChainConfig {
            chain_id: CHAIN_ID,
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
            prague_time: Some(0),
            deposit_contract_address: H160::from_str("0x00000000219ab540356cbb839cbe05303d7705fa").unwrap(),
            ..Default::default()
        },
        alloc,
        coinbase: Address::zero(),
        difficulty: U256::from(1),
        extra_data: Bytes::new(),
        gas_limit: BLOCK_GAS_LIMIT,
        nonce: 0x1234,
        mix_hash: H256::zero(),
        timestamp: 1718040081,
        base_fee_per_gas: Some(1_000_000_000), // 1 gwei
        blob_gas_used: Some(0),
        excess_blob_gas: Some(0),
        requests_hash: None,
    };

    println!("Genesis generation took {:?}", start.elapsed());
    println!("Total accounts: {}", genesis.alloc.len());

    (genesis, secret_keys, account_addresses)
}

/// Add system contracts required for post-merge operation
fn add_system_contracts(alloc: &mut BTreeMap<Address, GenesisAccount>) {
    // Beacon roots contract (EIP-4788)
    let beacon_roots_addr = H160::from_str("0x000F3df6D732807Ef1319fB7B8bB8522d0Beac02").unwrap();
    alloc.insert(beacon_roots_addr, GenesisAccount {
        code: Bytes::from(hex::decode("3373fffffffffffffffffffffffffffffffffffffffe14604d57602036146024575f5ffd5b5f35801560495762001fff810690815414603c575f5ffd5b62001fff01545f5260205ff35b5f5ffd5b62001fff42064281555f359062001fff015500").unwrap()),
        storage: BTreeMap::new(),
        balance: U256::zero(),
        nonce: 1,
    });

    // Deposit contract
    let deposit_addr = H160::from_str("0x00000000219ab540356cbb839cbe05303d7705fa").unwrap();
    alloc.insert(deposit_addr, GenesisAccount {
        code: Bytes::from(hex::decode("60806040526004361061003f5760003560e01c806301ffc9a71461004457806322895118146100a4578063621fd130146101ba578063c5f2892f14610244575b600080fd5b34801561005057600080fd5b506100906004803603602081101561006757600080fd5b50357fffffffff000000000000000000000000000000000000000000000000000000001661026b565b604080519115158252519081900360200190f35b6101b8600480360360808110156100ba57600080fd5b8101906020810181356401000000008111156100d557600080fd5b8201836020820111156100e757600080fd5b8035906020019184600183028401116401000000008311171561010957600080fd5b91939092909160208101903564010000000081111561012757600080fd5b82018360208201111561013957600080fd5b8035906020019184600183028401116401000000008311171561015b57600080fd5b91939092909160208101903564010000000081111561017957600080fd5b82018360208201111561018b57600080fd5b803590602001918460018302840111640100000000831117156101ad57600080fd5b919350915035610304565b005b3480156101c657600080fd5b506101cf6110b5565b6040805160208082528351818301528351919283929083019185019080838360005b838110156102095781810151838201526020016101f1565b50505050905090810190601f1680156102365780820380516001836020036101000a031916815260200191505b509250505060405180910390f35b34801561025057600080fd5b506102596110c7565b60408051918252519081900360200190f35b60007fffffffff0000000000000000000000000000000000000000000000000000000082167f01ffc9a70000000000000000000000000000000000000000000000000000000014806102fe57507fffffffff0000000000000000000000000000000000000000000000000000000082167f8564090700000000000000000000000000000000000000000000000000000000145b92915050565b5050505050505050505050505050565b").unwrap()),
        storage: BTreeMap::new(),
        balance: U256::zero(),
        nonce: 0,
    });

    // Withdrawal request contract (EIP-7002)
    let withdrawal_addr = H160::from_str("0x00000961ef480eb55e80d19ad83579a64c007002").unwrap();
    alloc.insert(withdrawal_addr, GenesisAccount {
        code: Bytes::from(hex::decode("3373fffffffffffffffffffffffffffffffffffffffe1460cb5760115f54807fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff146101f457600182026001905f5b5f82111560685781019083028483029004916001019190604d565b909390049250505036603814608857366101f457346101f4575f5260205ff35b34106101f457600154600101600155600354806003026004013381556001015f35815560010160203590553360601b5f5260385f601437604c5fa0600101600355005b6003546002548082038060101160df575060105b5f5b8181146101835782810160030260040181604c02815460601b8152601401816001015481526020019060020154807fffffffffffffffffffffffffffffffff00000000000000000000000000000000168252906010019060401c908160381c81600701538160301c81600601538160281c81600501538160201c81600401538160181c81600301538160101c81600201538160081c81600101535360010160e1565b910180921461019557906002556101a0565b90505f6002555f6003555b5f54807fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff14156101cd57505f5b6001546002828201116101e25750505f6101e8565b01600290035b5f555f600155604c025ff35b5f5ffd").unwrap()),
        storage: BTreeMap::new(),
        balance: U256::zero(),
        nonce: 1,
    });

    // Consolidation request contract (EIP-7251)
    let consolidation_addr = H160::from_str("0x0000bbddc7ce488642fb579f8b00f3a590007251").unwrap();
    alloc.insert(consolidation_addr, GenesisAccount {
        code: Bytes::from(hex::decode("3373fffffffffffffffffffffffffffffffffffffffe1460d35760115f54807fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff1461019a57600182026001905f5b5f82111560685781019083028483029004916001019190604d565b9093900492505050366060146088573661019a573461019a575f5260205ff35b341061019a57600154600101600155600354806004026004013381556001015f358155600101602035815560010160403590553360601b5f5260605f60143760745fa0600101600355005b6003546002548082038060021160e7575060025b5f5b8181146101295782810160040260040181607402815460601b815260140181600101548152602001816002015481526020019060030154905260010160e9565b910180921461013b5790600255610146565b90505f6002555f6003555b5f54807fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff141561017357505f5b6001546001828201116101885750505f61018e565b01600190035b5f555f6001556074025ff35b5f5ffd").unwrap()),
        storage: BTreeMap::new(),
        balance: U256::zero(),
        nonce: 1,
    });

    // History storage contract (EIP-2935)
    let history_addr = H160::from_str("0x0000F90827f1C53a10cb7a02335B175320002935").unwrap();
    alloc.insert(history_addr, GenesisAccount {
        code: Bytes::from(hex::decode("3373fffffffffffffffffffffffffffffffffffffffe14604657602036036042575f35600143038111604257611fff81430311604257611fff9006545f5260205ff35b5f5ffd5b5f35611fff60014303065500").unwrap()),
        storage: BTreeMap::new(),
        balance: U256::zero(),
        nonce: 1,
    });
}

/// Fill the mempool with all transactions upfront
/// Creates `txs_per_account` transactions per account with sequential nonces
async fn fill_mempool(
    blockchain: &Blockchain,
    secret_keys: &[SecretKey],
    account_addresses: &[Address],
    txs_per_account: usize,
) {
    let mut txs = vec![];

    for (sender_idx, sk) in secret_keys.iter().enumerate() {
        let signer = Signer::Local(LocalSigner::new(*sk));
        let receiver_idx = (sender_idx + 1) % account_addresses.len();
        let receiver = account_addresses[receiver_idx];

        for nonce in 0..txs_per_account as u64 {
            let mut tx = Transaction::EIP1559Transaction(EIP1559Transaction {
                chain_id: CHAIN_ID,
                nonce,
                max_priority_fee_per_gas: 1_000_000_000,
                max_fee_per_gas: 100_000_000_000_000, // 100k gwei - high to handle base fee growth
                gas_limit: 21_000,
                to: TxKind::Call(receiver),
                value: U256::from(1_000_000_000_000_000_u64), // 0.001 ETH
                data: Bytes::new(),
                access_list: vec![],
                signature_y_parity: false,
                signature_r: U256::zero(),
                signature_s: U256::zero(),
                inner_hash: OnceCell::new(),
            });

            tx.sign_inplace(&signer).await.expect("sign tx");
            txs.push(tx);
        }
    }

    println!("Adding {} transactions to mempool...", txs.len());
    for tx in txs {
        blockchain.add_transaction_to_pool(tx).await.expect("add tx to pool");
    }
}

/// Run the benchmark
pub async fn run_benchmark() -> BenchmarkResult {
    println!("\n=== ETHREX PERFORMANCE BENCHMARK ===\n");

    // Generate genesis
    let (genesis, secret_keys, account_addresses) = generate_genesis();

    // Create storage
    println!("Initializing storage...");
    let storage_path = tempfile::TempDir::new().expect("create temp dir");
    let mut store = Store::new(storage_path.path(), EngineType::RocksDB).expect("create store");

    let init_start = Instant::now();
    store.add_initial_state(genesis.clone()).await.expect("add initial state");
    println!("Storage initialization took {:?}", init_start.elapsed());

    // Create blockchain with large mempool to hold all transactions
    let total_txs_needed = NUM_BLOCKS * TXS_PER_BLOCK;
    let blockchain = Blockchain::new(
        store.clone(),
        BlockchainOptions {
            r#type: BlockchainType::L1,
            perf_logs_enabled: true,
            max_mempool_size: total_txs_needed * 2, // 2x buffer for safety
            ..Default::default()
        },
    );

    let genesis_block = genesis.get_block();
    let mut parent_hash = genesis_block.hash();
    let mut timestamp = genesis.timestamp;

    let mut times_ms = Vec::with_capacity(NUM_BLOCKS);
    let mut total_gas_used = 0u64;

    // Calculate transactions needed: enough to fill all blocks
    // Each account needs enough txs so total txs >= NUM_BLOCKS * TXS_PER_BLOCK
    let total_txs_needed = NUM_BLOCKS * TXS_PER_BLOCK;
    let txs_per_account = (total_txs_needed / secret_keys.len()) + 1;

    // Fill mempool upfront with all transactions
    fill_mempool(&blockchain, &secret_keys, &account_addresses, txs_per_account).await;

    println!("\nRunning {} blocks with ~{} txs each...", NUM_BLOCKS, TXS_PER_BLOCK);

    for block_idx in 0..NUM_BLOCKS {
        timestamp += 12;

        // Build payload args
        let payload_args = BuildPayloadArgs {
            parent: parent_hash,
            timestamp,
            fee_recipient: H160::random(),
            random: H256::random(),
            withdrawals: Some(vec![]),
            beacon_root: Some(H256::zero()),
            version: 3,
            elasticity_multiplier: 2,
            gas_ceil: BLOCK_GAS_LIMIT,
        };

        // Create initial payload template
        let payload_block = create_payload(&payload_args, &store, Bytes::new()).expect("create payload");

        // Build payload synchronously (fills with transactions)
        let start = Instant::now();
        let PayloadBuildResult { payload, .. } = blockchain.build_payload(payload_block).expect("build payload");
        let block_hash = payload.hash();
        let payload_time = start.elapsed();

        // Add block to blockchain (validates and commits state)
        let add_start = Instant::now();
        blockchain.add_block(payload.clone()).expect("add block");
        let add_time = add_start.elapsed();

        let total_time = payload_time + add_time;
        times_ms.push(total_time.as_secs_f64() * 1000.0);

        // Get actual gas used from stored header
        let header = store
            .get_block_header_by_hash(block_hash)
            .expect("get header")
            .expect("header exists");
        total_gas_used += header.gas_used;

        parent_hash = block_hash;

        println!(
            "  Block {}: payload={:.2}ms, add={:.2}ms, total={:.2}ms, gas={}",
            block_idx + 1,
            payload_time.as_secs_f64() * 1000.0,
            add_time.as_secs_f64() * 1000.0,
            total_time.as_secs_f64() * 1000.0,
            header.gas_used
        );
    }

    // Calculate statistics
    let mean_ms = if times_ms.is_empty() {
        0.0
    } else {
        times_ms.iter().sum::<f64>() / times_ms.len() as f64
    };

    let result = BenchmarkResult {
        times_ms: times_ms.clone(),
        mean_ms,
        total_gas_used,
    };

    // Print results
    println!("\n=== BENCHMARK RESULTS ===\n");
    println!("Blocks executed: {}", NUM_BLOCKS);
    println!("Transactions per block: {}", TXS_PER_BLOCK);
    println!("Mean time per block: {:.2} ms", result.mean_ms);
    if !result.times_ms.is_empty() {
        println!("Min time: {:.2} ms", result.times_ms.iter().cloned().fold(f64::INFINITY, f64::min));
        println!("Max time: {:.2} ms", result.times_ms.iter().cloned().fold(0.0, f64::max));
    }

    println!("\nTotal gas used: {}", result.total_gas_used);
    let total_time_s = result.mean_ms * NUM_BLOCKS as f64 / 1000.0;
    if total_time_s > 0.0 {
        println!("Throughput: {:.2} Mgas/s", result.total_gas_used as f64 / 1_000_000.0 / total_time_s);
    }

    result
}

#[tokio::main]
async fn main() {
    run_benchmark().await;
}
