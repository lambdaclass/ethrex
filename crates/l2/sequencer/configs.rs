use aligned_sdk::common::types::Network;
use ethrex_common::{Address, U256};
use secp256k1::SecretKey;
use std::net::IpAddr;

#[derive(Clone, Debug)]
pub struct SequencerConfig {
    pub block_producer: BlockProducerConfig,
    pub l1_committer: CommitterConfig,
    pub eth: EthConfig,
    pub l1_watcher: L1WatcherConfig,
    pub proof_coordinator: ProofCoordinatorConfig,
    pub based: BasedConfig,
    pub aligned: AlignedConfig,
}

// TODO: Move to blockchain/dev
#[derive(Clone, Debug)]
pub struct BlockProducerConfig {
    pub block_time_ms: u64,
    pub coinbase_address: Address,
    pub elasticity_multiplier: u64,
}

#[derive(Clone, Debug)]
pub struct CommitterConfig {
    pub on_chain_proposer_address: Address,
    pub l1_address: Address,
    pub l1_private_key: SecretKey,
    pub commit_time_ms: u64,
    pub arbitrary_base_blob_gas_price: u64,
    pub validium: bool,
}

#[derive(Clone, Debug)]
pub struct EthConfig {
    pub rpc_url: Vec<String>,
    pub maximum_allowed_max_fee_per_gas: u64,
    pub maximum_allowed_max_fee_per_blob_gas: u64,
    pub max_number_of_retries: u64,
    pub backoff_factor: u64,
    pub min_retry_delay: u64,
    pub max_retry_delay: u64,
}

#[derive(Clone, Debug)]
pub struct L1WatcherConfig {
    pub bridge_address: Address,
    pub check_interval_ms: u64,
    pub max_block_step: U256,
    pub watcher_block_delay: u64,
}

#[derive(Clone, Debug)]
pub struct ProofCoordinatorConfig {
    pub l1_address: Address,
    pub l1_private_key: SecretKey,
    pub listen_ip: IpAddr,
    pub listen_port: u16,
    pub proof_send_interval_ms: u64,
    pub dev_mode: bool,
    pub validium: bool,
}

#[derive(Clone, Debug)]
pub struct BasedConfig {
    pub based: bool,
    pub state_updater: StateUpdaterConfig,
    pub block_fetcher: BlockFetcherConfig,
}

#[derive(Clone, Debug)]
pub struct StateUpdaterConfig {
    pub sequencer_registry: Address,
    pub check_interval_ms: u64,
}

#[derive(Clone, Debug)]
pub struct BlockFetcherConfig {
    pub fetch_interval_ms: u64,
    pub fetch_block_step: u64,
}

#[derive(Clone, Debug)]
pub struct AlignedConfig {
    pub aligned_mode: bool,
    pub aligned_verifier_interval_ms: u64,
    pub beacon_urls: Vec<String>,
    pub network: Network,
    pub fee_estimate: String,
    pub aligned_sp1_elf_path: String,
}
