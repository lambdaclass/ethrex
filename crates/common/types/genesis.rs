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
    ops::{Index, IndexMut},
    path::Path,
};
use tracing::warn;

use self::Fork::*;
use super::{
    AccountState, Block, BlockBody, BlockHeader, INITIAL_BASE_FEE, compute_receipts_root,
    compute_transactions_root, compute_withdrawals_root,
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

        if genesis.config.fork_activation_timestamps[BPO3].is_some()
            && genesis.config.blob_schedule[BPO3].is_none()
            || genesis.config.fork_activation_timestamps[BPO4].is_some()
                && genesis.config.blob_schedule[BPO4].is_none()
            || genesis.config.fork_activation_timestamps[BPO5].is_some()
                && genesis.config.blob_schedule[BPO5].is_none()
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
    pub fork: Fork,
    pub base_fee_update_fraction: u64,
    pub max: u32,
    pub target: u32,
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
    /// We don't need these to check if certain forks are activated given Ethrex is post-merge,
    /// but we need the numbers to calculate the ForkID on some networks (namely mainnet)
    pub fork_activation_blocks: [Option<u64>; PRE_MERGE_FORKS],

    /// Timestamp at which each fork was activated
    /// (None = no fork, 0 = fork is already active)
    pub fork_activation_timestamps: [Option<u64>; FORKS.len()],

    /// Amount of total difficulty reached by the network that triggers the consensus upgrade.
    pub terminal_total_difficulty: Option<u128>,
    /// Network has already passed the terminal total difficult
    #[serde(default)]
    pub terminal_total_difficulty_passed: bool,
    #[serde(default)]
    pub blob_schedule: [Option<ForkBlobSchedule>; FORKS.len() - Cancun as usize],
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
#[derive(
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Default,
    Hash,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    RSerialize,
    RDeserialize,
    Archive,
)]
pub enum Fork {
    Paris = 0,
    Shanghai = 1,
    #[default]
    Cancun = 2,
    Prague = 3,
    Osaka = 4,
    BPO1 = 5,
    BPO2 = 6,
    BPO3 = 7,
    BPO4 = 8,
    BPO5 = 9,
}

impl<T> Index<Fork> for [T] {
    type Output = T;
    fn index(&self, fork: Fork) -> &Self::Output {
        &self[fork as usize]
    }
}

impl<T> IndexMut<Fork> for [T] {
    fn index_mut(&mut self, fork: Fork) -> &mut Self::Output {
        &mut self[fork as usize]
    }
}

pub const FORKS: [Fork; 10] = [
    Paris, Shanghai, Cancun, Prague, Osaka, BPO1, BPO2, BPO3, BPO4, BPO5,
];

pub const PRE_MERGE_FORKS: usize = 15;

impl From<Fork> for &str {
    fn from(fork: Fork) -> Self {
        match fork {
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
    pub fn is_fork_activated(&self, fork: Fork, block_timestamp: u64) -> bool {
        self.fork_activation_timestamps[fork]
            .is_some_and(|activation_time| block_timestamp >= activation_time)
    }

    pub fn display_config(&self) -> String {
        let network = NETWORK_NAMES.get(&self.chain_id).unwrap_or(&"unknown");
        let mut output = format!("Chain ID: {} ({})\n\n", self.chain_id, network);

        let post_merge_forks = [
            ("Shanghai", self.fork_activation_timestamps[Shanghai]),
            ("Cancun", self.fork_activation_timestamps[Cancun]),
            ("Prague", self.fork_activation_timestamps[Prague]),
            ("Osaka", self.fork_activation_timestamps[Osaka]),
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
        let Some(index) = self
            .fork_activation_timestamps
            .iter()
            .rposition(|possible_time| {
                possible_time.is_some_and(|activation_time| activation_time <= block_timestamp)
            })
        else {
            return Paris;
        };
        FORKS[index]
    }

    pub fn get_fork_blob_schedule(&self, block_timestamp: u64) -> Option<ForkBlobSchedule> {
        let current_fork = self.get_fork(block_timestamp);
        self.get_blob_schedule_for_fork(current_fork)
    }

    pub fn fork(&self, block_timestamp: u64) -> Fork {
        self.get_fork(block_timestamp)
    }

    pub fn next_fork(&self, block_timestamp: u64) -> Option<Fork> {
        let Some(index) = self
            .fork_activation_timestamps
            .iter()
            .position(|possible_time| {
                possible_time.is_some_and(|activation_time| activation_time > block_timestamp)
            })
        else {
            return None;
        };
        Some(FORKS[index])
    }

    pub fn get_last_scheduled_fork(&self) -> Fork {
        let Some(index) = self
            .fork_activation_timestamps
            .iter()
            .rposition(|possible_time| possible_time.is_some())
        else {
            return Paris;
        };
        FORKS[index]
    }

    pub fn get_activation_timestamp_for_fork(&self, fork: Fork) -> Option<u64> {
        self.fork_activation_timestamps[fork]
    }

    pub fn get_blob_schedule_for_fork(&self, fork: Fork) -> Option<ForkBlobSchedule> {
        *self.blob_schedule[0..fork as usize]
            .iter()
            .rfind(|option| {
                option.is_some_and(|fork_blob_schedule| fork_blob_schedule.fork <= fork)
            })
            .unwrap_or(&None)
    }

    pub fn gather_forks(&self, genesis_header: BlockHeader) -> (Vec<u64>, Vec<u64>) {
        let mut block_number_based_forks: Vec<u64> = self
            .fork_activation_blocks
            .to_vec()
            .into_iter()
            .flatten()
            .collect();

        // Remove repeated values
        block_number_based_forks.sort();
        block_number_based_forks.dedup();

        let mut timestamp_based_forks: Vec<u64> = self
            .fork_activation_timestamps
            .to_vec()
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

        if let Some(cancun_time) = self.config.fork_activation_timestamps[Cancun]
            && cancun_time <= self.timestamp
        {
            blob_gas_used = Some(self.blob_gas_used.unwrap_or(0));
            excess_blob_gas = Some(self.excess_blob_gas.unwrap_or(0));
        }
        let base_fee_per_gas = self.base_fee_per_gas.or(Some(INITIAL_BASE_FEE));

        let withdrawals_root = self
            .config
            .is_fork_activated(Shanghai, self.timestamp)
            .then_some(compute_withdrawals_root(&[]));

        let parent_beacon_block_root = self
            .config
            .is_fork_activated(Cancun, self.timestamp)
            .then_some(H256::zero());

        let requests_hash = self
            .config
            .is_fork_activated(Prague, self.timestamp)
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
        let mut blob_schedule: [Option<ForkBlobSchedule>; 8] = [None; 8];
        blob_schedule[0] = Some(ForkBlobSchedule {
            fork: Cancun,
            target: 2,
            max: 3,
            base_fee_update_fraction: 6676954,
        });
        blob_schedule[1] = Some(ForkBlobSchedule {
            fork: Prague,
            target: 3,
            max: 4,
            base_fee_update_fraction: 13353908,
        });

        let mut fork_activation_timestamps: [Option<u64>; FORKS.len()] = [None; FORKS.len()];
        fork_activation_timestamps[Shanghai] = Some(0);
        fork_activation_timestamps[Cancun] = Some(0);
        fork_activation_timestamps[Prague] = Some(1718232101);

        let fork_activation_blocks: [Option<u64>; 15] = [None; 15];

        // Check Genesis fields
        // Chain config
        let expected_chain_config = ChainConfig {
            chain_id: 3151908_u64,
            fork_activation_blocks,
            fork_activation_timestamps,
            terminal_total_difficulty: Some(0),
            terminal_total_difficulty_passed: true,
            deposit_contract_address: H160::from_str("0x4242424242424242424242424242424242424242")
                .unwrap(),
            // Note this BlobSchedule config is not the default
            blob_schedule,
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
        let mut blob_schedule: [Option<ForkBlobSchedule>; 8] = [None; 8];
        blob_schedule[0] = Some(ForkBlobSchedule {
            fork: Cancun,
            target: 1,
            max: 2,
            base_fee_update_fraction: 10000,
        });
        blob_schedule[1] = Some(ForkBlobSchedule {
            fork: Prague,
            target: 3,
            max: 4,
            base_fee_update_fraction: 20000,
        });

        let expected_chain_config = ChainConfig {
            chain_id: 123,
            blob_schedule,
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
        let mut blob_schedule: [Option<ForkBlobSchedule>; 8] = [None; 8];
        blob_schedule[0] = Some(ForkBlobSchedule {
            fork: Cancun,
            target: 3,
            max: 6,
            base_fee_update_fraction: 3338477,
        });
        blob_schedule[1] = Some(ForkBlobSchedule {
            fork: Prague,
            target: 6,
            max: 9,
            base_fee_update_fraction: 5007716,
        });
        let expected_chain_config = ChainConfig {
            chain_id: 123,
            blob_schedule,
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
        let mut blob_schedule: [Option<ForkBlobSchedule>; 8] = [None; 8];
        blob_schedule[0] = Some(ForkBlobSchedule {
            fork: Cancun,
            target: 3,
            max: 6,
            base_fee_update_fraction: 3338477,
        });
        blob_schedule[1] = Some(ForkBlobSchedule {
            fork: Prague,
            target: 3,
            max: 4,
            base_fee_update_fraction: 20000,
        });
        let expected_chain_config = ChainConfig {
            chain_id: 123,
            blob_schedule,
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
        let mut blob_schedule: [Option<ForkBlobSchedule>; 8] = [None; 8];
        blob_schedule[0] = Some(ForkBlobSchedule {
            fork: Cancun,
            target: 1,
            max: 2,
            base_fee_update_fraction: 10000,
        });
        blob_schedule[1] = Some(ForkBlobSchedule {
            fork: Prague,
            target: 6,
            max: 9,
            base_fee_update_fraction: 5007716,
        });
        let expected_chain_config = ChainConfig {
            chain_id: 123,
            blob_schedule,
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
