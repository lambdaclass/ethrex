use ethrex_common::{Address, H256, U256};

use crate::vm::EVMConfig;

use std::collections::HashMap;
/// [EIP-1153]: https://eips.ethereum.org/EIPS/eip-1153#reference-implementation
pub type TransientStorage = HashMap<(Address, U256), U256>;

#[derive(Debug, Default, Clone)]
/// Environmental information that the execution agent must provide.
pub struct Environment {
    /// The sender address of the external transaction.
    pub origin: Address,
    pub refunded_gas: u64, //TODO: Move this to Substate
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
    pub transient_storage: TransientStorage,
    pub is_privileged: bool,
}
