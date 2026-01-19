//! Mock Consensus Client
//!
//! A benchmark tool that acts as a simplified consensus client, sending blocks
//! to an ethrex node via the Engine API. It measures the time taken for each
//! Engine API call and records statistics.
//!
//! Usage:
//!   cargo run -p ethrex-benches --bin mock_consensus --release -- [OPTIONS]
//!
//! Options:
//!   --node-url <URL>       HTTP RPC endpoint (default: http://localhost:8545)
//!   --auth-url <URL>       Auth RPC endpoint (default: http://localhost:8551)
//!   --jwt-secret <PATH>    Path to JWT secret file (default: jwt.hex)
//!   --keys-file <PATH>     Path to private keys file (default: test_data/genesis_1m/private_keys.txt)
//!   --num-blocks <N>       Number of blocks to produce (default: 10)
//!   --txs-per-block <N>    Transactions per block (default: 400)
//!   --output <PATH>        File to save timing results (default: timing_results.csv)
//!   --slot-time <MS>       Time between blocks in ms (default: 12000)

use bytes::Bytes;
use chrono::Utc;
use ethereum_types::{Address, H256, U256};
use ethrex_common::types::{EIP1559Transaction, TxKind};
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_rlp::encode::RLPEncode;
use ethrex_rpc::{
    clients::{auth::EngineClient, eth::EthClient},
    types::{
        block_identifier::{BlockIdentifier, BlockTag},
        fork_choice::{ForkChoiceState, PayloadAttributesV3},
        payload::ExecutionPayloadResponse,
    },
    utils::{RpcRequest, RpcRequestId},
};
use once_cell::sync::OnceCell;
use rand::{rngs::StdRng, Rng, SeedableRng};
use reqwest::Client;
use secp256k1::SecretKey;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tracing::{error, info, warn};

/// Chain ID for the test network (matches genesis)
const CHAIN_ID: u64 = 1337;

/// Gas limit for a simple transfer
const TRANSFER_GAS: u64 = 21000;

/// Transfer value: 100 gwei
const TRANSFER_VALUE: u64 = 100_000_000_000; // 100 gwei in wei

/// Max priority fee per gas (tip)
const MAX_PRIORITY_FEE: u64 = 1_000_000_000; // 1 gwei

/// Timing record for a single API call
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TimingRecord {
    block_number: u64,
    timestamp: String,
    call_type: String,
    duration_ms: f64,
    success: bool,
    error: Option<String>,
}

/// Statistics for a series of timing records
#[derive(Debug, Default)]
struct TimingStats {
    count: usize,
    total_ms: f64,
    min_ms: f64,
    max_ms: f64,
    successful: usize,
    failed: usize,
}

impl TimingStats {
    fn new() -> Self {
        Self {
            min_ms: f64::MAX,
            max_ms: f64::MIN,
            ..Default::default()
        }
    }

    fn add(&mut self, duration_ms: f64, success: bool) {
        self.count += 1;
        self.total_ms += duration_ms;
        self.min_ms = self.min_ms.min(duration_ms);
        self.max_ms = self.max_ms.max(duration_ms);
        if success {
            self.successful += 1;
        } else {
            self.failed += 1;
        }
    }

    fn mean_ms(&self) -> f64 {
        if self.count > 0 {
            self.total_ms / self.count as f64
        } else {
            0.0
        }
    }

    fn display(&self, name: &str) {
        if self.count == 0 {
            println!("  {}: No data", name);
            return;
        }
        println!(
            "  {}: count={}, mean={:.2}ms, min={:.2}ms, max={:.2}ms, success={}, failed={}",
            name,
            self.count,
            self.mean_ms(),
            self.min_ms,
            self.max_ms,
            self.successful,
            self.failed
        );
    }
}

/// Account with signer and nonce tracking
struct Account {
    signer: Signer,
    address: Address,
    nonce: u64,
}

impl Account {
    fn new(private_key: SecretKey) -> Self {
        let local_signer = LocalSigner::new(private_key);
        let address = local_signer.address;
        Self {
            signer: Signer::Local(local_signer),
            address,
            nonce: 0,
        }
    }
}

/// Mock consensus client that produces blocks
struct MockConsensus {
    engine_client: EngineClient,
    eth_client: EthClient,
    http_client: Client,
    auth_url: String,
    jwt_secret: Bytes,
    accounts: Vec<Account>,
    head_block_hash: H256,
    head_block_number: u64,
    timing_records: Vec<TimingRecord>,
    rng: StdRng,
    txs_per_block: usize,
}

impl MockConsensus {
    async fn new(
        auth_url: &str,
        node_url: &str,
        jwt_secret: Bytes,
        private_keys: Vec<SecretKey>,
        txs_per_block: usize,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let engine_client = EngineClient::new(auth_url, jwt_secret.clone());
        let eth_client = EthClient::new(node_url.parse()?)?;
        let http_client = Client::new();

        // Get the latest block info
        let latest_block = eth_client
            .get_block_by_number(BlockIdentifier::Tag(BlockTag::Latest), false)
            .await?;

        let head_block_hash = latest_block.hash;
        let head_block_number = latest_block.header.number;

        info!(
            "Connected to node. Latest block: {} ({})",
            head_block_number,
            hex::encode(head_block_hash)
        );

        // Create accounts from private keys
        let accounts: Vec<Account> = private_keys.into_iter().map(Account::new).collect();

        info!("Loaded {} accounts", accounts.len());

        Ok(Self {
            engine_client,
            eth_client,
            http_client,
            auth_url: auth_url.to_string(),
            jwt_secret,
            accounts,
            head_block_hash,
            head_block_number,
            timing_records: Vec::new(),
            rng: StdRng::seed_from_u64(42), // Deterministic for reproducibility
            txs_per_block,
        })
    }

    /// Create JWT auth token
    fn auth_token(&self) -> Result<String, Box<dyn std::error::Error>> {
        let header = jsonwebtoken::Header::default();
        let valid_iat = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let claims = json!({"iat": valid_iat});
        let encoding_key = jsonwebtoken::EncodingKey::from_secret(&self.jwt_secret);
        Ok(jsonwebtoken::encode(&header, &claims, &encoding_key)?)
    }

    /// Call engine_getPayloadV3 (for Cancun fork)
    async fn engine_get_payload_v3(
        &self,
        payload_id: u64,
    ) -> Result<ExecutionPayloadResponse, Box<dyn std::error::Error>> {
        let request = RpcRequest {
            id: RpcRequestId::Number(1),
            jsonrpc: "2.0".to_string(),
            method: "engine_getPayloadV3".to_string(),
            params: Some(vec![json!(format!("{:#x}", payload_id))]),
        };

        let response = self
            .http_client
            .post(&self.auth_url)
            .bearer_auth(self.auth_token()?)
            .header("content-type", "application/json")
            .body(serde_json::to_string(&request)?)
            .send()
            .await?;

        let response_text = response.text().await?;
        let response_json: serde_json::Value = serde_json::from_str(&response_text)?;

        if let Some(error) = response_json.get("error") {
            let message = error
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Unknown error");
            return Err(message.to_string().into());
        }

        let result = response_json
            .get("result")
            .ok_or("No result in response")?;
        let payload_response: ExecutionPayloadResponse = serde_json::from_value(result.clone())?;
        Ok(payload_response)
    }

    /// Call engine_newPayloadV3 (for Cancun fork)
    async fn engine_new_payload_v3(
        &self,
        execution_payload: &ethrex_rpc::types::payload::ExecutionPayload,
        expected_blob_versioned_hashes: Vec<H256>,
        parent_beacon_block_root: H256,
    ) -> Result<ethrex_rpc::types::payload::PayloadStatus, Box<dyn std::error::Error>> {
        let request = RpcRequest {
            id: RpcRequestId::Number(1),
            jsonrpc: "2.0".to_string(),
            method: "engine_newPayloadV3".to_string(),
            params: Some(vec![
                serde_json::to_value(execution_payload)?,
                json!(expected_blob_versioned_hashes),
                json!(parent_beacon_block_root),
            ]),
        };

        let response = self
            .http_client
            .post(&self.auth_url)
            .bearer_auth(self.auth_token()?)
            .header("content-type", "application/json")
            .body(serde_json::to_string(&request)?)
            .send()
            .await?;

        let response_text = response.text().await?;
        let response_json: serde_json::Value = serde_json::from_str(&response_text)?;

        if let Some(error) = response_json.get("error") {
            let message = error
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Unknown error");
            return Err(message.to_string().into());
        }

        let result = response_json
            .get("result")
            .ok_or("No result in response")?;
        let payload_status: ethrex_rpc::types::payload::PayloadStatus =
            serde_json::from_value(result.clone())?;
        Ok(payload_status)
    }

    /// Record a timing measurement
    fn record_timing(
        &mut self,
        block_number: u64,
        call_type: &str,
        duration: Duration,
        success: bool,
        error: Option<String>,
    ) {
        let duration_ms = duration.as_secs_f64() * 1000.0;
        let record = TimingRecord {
            block_number,
            timestamp: Utc::now().to_rfc3339(),
            call_type: call_type.to_string(),
            duration_ms,
            success,
            error,
        };

        // Print to stdout
        if success {
            println!(
                "  {} took {:.2}ms",
                call_type, duration_ms
            );
        } else {
            println!(
                "  {} FAILED after {:.2}ms: {}",
                call_type,
                duration_ms,
                record.error.as_deref().unwrap_or("unknown")
            );
        }

        self.timing_records.push(record);
    }

    /// Create a random transfer transaction
    async fn create_transfer(&mut self, base_fee: u64) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        // Pick random sender and receiver (different accounts)
        let num_accounts = self.accounts.len();
        let sender_idx = self.rng.gen_range(0..num_accounts);
        let mut receiver_idx = self.rng.gen_range(0..num_accounts);
        while receiver_idx == sender_idx {
            receiver_idx = self.rng.gen_range(0..num_accounts);
        }

        let receiver_address = self.accounts[receiver_idx].address;

        // Get sender info and increment nonce
        let sender = &mut self.accounts[sender_idx];
        let nonce = sender.nonce;
        sender.nonce += 1;

        // Calculate max fee per gas (base fee + priority fee with some buffer)
        let max_fee_per_gas = base_fee * 2 + MAX_PRIORITY_FEE;

        // Create EIP1559 transaction
        let mut tx = EIP1559Transaction {
            chain_id: CHAIN_ID,
            nonce,
            max_priority_fee_per_gas: MAX_PRIORITY_FEE,
            max_fee_per_gas,
            gas_limit: TRANSFER_GAS,
            to: TxKind::Call(receiver_address),
            value: U256::from(TRANSFER_VALUE),
            data: Bytes::new(),
            access_list: Vec::new(),
            signature_y_parity: false,
            signature_r: U256::zero(),
            signature_s: U256::zero(),
            inner_hash: OnceCell::new(),
        };

        // Sign the transaction
        tx.sign_inplace(&sender.signer).await?;

        // RLP encode the signed transaction with type prefix
        let mut encoded = vec![0x02u8]; // EIP1559 type prefix
        tx.encode(&mut encoded);

        Ok(encoded)
    }

    /// Send transactions to the mempool
    async fn send_transactions_to_mempool(
        &mut self,
        base_fee: u64,
    ) -> Result<usize, Box<dyn std::error::Error>> {
        let mut sent = 0;

        for _ in 0..self.txs_per_block {
            let tx_bytes = self.create_transfer(base_fee).await?;

            match self.eth_client.send_raw_transaction(&tx_bytes).await {
                Ok(_) => sent += 1,
                Err(e) => {
                    // Log but continue - some failures are expected (e.g., nonce issues)
                    warn!("Failed to send transaction: {}", e);
                }
            }
        }

        Ok(sent)
    }

    /// Produce a single block
    async fn produce_block(&mut self) -> Result<bool, Box<dyn std::error::Error>> {
        let block_number = self.head_block_number + 1;
        println!("\n=== Producing block {} ===", block_number);

        // Get the current base fee
        let latest_block = self.eth_client
            .get_block_by_number(BlockIdentifier::Tag(BlockTag::Latest), false)
            .await?;
        let base_fee = latest_block.header.base_fee_per_gas.unwrap_or(1_000_000_000);

        // 1. Send transactions to mempool
        let start = Instant::now();
        let sent = self.send_transactions_to_mempool(base_fee).await?;
        let duration = start.elapsed();
        println!("  Sent {} transactions to mempool in {:.2}ms", sent, duration.as_secs_f64() * 1000.0);

        // 2. Trigger block building with forkchoiceUpdatedV3
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let fork_choice_state = ForkChoiceState {
            head_block_hash: self.head_block_hash,
            safe_block_hash: self.head_block_hash,
            finalized_block_hash: self.head_block_hash,
        };
        let payload_attributes = PayloadAttributesV3 {
            timestamp,
            prev_randao: H256::zero(),
            suggested_fee_recipient: Address::zero(),
            withdrawals: Some(Vec::new()),
            parent_beacon_block_root: Some(H256::zero()),
        };

        let start = Instant::now();
        let fork_choice_response = self
            .engine_client
            .engine_forkchoice_updated_v3(fork_choice_state.clone(), Some(payload_attributes))
            .await;
        let duration = start.elapsed();

        let payload_id = match fork_choice_response {
            Ok(response) => {
                self.record_timing(block_number, "forkchoiceUpdatedV3 (build)", duration, true, None);
                response.payload_id.ok_or("No payload_id returned")?
            }
            Err(e) => {
                self.record_timing(
                    block_number,
                    "forkchoiceUpdatedV3 (build)",
                    duration,
                    false,
                    Some(e.to_string()),
                );
                return Err(e.into());
            }
        };

        // 3. Get the built payload (using V3 for Cancun fork)
        let start = Instant::now();
        let payload_response = self.engine_get_payload_v3(payload_id).await;
        let duration = start.elapsed();

        let execution_payload = match payload_response {
            Ok(response) => {
                self.record_timing(block_number, "getPayloadV3", duration, true, None);
                response.execution_payload
            }
            Err(e) => {
                self.record_timing(
                    block_number,
                    "getPayloadV3",
                    duration,
                    false,
                    Some(e.to_string()),
                );
                return Err(e.into());
            }
        };

        let new_block_hash = execution_payload.block_hash;
        let new_block_number = execution_payload.block_number;

        // 4. Send the new payload (using V3 for Cancun fork)
        let start = Instant::now();
        let new_payload_result = self
            .engine_new_payload_v3(&execution_payload, vec![], H256::zero())
            .await;
        let duration = start.elapsed();

        match &new_payload_result {
            Ok(status) => {
                self.record_timing(block_number, "newPayloadV3", duration, true, None);
                if status.latest_valid_hash.is_none() {
                    warn!("newPayloadV3 returned status: {:?}", status);
                }
            }
            Err(e) => {
                self.record_timing(
                    block_number,
                    "newPayloadV3",
                    duration,
                    false,
                    Some(e.to_string()),
                );
                return Err(e.to_string().into());
            }
        }

        // 5. Update fork choice to make block canonical
        let new_fork_choice_state = ForkChoiceState {
            head_block_hash: new_block_hash,
            safe_block_hash: new_block_hash,
            finalized_block_hash: new_block_hash,
        };

        let start = Instant::now();
        let finalize_result = self
            .engine_client
            .engine_forkchoice_updated_v3(new_fork_choice_state, None)
            .await;
        let duration = start.elapsed();

        match finalize_result {
            Ok(_) => {
                self.record_timing(block_number, "forkchoiceUpdatedV3 (finalize)", duration, true, None);
            }
            Err(e) => {
                self.record_timing(
                    block_number,
                    "forkchoiceUpdatedV3 (finalize)",
                    duration,
                    false,
                    Some(e.to_string()),
                );
                return Err(e.into());
            }
        }

        // Update state
        self.head_block_hash = new_block_hash;
        self.head_block_number = block_number;

        println!(
            "  Block {} produced, hash: {}",
            new_block_number,
            hex::encode(new_block_hash)
        );

        Ok(true)
    }

    /// Run the mock consensus for N blocks
    /// Returns the total benchmark duration (excluding initial setup)
    async fn run(&mut self, num_blocks: u64, slot_time_ms: u64) -> Result<Duration, Box<dyn std::error::Error>> {
        info!("Starting mock consensus for {} blocks", num_blocks);

        let benchmark_start = Instant::now();
        let slot_duration = Duration::from_millis(slot_time_ms);

        for i in 0..num_blocks {
            let slot_start = Instant::now();

            match self.produce_block().await {
                Ok(_) => {}
                Err(e) => {
                    error!("Failed to produce block {}: {}", self.head_block_number + 1, e);
                    // Continue to next block
                }
            }

            // Wait for the remainder of the slot
            let elapsed = slot_start.elapsed();
            if elapsed < slot_duration && i < num_blocks - 1 {
                let remaining = slot_duration - elapsed;
                println!("  Waiting {:.2}s for next slot...", remaining.as_secs_f64());
                tokio::time::sleep(remaining).await;
            }
        }

        Ok(benchmark_start.elapsed())
    }

    /// Save timing records to CSV file
    fn save_timing_records(&self, path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);

        // Write CSV header
        writeln!(writer, "block_number,timestamp,call_type,duration_ms,success,error")?;

        // Write records
        for record in &self.timing_records {
            writeln!(
                writer,
                "{},{},{},{:.3},{},{}",
                record.block_number,
                record.timestamp,
                record.call_type,
                record.duration_ms,
                record.success,
                record.error.as_deref().unwrap_or("")
            )?;
        }

        writer.flush()?;
        info!("Saved {} timing records to {}", self.timing_records.len(), path.display());

        Ok(())
    }

    /// Print summary statistics
    fn print_summary(&self) {
        println!("\n========================================");
        println!("           TIMING SUMMARY");
        println!("========================================\n");

        // Group by call type
        let mut stats_by_type: HashMap<String, TimingStats> = HashMap::new();

        for record in &self.timing_records {
            let stats = stats_by_type
                .entry(record.call_type.clone())
                .or_insert_with(TimingStats::new);
            stats.add(record.duration_ms, record.success);
        }

        // Print stats for each call type
        let call_order = [
            "forkchoiceUpdatedV3 (build)",
            "getPayloadV3",
            "newPayloadV3",
            "forkchoiceUpdatedV3 (finalize)",
        ];

        for call_type in call_order {
            if let Some(stats) = stats_by_type.get(call_type) {
                stats.display(call_type);
            }
        }

        // Calculate total block production time
        println!("\nTotal blocks attempted: {}", self.head_block_number);

        // Calculate overall success rate
        let total_calls = self.timing_records.len();
        let successful_calls = self.timing_records.iter().filter(|r| r.success).count();
        println!(
            "Overall API call success rate: {}/{} ({:.1}%)",
            successful_calls,
            total_calls,
            (successful_calls as f64 / total_calls as f64) * 100.0
        );

        println!("\n========================================\n");
    }
}

/// Load private keys from file
fn load_private_keys(path: &PathBuf, max_keys: usize) -> Result<Vec<SecretKey>, Box<dyn std::error::Error>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);

    let mut keys = Vec::new();
    for line in reader.lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Remove 0x prefix if present
        let hex_str = line.strip_prefix("0x").unwrap_or(line);
        let key_bytes = hex::decode(hex_str)?;
        let secret_key = SecretKey::from_slice(&key_bytes)?;
        keys.push(secret_key);

        if keys.len() >= max_keys {
            break;
        }
    }

    Ok(keys)
}

/// Load JWT secret from file
fn load_jwt_secret(path: &PathBuf) -> Result<Bytes, Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(path)?;
    let hex_str = content.trim().strip_prefix("0x").unwrap_or(content.trim());
    let bytes = hex::decode(hex_str)?;
    Ok(Bytes::from(bytes))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_target(false)
        .init();

    let args: Vec<String> = std::env::args().collect();

    // Parse arguments
    let mut node_url = "http://localhost:8545".to_string();
    let mut auth_url = "http://localhost:8551".to_string();
    let mut jwt_secret_path = PathBuf::from("jwt.hex");
    let mut keys_path = PathBuf::from("test_data/genesis_1m/private_keys.txt");
    let mut num_blocks: u64 = 10;
    let mut txs_per_block: usize = 400;
    let mut output_path = PathBuf::from("timing_results.csv");
    let mut slot_time_ms: u64 = 12000;
    let mut max_accounts: usize = 10000; // Limit accounts to manage memory

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--node-url" => {
                i += 1;
                node_url = args[i].clone();
            }
            "--auth-url" => {
                i += 1;
                auth_url = args[i].clone();
            }
            "--jwt-secret" => {
                i += 1;
                jwt_secret_path = PathBuf::from(&args[i]);
            }
            "--keys-file" => {
                i += 1;
                keys_path = PathBuf::from(&args[i]);
            }
            "--num-blocks" => {
                i += 1;
                num_blocks = args[i].parse()?;
            }
            "--txs-per-block" => {
                i += 1;
                txs_per_block = args[i].parse()?;
            }
            "--output" => {
                i += 1;
                output_path = PathBuf::from(&args[i]);
            }
            "--slot-time" => {
                i += 1;
                slot_time_ms = args[i].parse()?;
            }
            "--max-accounts" => {
                i += 1;
                max_accounts = args[i].parse()?;
            }
            "--help" | "-h" => {
                println!("Mock Consensus Client - Benchmark tool for ethrex Engine API");
                println!();
                println!("Usage: mock_consensus [OPTIONS]");
                println!();
                println!("Options:");
                println!("  --node-url <URL>       HTTP RPC endpoint (default: http://localhost:8545)");
                println!("  --auth-url <URL>       Auth RPC endpoint (default: http://localhost:8551)");
                println!("  --jwt-secret <PATH>    Path to JWT secret file (default: jwt.hex)");
                println!("  --keys-file <PATH>     Path to private keys file");
                println!("  --num-blocks <N>       Number of blocks to produce (default: 10)");
                println!("  --txs-per-block <N>    Transactions per block (default: 400)");
                println!("  --output <PATH>        Output file for timing results (default: timing_results.csv)");
                println!("  --slot-time <MS>       Time between blocks in ms (default: 12000)");
                println!("  --max-accounts <N>     Maximum accounts to load (default: 10000)");
                println!("  --help                 Show this help");
                return Ok(());
            }
            _ => {
                eprintln!("Unknown argument: {}", args[i]);
                std::process::exit(1);
            }
        }
        i += 1;
    }

    println!("=== Mock Consensus Client ===\n");
    println!("Configuration:");
    println!("  Node URL: {}", node_url);
    println!("  Auth URL: {}", auth_url);
    println!("  JWT Secret: {}", jwt_secret_path.display());
    println!("  Keys File: {}", keys_path.display());
    println!("  Num Blocks: {}", num_blocks);
    println!("  Txs/Block: {}", txs_per_block);
    println!("  Slot Time: {}ms", slot_time_ms);
    println!("  Output: {}", output_path.display());
    println!();

    // Load JWT secret
    let jwt_secret = load_jwt_secret(&jwt_secret_path)?;
    info!("Loaded JWT secret");

    // Load private keys
    info!("Loading private keys from {}...", keys_path.display());
    let private_keys = load_private_keys(&keys_path, max_accounts)?;
    info!("Loaded {} private keys", private_keys.len());

    if private_keys.len() < 2 {
        return Err("Need at least 2 accounts for transfers".into());
    }

    // Create mock consensus
    let mut mock = MockConsensus::new(
        &auth_url,
        &node_url,
        jwt_secret,
        private_keys,
        txs_per_block,
    )
    .await?;

    // Run the benchmark
    let benchmark_duration = mock.run(num_blocks, slot_time_ms).await?;

    // Save results
    mock.save_timing_records(&output_path)?;

    // Print summary
    mock.print_summary();

    // Verify final state
    let final_block = mock.eth_client
        .get_block_by_number(BlockIdentifier::Tag(BlockTag::Latest), false)
        .await?;
    println!("Final block number: {}", final_block.header.number);
    println!("Final block hash: {}", hex::encode(final_block.hash));

    // Print benchmark runtime
    let total_secs = benchmark_duration.as_secs();
    let minutes = total_secs / 60;
    let seconds = total_secs % 60;
    let millis = benchmark_duration.subsec_millis();

    println!();
    println!("========================================");
    println!("         BENCHMARK RUNTIME");
    println!("========================================");
    if minutes > 0 {
        println!("  Total time: {}m {}s", minutes, seconds);
    } else {
        println!("  Total time: {}.{:03}s", seconds, millis);
    }
    println!("  Blocks produced: {}", num_blocks);
    println!("  Avg time per block: {:.2}ms", benchmark_duration.as_secs_f64() * 1000.0 / num_blocks as f64);
    println!("========================================");

    Ok(())
}
