use ethrex_core::{Address, H256, U256};

#[derive(Debug, Default, Clone)]
pub struct Environment {
    /// The sender address of the transaction that originated
    /// this execution.
    pub origin: Address,
    pub consumed_gas: U256,
    pub refunded_gas: U256,
    pub gas_limit: U256,
    pub block_number: U256,
    pub coinbase: Address,
    pub timestamp: U256,
    pub prev_randao: Option<H256>,
    pub chain_id: U256,
    pub base_fee_per_gas: U256,
    // It should store effective gas price, in type 2 transactions it is not defined but we calculate if with max fee per gas and max priority fee per gas.
    pub gas_price: U256,
    pub block_excess_blob_gas: Option<U256>,
    pub block_blob_gas_used: Option<U256>,
    pub tx_blob_hashes: Option<Vec<H256>>,
    pub block_gas_limit: U256,
    pub tx_max_priority_fee_per_gas: Option<U256>,
    pub tx_max_fee_per_gas: Option<U256>,
    pub tx_max_fee_per_blob_gas: Option<U256>,
}

impl Environment {
    pub fn default_from_address(origin: Address) -> Self {
        Self {
            origin,
            consumed_gas: U256::zero(),
            refunded_gas: U256::default(),
            gas_limit: U256::MAX,
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
            block_gas_limit: Default::default(),
            tx_max_priority_fee_per_gas: Default::default(),
            tx_max_fee_per_gas: Default::default(),
            tx_max_fee_per_blob_gas: Default::default(),
        }
    }
}
