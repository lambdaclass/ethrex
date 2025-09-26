use bytes::Bytes;
use ethereum_types::{Address, Bloom, H256, U256};
use ethrex_rlp::encode::RLPEncode;
use ethrex_trie::Trie;
use rkyv::{Archive, Deserialize as RDeserialize, Serialize as RSerialize};
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use std::{
    collections::{BTreeMap, HashMap},
    io::{BufReader, Error},
    path::Path,
};
use tracing::warn;

use super::{
    AccountState, Block, BlockBody, BlockHeader, BlockNumber, INITIAL_BASE_FEE,
    compute_receipts_root, compute_transactions_root, compute_withdrawals_root,
};
use crate::{
    constants::{DEFAULT_OMMERS_HASH, DEFAULT_REQUESTS_HASH},
    rkyv_utils,
};

#[allow(unused)]
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Genesis {
    /// Chain configuration
    pub config: ChainConfig,
    /// The initial state of the accounts in the genesis block.
    /// This is a BTreeMap because: https://github.com/lambdaclass/ethrex/issues/2070
    pub alloc: BTreeMap<Address, GenesisAccount>,
    /// Genesis header values
    pub coinbase: Address,
    pub difficulty: U256,
    #[serde(default, with = "crate::serde_utils::bytes")]
    pub extra_data: Bytes,
    #[serde(with = "crate::serde_utils::u64::hex_str")]
    pub gas_limit: u64,
    #[serde(with = "crate::serde_utils::u64::hex_str")]
    pub nonce: u64,
    #[serde(alias = "mixHash", alias = "mixhash")]
    pub mix_hash: H256,
    #[serde(deserialize_with = "crate::serde_utils::u64::deser_hex_or_dec_str")]
    #[serde(serialize_with = "crate::serde_utils::u256::serialize_number")]
    pub timestamp: u64,
    #[serde(default, with = "crate::serde_utils::u64::hex_str_opt")]
    pub base_fee_per_gas: Option<u64>,
    #[serde(default, with = "crate::serde_utils::u64::hex_str_opt")]
    pub blob_gas_used: Option<u64>,
    #[serde(default, with = "crate::serde_utils::u64::hex_str_opt")]
    pub excess_blob_gas: Option<u64>,
    pub requests_hash: Option<H256>,
}

#[derive(Debug, thiserror::Error)]
pub enum GenesisError {
    #[error("Failed to decode genesis file: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("Fork not supported. Only post-merge networks are supported.")]
    InvalidFork(),
    #[error("Failed to open genesis file: {0}")]
    File(#[from] Error),
}

impl TryFrom<&Path> for Genesis {
    type Error = GenesisError;

    fn try_from(genesis_file_path: &Path) -> Result<Self, Self::Error> {
        let genesis_file = std::fs::File::open(genesis_file_path)?;
        let genesis_reader = BufReader::new(genesis_file);
        let genesis: Genesis = serde_json::from_reader(genesis_reader)?;

        // Try to derive if the genesis file is PoS
        // Different genesis files have different configurations
        // TODO: Remove once we have a way to run PoW chains, i.e Snap Sync
        if genesis.config.terminal_total_difficulty != Some(0)
            && genesis.config.merge_netsplit_block != Some(0)
            && genesis.config.shanghai_time != Some(0)
            && genesis.config.cancun_time != Some(0)
            && genesis.config.prague_time != Some(0)
        {
            // Hive has a minimalistic genesis file, which is not supported
            // return Err(GenesisError::InvalidFork());
            warn!("Invalid fork, only post-merge networks are supported.");
        }

        if genesis.config.bpo3_time.is_some() && genesis.config.blob_schedule.bpo3.is_none()
            || genesis.config.bpo4_time.is_some() && genesis.config.blob_schedule.bpo4.is_none()
            || genesis.config.bpo5_time.is_some() && genesis.config.blob_schedule.bpo5.is_none()
        {
            warn!("BPO time set but no BPO BlobSchedule found in ChainConfig")
        }

        Ok(genesis)
    }
}

#[allow(unused)]
#[derive(
    Clone,
    Copy,
    Debug,
    Serialize,
    Deserialize,
    PartialEq,
    RSerialize,
    RDeserialize,
    Archive,
    Default,
)]
#[serde(rename_all = "camelCase")]
pub struct ForkBlobSchedule {
    pub base_fee_update_fraction: u64,
    pub max: u32,
    pub target: u32,
}

#[allow(unused)]
#[derive(
    Clone, Copy, Debug, Serialize, Deserialize, PartialEq, RSerialize, RDeserialize, Archive,
)]
#[serde(rename_all = "camelCase")]
pub struct BlobSchedule {
    #[serde(default = "default_cancun_schedule")]
    pub cancun: ForkBlobSchedule,
    #[serde(default = "default_prague_schedule")]
    pub prague: ForkBlobSchedule,
    #[serde(default = "default_osaka_schedule")]
    pub osaka: ForkBlobSchedule,
    #[serde(default = "default_bpo1_schedule")]
    pub bpo1: ForkBlobSchedule,
    #[serde(default = "default_bpo2_schedule")]
    pub bpo2: ForkBlobSchedule,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bpo3: Option<ForkBlobSchedule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bpo4: Option<ForkBlobSchedule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bpo5: Option<ForkBlobSchedule>,
}

impl Default for BlobSchedule {
    fn default() -> Self {
        BlobSchedule {
            cancun: default_cancun_schedule(),
            prague: default_prague_schedule(),
            osaka: default_osaka_schedule(),
            bpo1: default_bpo1_schedule(),
            bpo2: default_bpo2_schedule(),
            bpo3: None,
            bpo4: None,
            bpo5: None,
        }
    }
}

fn default_cancun_schedule() -> ForkBlobSchedule {
    ForkBlobSchedule {
        target: 3,
        max: 6,
        base_fee_update_fraction: 3338477,
    }
}

fn default_prague_schedule() -> ForkBlobSchedule {
    ForkBlobSchedule {
        target: 6,
        max: 9,
        base_fee_update_fraction: 5007716,
    }
}

fn default_osaka_schedule() -> ForkBlobSchedule {
    ForkBlobSchedule {
        target: 6,
        max: 9,
        base_fee_update_fraction: 5007716,
    }
}

fn default_bpo1_schedule() -> ForkBlobSchedule {
    ForkBlobSchedule {
        target: 10,
        max: 15,
        base_fee_update_fraction: 8346193,
    }
}

fn default_bpo2_schedule() -> ForkBlobSchedule {
    ForkBlobSchedule {
        target: 14,
        max: 21,
        base_fee_update_fraction: 11684671,
    }
}
/// Blockchain settings defined per block
#[allow(unused)]
#[derive(
    Clone,
    Copy,
    Debug,
    Serialize,
    Deserialize,
    Default,
    PartialEq,
    RSerialize,
    RDeserialize,
    Archive,
)]
#[serde(rename_all = "camelCase")]
pub struct ChainConfig {
    /// Current chain identifier
    pub chain_id: u64,

    /// Block numbers for the block where each fork was activated
    /// (None = no fork, 0 = fork is already active)
    pub homestead_block: Option<u64>,

    pub dao_fork_block: Option<u64>,
    /// Whether the node supports or opposes the DAO hard-fork
    #[serde(default)]
    pub dao_fork_support: bool,

    pub eip150_block: Option<u64>,
    pub eip155_block: Option<u64>,
    pub eip158_block: Option<u64>,

    pub byzantium_block: Option<u64>,
    pub constantinople_block: Option<u64>,
    pub petersburg_block: Option<u64>,
    pub istanbul_block: Option<u64>,
    pub muir_glacier_block: Option<u64>,
    pub berlin_block: Option<u64>,
    pub london_block: Option<u64>,
    pub arrow_glacier_block: Option<u64>,
    pub gray_glacier_block: Option<u64>,
    pub merge_netsplit_block: Option<u64>,

    /// Timestamp at which each fork was activated
    /// (None = no fork, 0 = fork is already active)
    pub shanghai_time: Option<u64>,
    pub cancun_time: Option<u64>,
    pub prague_time: Option<u64>,
    pub verkle_time: Option<u64>,
    pub osaka_time: Option<u64>,

    pub bpo1_time: Option<u64>,
    pub bpo2_time: Option<u64>,
    pub bpo3_time: Option<u64>,
    pub bpo4_time: Option<u64>,
    pub bpo5_time: Option<u64>,

    /// Amount of total difficulty reached by the network that triggers the consensus upgrade.
    pub terminal_total_difficulty: Option<u128>,
    /// Network has already passed the terminal total difficult
    #[serde(default)]
    pub terminal_total_difficulty_passed: bool,
    #[serde(default)]
    pub blob_schedule: BlobSchedule,
    #[rkyv(with = rkyv_utils::H160Wrapper)]
    // Deposits system contract address
    pub deposit_contract_address: Address,

    #[serde(default)]
    pub enable_verkle_at_genesis: bool,
}

lazy_static::lazy_static! {
    pub static ref NETWORK_NAMES: HashMap<u64, &'static str> = {
        HashMap::from([
            (1, "mainnet"),
            (11155111, "sepolia"),
            (17000, "holesky"),
            (560048, "hoodi"),
            (9, "L1 local devnet"),
            (65536999, "L2 local devnet"),
        ])
    };
}

#[repr(u8)]
#[derive(Debug, PartialEq, Eq, PartialOrd, Default, Hash, Clone, Copy, Serialize, Deserialize)]
pub enum Fork {
    Frontier = 0,
    FrontierThawing = 1,
    Homestead = 2,
    DaoFork = 3,
    Tangerine = 4,
    SpuriousDragon = 5,
    Byzantium = 6,
    Constantinople = 7,
    Petersburg = 8,
    Istanbul = 9,
    MuirGlacier = 10,
    Berlin = 11,
    London = 12,
    ArrowGlacier = 13,
    GrayGlacier = 14,
    Paris = 15,
    Shanghai = 16,
    #[default]
    Cancun = 17,
    Prague = 18,
    Osaka = 19,
    BPO1 = 20,
    BPO2 = 21,
    BPO3 = 22,
    BPO4 = 23,
    BPO5 = 24,
}

impl From<Fork> for &str {
    fn from(fork: Fork) -> Self {
        match fork {
            Fork::Frontier => "Frontier",
            Fork::FrontierThawing => "FrontierThawing",
            Fork::Homestead => "Homestead",
            Fork::DaoFork => "DaoFork",
            Fork::Tangerine => "Tangerine",
            Fork::SpuriousDragon => "SpuriousDragon",
            Fork::Byzantium => "Byzantium",
            Fork::Constantinople => "Constantinople",
            Fork::Petersburg => "Petersburg",
            Fork::Istanbul => "Istanbul",
            Fork::MuirGlacier => "MuirGlacier",
            Fork::Berlin => "Berlin",
            Fork::London => "London",
            Fork::ArrowGlacier => "ArrowGlacier",
            Fork::GrayGlacier => "GrayGlacier",
            Fork::Paris => "Paris",
            Fork::Shanghai => "Shanghai",
            Fork::Cancun => "Cancun",
            Fork::Prague => "Prague",
            Fork::Osaka => "Osaka",
            Fork::BPO1 => "BPO1",
            Fork::BPO2 => "BPO2",
            Fork::BPO3 => "BPO3",
            Fork::BPO4 => "BPO4",
            Fork::BPO5 => "BPO5",
        }
    }
}

impl ChainConfig {
    pub fn is_bpo1_activated(&self, block_timestamp: u64) -> bool {
        self.bpo1_time.is_some_and(|time| time <= block_timestamp)
    }

    pub fn is_bpo2_activated(&self, block_timestamp: u64) -> bool {
        self.bpo2_time.is_some_and(|time| time <= block_timestamp)
    }

    pub fn is_bpo3_activated(&self, block_timestamp: u64) -> bool {
        self.bpo3_time.is_some_and(|time| time <= block_timestamp)
    }

    pub fn is_bpo4_activated(&self, block_timestamp: u64) -> bool {
        self.bpo4_time.is_some_and(|time| time <= block_timestamp)
    }

    pub fn is_bpo5_activated(&self, block_timestamp: u64) -> bool {
        self.bpo5_time.is_some_and(|time| time <= block_timestamp)
    }

    pub fn is_osaka_activated(&self, block_timestamp: u64) -> bool {
        self.osaka_time.is_some_and(|time| time <= block_timestamp)
    }

    pub fn is_prague_activated(&self, block_timestamp: u64) -> bool {
        self.prague_time.is_some_and(|time| time <= block_timestamp)
    }

    pub fn is_shanghai_activated(&self, block_timestamp: u64) -> bool {
        self.shanghai_time
            .is_some_and(|time| time <= block_timestamp)
    }

    pub fn is_cancun_activated(&self, block_timestamp: u64) -> bool {
        self.cancun_time.is_some_and(|time| time <= block_timestamp)
    }

    pub fn is_istanbul_activated(&self, block_number: BlockNumber) -> bool {
        self.istanbul_block.is_some_and(|num| num <= block_number)
    }

    pub fn is_london_activated(&self, block_number: BlockNumber) -> bool {
        self.london_block.is_some_and(|num| num <= block_number)
    }

    pub fn is_eip155_activated(&self, block_number: BlockNumber) -> bool {
        self.eip155_block.is_some_and(|num| num <= block_number)
    }

    pub fn display_config(&self) -> String {
        let network = NETWORK_NAMES.get(&self.chain_id).unwrap_or(&"unknown");
        let mut output = format!("Chain ID: {} ({})\n\n", self.chain_id, network);

        let post_merge_forks = [
            ("Shanghai", self.shanghai_time),
            ("Cancun", self.cancun_time),
            ("Prague", self.prague_time),
            ("Verkle", self.verkle_time),
            ("Osaka", self.osaka_time),
        ];

        let active_forks: Vec<_> = post_merge_forks
            .iter()
            .filter_map(|(name, t)| t.map(|time| format!("- {}: @{:<10}", name, time)))
            .collect();

        if !active_forks.is_empty() {
            output.push_str("Network is post-merge\n\n");
            output.push_str("Post-Merge hard forks (timestamp based):\n");
            output.push_str(&active_forks.join("\n"));
        } else {
            output.push_str("Network is at Paris\n\n");
        }

        output
    }

    pub fn get_fork(&self, block_timestamp: u64) -> Fork {
        if self.is_osaka_activated(block_timestamp) {
            Fork::Osaka
        } else if self.is_prague_activated(block_timestamp) {
            Fork::Prague
        } else if self.is_cancun_activated(block_timestamp) {
            Fork::Cancun
        } else if self.is_shanghai_activated(block_timestamp) {
            Fork::Shanghai
        } else {
            Fork::Paris
        }
    }

    pub fn get_fork_blob_schedule(&self, block_timestamp: u64) -> Option<ForkBlobSchedule> {
        if self.is_bpo5_activated(block_timestamp) {
            Some(self.blob_schedule.bpo5.unwrap_or_default())
        } else if self.is_bpo4_activated(block_timestamp) {
            Some(self.blob_schedule.bpo4.unwrap_or_default())
        } else if self.is_bpo3_activated(block_timestamp) {
            Some(self.blob_schedule.bpo3.unwrap_or_default())
        } else if self.is_bpo2_activated(block_timestamp) {
            Some(self.blob_schedule.bpo2)
        } else if self.is_bpo1_activated(block_timestamp) {
            Some(self.blob_schedule.bpo1)
        } else if self.is_osaka_activated(block_timestamp) {
            Some(self.blob_schedule.osaka)
        } else if self.is_prague_activated(block_timestamp) {
            Some(self.blob_schedule.prague)
        } else if self.is_cancun_activated(block_timestamp) {
            Some(self.blob_schedule.cancun)
        } else {
            None
        }
    }

    pub fn fork(&self, block_timestamp: u64) -> Fork {
        self.get_fork(block_timestamp)
    }

    pub fn next_fork(&self, block_timestamp: u64) -> Option<Fork> {
        let next = if self.is_bpo5_activated(block_timestamp) {
            None
        } else if self.is_bpo4_activated(block_timestamp) && self.bpo5_time.is_some() {
            Some(Fork::BPO5)
        } else if self.is_bpo3_activated(block_timestamp) && self.bpo4_time.is_some() {
            Some(Fork::BPO4)
        } else if self.is_bpo2_activated(block_timestamp) && self.bpo3_time.is_some() {
            Some(Fork::BPO3)
        } else if self.is_bpo1_activated(block_timestamp) && self.bpo2_time.is_some() {
            Some(Fork::BPO2)
        } else if self.is_osaka_activated(block_timestamp) && self.bpo1_time.is_some() {
            Some(Fork::BPO1)
        } else if self.is_prague_activated(block_timestamp) && self.osaka_time.is_some() {
            Some(Fork::Osaka)
        } else if self.is_cancun_activated(block_timestamp) && self.prague_time.is_some() {
            Some(Fork::Prague)
        } else if self.is_shanghai_activated(block_timestamp) && self.cancun_time.is_some() {
            Some(Fork::Cancun)
        } else {
            None
        };
        match next {
            Some(fork) if fork > self.fork(block_timestamp) => next,
            _ => None,
        }
    }

    pub fn get_last_scheduled_fork(&self) -> Fork {
        if self.bpo5_time.is_some() {
            Fork::BPO5
        } else if self.bpo4_time.is_some() {
            Fork::BPO4
        } else if self.bpo3_time.is_some() {
            Fork::BPO3
        } else if self.bpo2_time.is_some() {
            Fork::BPO2
        } else if self.bpo1_time.is_some() {
            Fork::BPO1
        } else if self.osaka_time.is_some() {
            Fork::Osaka
        } else if self.prague_time.is_some() {
            Fork::Prague
        } else if self.cancun_time.is_some() {
            Fork::Cancun
        } else {
            Fork::Paris
        }
    }

    pub fn get_activation_timestamp_for_fork(&self, fork: Fork) -> Option<u64> {
        match fork {
            Fork::Cancun => self.cancun_time,
            Fork::Prague => self.prague_time,
            Fork::Osaka => self.osaka_time,
            Fork::BPO1 => self.bpo1_time,
            Fork::BPO2 => self.bpo2_time,
            Fork::BPO3 => self.bpo3_time,
            Fork::BPO4 => self.bpo4_time,
            Fork::BPO5 => self.bpo5_time,
            Fork::Homestead => self.homestead_block,
            Fork::DaoFork => self.dao_fork_block,
            Fork::Byzantium => self.byzantium_block,
            Fork::Constantinople => self.constantinople_block,
            Fork::Petersburg => self.petersburg_block,
            Fork::Istanbul => self.istanbul_block,
            Fork::MuirGlacier => self.muir_glacier_block,
            Fork::Berlin => self.berlin_block,
            Fork::London => self.london_block,
            Fork::ArrowGlacier => self.arrow_glacier_block,
            Fork::GrayGlacier => self.gray_glacier_block,
            Fork::Paris => self.merge_netsplit_block,
            Fork::Shanghai => self.shanghai_time,
            _ => None,
        }
    }

    pub fn get_blob_schedule_for_fork(&self, fork: Fork) -> Option<ForkBlobSchedule> {
        match fork {
            Fork::Cancun => Some(self.blob_schedule.cancun),
            Fork::Prague => Some(self.blob_schedule.prague),
            Fork::Osaka => Some(self.blob_schedule.osaka),
            Fork::BPO1 => self.blob_schedule.bpo1,
            Fork::BPO2 => self.blob_schedule.bpo2,
            Fork::BPO3 => self.blob_schedule.bpo3,
            Fork::BPO4 => self.blob_schedule.bpo4,
            Fork::BPO5 => self.blob_schedule.bpo5,
            _ => None,
        }
    }

    pub fn gather_forks(&self, genesis_header: BlockHeader) -> (Vec<u64>, Vec<u64>) {
        let mut block_number_based_forks: Vec<u64> = vec![
            self.homestead_block,
            if self.dao_fork_support {
                self.dao_fork_block
            } else {
                None
            },
            self.eip150_block,
            self.eip155_block,
            self.eip158_block,
            self.byzantium_block,
            self.constantinople_block,
            self.petersburg_block,
            self.istanbul_block,
            self.muir_glacier_block,
            self.berlin_block,
            self.london_block,
            self.arrow_glacier_block,
            self.gray_glacier_block,
            self.merge_netsplit_block,
        ]
        .into_iter()
        .flatten()
        .collect();

        // Remove repeated values
        block_number_based_forks.sort();
        block_number_based_forks.dedup();

        let mut timestamp_based_forks: Vec<u64> = vec![
            self.shanghai_time,
            self.cancun_time,
            self.prague_time,
            self.osaka_time,
            self.bpo1_time,
            self.bpo2_time,
            self.bpo3_time,
            self.bpo4_time,
            self.bpo5_time,
            self.verkle_time,
        ]
        .into_iter()
        .flatten()
        .collect();

        // Remove repeated values
        timestamp_based_forks.sort();
        timestamp_based_forks.dedup();

        // Filter forks before genesis
        block_number_based_forks.retain(|block_number| *block_number != 0);
        timestamp_based_forks.retain(|block_timestamp| *block_timestamp > genesis_header.timestamp);

        (block_number_based_forks, timestamp_based_forks)
    }
}

#[allow(unused)]
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GenesisAccount {
    #[serde(default, with = "crate::serde_utils::bytes")]
    pub code: Bytes,
    #[serde(default)]
    pub storage: HashMap<U256, U256>,
    #[serde(deserialize_with = "crate::serde_utils::u256::deser_hex_or_dec_str")]
    pub balance: U256,
    #[serde(default, with = "crate::serde_utils::u64::hex_str")]
    pub nonce: u64,
}

impl Genesis {
    pub fn get_block(&self) -> Block {
        Block::new(self.get_block_header(), self.get_block_body())
    }

    fn get_block_header(&self) -> BlockHeader {
        let mut blob_gas_used: Option<u64> = None;
        let mut excess_blob_gas: Option<u64> = None;

        if let Some(cancun_time) = self.config.cancun_time {
            if cancun_time <= self.timestamp {
                blob_gas_used = Some(self.blob_gas_used.unwrap_or(0));
                excess_blob_gas = Some(self.excess_blob_gas.unwrap_or(0));
            }
        }
        let base_fee_per_gas = self.base_fee_per_gas.or_else(|| {
            self.config
                .is_london_activated(0)
                .then_some(INITIAL_BASE_FEE)
        });

        let withdrawals_root = self
            .config
            .is_shanghai_activated(self.timestamp)
            .then_some(compute_withdrawals_root(&[]));

        let parent_beacon_block_root = self
            .config
            .is_cancun_activated(self.timestamp)
            .then_some(H256::zero());

        let requests_hash = self
            .config
            .is_prague_activated(self.timestamp)
            .then_some(self.requests_hash.unwrap_or(*DEFAULT_REQUESTS_HASH));

        BlockHeader {
            parent_hash: H256::zero(),
            ommers_hash: *DEFAULT_OMMERS_HASH,
            coinbase: self.coinbase,
            state_root: self.compute_state_root(),
            transactions_root: compute_transactions_root(&[]),
            receipts_root: compute_receipts_root(&[]),
            logs_bloom: Bloom::zero(),
            difficulty: self.difficulty,
            number: 0,
            gas_limit: self.gas_limit,
            gas_used: 0,
            timestamp: self.timestamp,
            extra_data: self.extra_data.clone(),
            prev_randao: self.mix_hash,
            nonce: self.nonce,
            base_fee_per_gas,
            withdrawals_root,
            blob_gas_used,
            excess_blob_gas,
            parent_beacon_block_root,
            requests_hash,
            ..Default::default()
        }
    }

    fn get_block_body(&self) -> BlockBody {
        BlockBody {
            transactions: vec![],
            ommers: vec![],
            withdrawals: Some(vec![]),
        }
    }

    pub fn compute_state_root(&self) -> H256 {
        let iter = self.alloc.iter().map(|(addr, account)| {
            (
                Keccak256::digest(addr).to_vec(),
                AccountState::from(account).encode_to_vec(),
            )
        });
        Trie::compute_hash_from_unsorted_iter(iter)
    }
}
#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use std::{fs::File, io::BufReader};

    use ethereum_types::H160;

    use crate::types::INITIAL_BASE_FEE;

    use super::*;

    #[test]
    fn deserialize_genesis_file() {
        // Deserialize genesis file
        let file = File::open("../../fixtures/genesis/kurtosis.json")
            .expect("Failed to open genesis file");
        let reader = BufReader::new(file);
        let genesis: Genesis =
            serde_json::from_reader(reader).expect("Failed to deserialize genesis file");
        // Check Genesis fields
        // Chain config
        let expected_chain_config = ChainConfig {
            chain_id: 3151908_u64,
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
            shanghai_time: Some(0),
            cancun_time: Some(0),
            prague_time: Some(1718232101),
            terminal_total_difficulty: Some(0),
            terminal_total_difficulty_passed: true,
            deposit_contract_address: H160::from_str("0x4242424242424242424242424242424242424242")
                .unwrap(),
            // Note this BlobSchedule config is not the default
            blob_schedule: BlobSchedule {
                cancun: ForkBlobSchedule {
                    target: 2,
                    max: 3,
                    base_fee_update_fraction: 6676954,
                },
                prague: ForkBlobSchedule {
                    target: 3,
                    max: 4,
                    base_fee_update_fraction: 13353908,
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(&genesis.config, &expected_chain_config);
        // Genesis header fields
        assert_eq!(genesis.coinbase, Address::from([0; 20]));
        assert_eq!(genesis.difficulty, U256::from(1));
        assert!(genesis.extra_data.is_empty());
        assert_eq!(genesis.gas_limit, 0x17d7840);
        assert_eq!(genesis.nonce, 0x1234);
        assert_eq!(genesis.mix_hash, H256::from([0; 32]));
        assert_eq!(genesis.timestamp, 1718040081);
        // Check alloc field
        // We will only check a couple of the hashmap's values as it is quite large
        let addr_a = Address::from_str("0x000F3df6D732807Ef1319fB7B8bB8522d0Beac02").unwrap();
        assert!(genesis.alloc.contains_key(&addr_a));
        let expected_account_a = GenesisAccount {
        code: Bytes::from(hex::decode("3373fffffffffffffffffffffffffffffffffffffffe14604d57602036146024575f5ffd5b5f35801560495762001fff810690815414603c575f5ffd5b62001fff01545f5260205ff35b5f5ffd5b62001fff42064281555f359062001fff015500").unwrap()),
        balance: 0.into(),
        nonce: 1,
        storage: Default::default(),
    };
        assert_eq!(genesis.alloc[&addr_a], expected_account_a);
        // Check some storage values from another account
        let addr_b = Address::from_str("0x4242424242424242424242424242424242424242").unwrap();
        assert!(genesis.alloc.contains_key(&addr_b));
        let addr_b_storage = &genesis.alloc[&addr_b].storage;
        assert_eq!(
            addr_b_storage.get(
                &U256::from_str(
                    "0x0000000000000000000000000000000000000000000000000000000000000022"
                )
                .unwrap()
            ),
            Some(
                &U256::from_str(
                    "0xf5a5fd42d16a20302798ef6ed309979b43003d2320d9f0e8ea9831a92759fb4b"
                )
                .unwrap()
            )
        );
        assert_eq!(
            addr_b_storage.get(
                &U256::from_str(
                    "0x0000000000000000000000000000000000000000000000000000000000000038"
                )
                .unwrap()
            ),
            Some(
                &U256::from_str(
                    "0xe71f0aa83cc32edfbefa9f4d3e0174ca85182eec9f3a09f6a6c0df6377a510d7"
                )
                .unwrap()
            )
        );
    }

    #[test]
    fn genesis_block() {
        // Deserialize genesis file
        let file = File::open("../../fixtures/genesis/kurtosis.json")
            .expect("Failed to open genesis file");
        let reader = BufReader::new(file);
        let genesis: Genesis =
            serde_json::from_reader(reader).expect("Failed to deserialize genesis file");
        let genesis_block = genesis.get_block();
        let header = genesis_block.header;
        let body = genesis_block.body;
        assert_eq!(header.parent_hash, H256::from([0; 32]));
        assert_eq!(header.ommers_hash, *DEFAULT_OMMERS_HASH);
        assert_eq!(header.coinbase, Address::default());
        assert_eq!(
            header.state_root,
            H256::from_str("0x2dab6a1d6d638955507777aecea699e6728825524facbd446bd4e86d44fa5ecd")
                .unwrap()
        );
        assert_eq!(header.transactions_root, compute_transactions_root(&[]));
        assert_eq!(header.receipts_root, compute_receipts_root(&[]));
        assert_eq!(header.logs_bloom, Bloom::default());
        assert_eq!(header.difficulty, U256::from(1));
        assert_eq!(header.gas_limit, 25_000_000);
        assert_eq!(header.gas_used, 0);
        assert_eq!(header.timestamp, 1_718_040_081);
        assert_eq!(header.extra_data, Bytes::default());
        assert_eq!(header.prev_randao, H256::from([0; 32]));
        assert_eq!(header.nonce, 4660);
        assert_eq!(
            header.base_fee_per_gas.unwrap_or(INITIAL_BASE_FEE),
            INITIAL_BASE_FEE
        );
        assert_eq!(header.withdrawals_root, Some(compute_withdrawals_root(&[])));
        assert_eq!(header.blob_gas_used, Some(0));
        assert_eq!(header.excess_blob_gas, Some(0));
        assert_eq!(header.parent_beacon_block_root, Some(H256::zero()));
        assert!(body.transactions.is_empty());
        assert!(body.ommers.is_empty());
        assert!(body.withdrawals.is_some_and(|w| w.is_empty()));
    }

    #[test]
    // Parses genesis received by kurtosis and checks that the hash matches the next block's parent hash
    fn read_and_compute_kurtosis_hash() {
        let file = File::open("../../fixtures/genesis/kurtosis.json")
            .expect("Failed to open genesis file");
        let reader = BufReader::new(file);
        let genesis: Genesis =
            serde_json::from_reader(reader).expect("Failed to deserialize genesis file");
        let genesis_block_hash = genesis.get_block().hash();
        assert_eq!(
            genesis_block_hash,
            H256::from_str("0xcb5306dd861d0f2c1f9952fbfbc75a46d0b6ce4f37bea370c3471fe8410bf40b")
                .unwrap()
        )
    }

    #[test]
    fn parse_hive_genesis_file() {
        let file =
            File::open("../../fixtures/genesis/hive.json").expect("Failed to open genesis file");
        let reader = BufReader::new(file);
        let _genesis: Genesis =
            serde_json::from_reader(reader).expect("Failed to deserialize genesis file");
    }

    #[test]
    fn read_and_compute_hive_hash() {
        let file =
            File::open("../../fixtures/genesis/hive.json").expect("Failed to open genesis file");
        let reader = BufReader::new(file);
        let genesis: Genesis =
            serde_json::from_reader(reader).expect("Failed to deserialize genesis file");
        let computed_block_hash = genesis.get_block().hash();
        let genesis_block_hash =
            H256::from_str("0x30f516e34fc173bb5fc4daddcc7532c4aca10b702c7228f3c806b4df2646fb7e")
                .unwrap();
        assert_eq!(genesis_block_hash, computed_block_hash)
    }

    #[test]
    fn deserialize_chain_config_blob_schedule() {
        let json = r#"

            {
                "chainId": 123,
                "blobSchedule": {
                  "cancun": {
                    "target": 1,
                    "max": 2,
                    "baseFeeUpdateFraction": 10000
                  },
                  "prague": {
                    "target": 3,
                    "max": 4,
                    "baseFeeUpdateFraction": 20000
                  }
                },
                "depositContractAddress": "0x4242424242424242424242424242424242424242"
            }
            "#;

        let config: ChainConfig =
            serde_json::from_str(json).expect("Failed to deserialize ChainConfig");
        let expected_chain_config = ChainConfig {
            chain_id: 123,
            blob_schedule: BlobSchedule {
                cancun: ForkBlobSchedule {
                    target: 1,
                    max: 2,
                    base_fee_update_fraction: 10000,
                },
                prague: ForkBlobSchedule {
                    target: 3,
                    max: 4,
                    base_fee_update_fraction: 20000,
                },
                ..Default::default()
            },
            deposit_contract_address: H160::from_str("0x4242424242424242424242424242424242424242")
                .unwrap(),
            ..Default::default()
        };
        assert_eq!(&config, &expected_chain_config);
    }

    #[test]
    fn deserialize_chain_config_missing_entire_blob_schedule() {
        let json = r#"
            {
                "chainId": 123,
                "depositContractAddress": "0x4242424242424242424242424242424242424242"
            }
            "#;

        let config: ChainConfig =
            serde_json::from_str(json).expect("Failed to deserialize ChainConfig");
        let expected_chain_config = ChainConfig {
            chain_id: 123,
            blob_schedule: BlobSchedule {
                cancun: ForkBlobSchedule {
                    target: 3,
                    max: 6,
                    base_fee_update_fraction: 3338477,
                },
                prague: ForkBlobSchedule {
                    target: 6,
                    max: 9,
                    base_fee_update_fraction: 5007716,
                },
                ..Default::default()
            },
            deposit_contract_address: H160::from_str("0x4242424242424242424242424242424242424242")
                .unwrap(),
            ..Default::default()
        };
        assert_eq!(&config, &expected_chain_config);
    }

    #[test]
    fn deserialize_chain_config_missing_cancun_blob_schedule() {
        let json = r#"
            {
                "chainId": 123,
                "blobSchedule": {
                    "prague": {
                      "target": 3,
                      "max": 4,
                      "baseFeeUpdateFraction": 20000
                    }
                },
                "depositContractAddress": "0x4242424242424242424242424242424242424242"
            }
            "#;

        let config: ChainConfig =
            serde_json::from_str(json).expect("Failed to deserialize ChainConfig");
        let expected_chain_config = ChainConfig {
            chain_id: 123,
            blob_schedule: BlobSchedule {
                cancun: ForkBlobSchedule {
                    target: 3,
                    max: 6,
                    base_fee_update_fraction: 3338477,
                },
                prague: ForkBlobSchedule {
                    target: 3,
                    max: 4,
                    base_fee_update_fraction: 20000,
                },
                ..Default::default()
            },
            deposit_contract_address: H160::from_str("0x4242424242424242424242424242424242424242")
                .unwrap(),
            ..Default::default()
        };
        assert_eq!(&config, &expected_chain_config);
    }

    #[test]
    fn deserialize_chain_config_missing_prague_blob_schedule() {
        let json = r#"
            {
                "chainId": 123,
                "blobSchedule": {
                  "cancun": {
                    "target": 1,
                    "max": 2,
                    "baseFeeUpdateFraction": 10000
                  }
                },
                "depositContractAddress": "0x4242424242424242424242424242424242424242"
            }
            "#;

        let config: ChainConfig =
            serde_json::from_str(json).expect("Failed to deserialize ChainConfig");
        let expected_chain_config = ChainConfig {
            chain_id: 123,
            blob_schedule: BlobSchedule {
                cancun: ForkBlobSchedule {
                    target: 1,
                    max: 2,
                    base_fee_update_fraction: 10000,
                },
                prague: ForkBlobSchedule {
                    target: 6,
                    max: 9,
                    base_fee_update_fraction: 5007716,
                },
                ..Default::default()
            },
            deposit_contract_address: H160::from_str("0x4242424242424242424242424242424242424242")
                .unwrap(),
            ..Default::default()
        };
        assert_eq!(&config, &expected_chain_config);
    }

    #[test]
    fn deserialize_chain_config_missing_deposit_contract_address() {
        let json = r#"
            {
                "chainId": 123
            }
            "#;

        let result: Result<ChainConfig, _> = serde_json::from_str(json);

        assert!(result.is_err());

        let error_message = result.unwrap_err().to_string();
        assert!(error_message.contains("missing field `depositContractAddress`"),);
    }
}
