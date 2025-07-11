use ethrex_common::{Address, H256, U256};
use ethrex_levm::{EVMConfig, Environment};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize, Debug)]
pub struct Benchmark {
    pub fork: String,
    pub env: Env,
    pub pre: HashMap<String, Account>,
    pub transaction: Transaction,
    pub initial_memory: String,
    pub initial_stack: Vec<String>,
}

#[derive(Deserialize, Debug)]
pub struct Env {
    /// The sender address of the external transaction.
    pub origin: Address,
    /// Gas limit of the Transaction
    pub gas_limit: u64,
    pub config: EVMConfig,
    pub block_number: U256,
    /// Coinbase is the block's beneficiary - the address that receives the block rewards and fees.
    pub coinbase: Address,
    pub timestamp: U256,
    pub prev_randao: Option<H256>,
    pub difficulty: U256,
    pub chain_id: U256,
    pub base_fee_per_gas: U256,
    pub gas_price: U256, // Effective gas price
    pub block_excess_blob_gas: Option<U256>,
    pub block_blob_gas_used: Option<U256>,
    pub tx_blob_hashes: Vec<H256>,
    pub tx_max_priority_fee_per_gas: Option<U256>,
    pub tx_max_fee_per_gas: Option<U256>,
    pub tx_max_fee_per_blob_gas: Option<U256>,
    pub tx_nonce: u64,
    pub block_gas_limit: u64,
}

#[derive(Deserialize, Debug)]
pub struct Account {
    pub balance: Option<String>,
    pub code: Option<String>,
    pub nonce: Option<String>,
    pub storage: HashMap<String, String>,
}

#[derive(Deserialize, Debug)]
pub struct Transaction {
    // Add actual transaction fields as needed
    pub from: Option<String>,
    pub to: Option<String>,
    pub value: Option<String>,
    pub data: Option<String>,
    pub gas: Option<String>,
    pub gas_price: Option<String>,
}
