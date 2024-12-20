use std::collections::HashMap;

use ethrex_core::{Address, H256, U256};

/// [EIP-1153]: https://eips.ethereum.org/EIPS/eip-1153#reference-implementation
pub type TransientStorage = HashMap<(Address, U256), U256>;

#[derive(Debug, Default, Clone)]
pub struct Environment {
    /// The sender address of the transaction that originated
    /// this execution.
    pub origin: Address,
    pub refunded_gas: u64,
    pub gas_limit: u64,
    pub block_number: U256,
    pub coinbase: Address,
    pub timestamp: U256,
    pub prev_randao: Option<H256>,
    pub chain_id: U256,
    pub base_fee_per_gas: U256,
    pub gas_price: U256, // Effective gas price
    pub block_excess_blob_gas: Option<U256>,
    pub block_blob_gas_used: Option<U256>,
    pub tx_blob_hashes: Vec<H256>,
    pub tx_max_priority_fee_per_gas: Option<U256>,
    pub tx_max_fee_per_gas: Option<U256>,
    pub tx_max_fee_per_blob_gas: Option<U256>,
    pub block_gas_limit: u64,
    pub transient_storage: TransientStorage,
}

impl Environment {
    pub fn default_from_address(origin: Address) -> Self {
        Self {
            origin,
            refunded_gas: 0,
            gas_limit: u64::MAX,
            block_number: Default::default(),
            coinbase: Default::default(),
            timestamp: Default::default(),
            prev_randao: Default::default(),
            chain_id: U256::one(),
            base_fee_per_gas: Default::default(),
            gas_price: Default::default(),
            block_excess_blob_gas: Default::default(),
            block_blob_gas_used: Default::default(),
            tx_blob_hashes: Default::default(),
            tx_max_priority_fee_per_gas: Default::default(),
            tx_max_fee_per_gas: Default::default(),
            tx_max_fee_per_blob_gas: Default::default(),
            block_gas_limit: Default::default(),
            transient_storage: Default::default(),
        }
    }
}
