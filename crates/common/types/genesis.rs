use bytes::Bytes;
use ethereum_rust_trie::Trie;
use ethereum_types::{Address, Bloom, H256, U256};
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use std::collections::HashMap;

use ethereum_rust_rlp::encode::RLPEncode;

use super::{
    compute_receipts_root, compute_transactions_root, compute_withdrawals_root, AccountState,
    Block, BlockBody, BlockHeader, BlockNumber, DEFAULT_OMMERS_HASH, INITIAL_BASE_FEE,
};

#[allow(unused)]
#[derive(Debug, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Genesis {
    /// Chain configuration
    pub config: ChainConfig,
    /// The initial state of the accounts in the genesis block.
    pub alloc: HashMap<Address, GenesisAccount>,
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
    pub timestamp: u64,
    #[serde(default, with = "crate::serde_utils::u64::hex_str_opt")]
    pub base_fee_per_gas: Option<u64>,
    #[serde(default, with = "crate::serde_utils::u64::hex_str_opt")]
    pub blob_gas_used: Option<u64>,
    #[serde(default, with = "crate::serde_utils::u64::hex_str_opt")]
    pub excess_blob_gas: Option<u64>,
}

/// Blockchain settings defined per block
#[allow(unused)]
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Default, PartialEq)]
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

    /// Amount of total difficulty reached by the network that triggers the consensus upgrade.
    pub terminal_total_difficulty: Option<u128>,
    /// Network has already passed the terminal total difficult
    #[serde(default)]
    pub terminal_total_difficulty_passed: bool,
}

#[derive(Debug, PartialEq, PartialOrd)]
pub enum Fork {
    Paris = 0,
    Shanghai = 1,
    Cancun = 2,
}

impl ChainConfig {
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

    pub fn is_eip155_activated(&self, block_number: BlockNumber) -> bool {
        self.eip155_block.is_some_and(|num| num <= block_number)
    }

    pub fn get_fork(&self, block_timestamp: u64) -> Fork {
        if self.is_cancun_activated(block_timestamp) {
            Fork::Cancun
        } else if self.is_shanghai_activated(block_timestamp) {
            Fork::Shanghai
        } else {
            Fork::Paris
        }
    }

    pub fn gather_forks(&self) -> (Vec<Option<u64>>, Vec<Option<u64>>) {
        let block_number_based_forks: Vec<Option<u64>> = vec![
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
        ];

        let timestamp_based_forks: Vec<Option<u64>> = vec![
            self.shanghai_time,
            self.cancun_time,
            self.prague_time,
            self.verkle_time,
        ];

        (block_number_based_forks, timestamp_based_forks)
    }
}

#[allow(unused)]
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct GenesisAccount {
    #[serde(default, with = "crate::serde_utils::bytes")]
    pub code: Bytes,
    #[serde(default)]
    pub storage: HashMap<H256, U256>,
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
            base_fee_per_gas: self.base_fee_per_gas.or(Some(INITIAL_BASE_FEE)),
            withdrawals_root: self
                .config
                .is_shanghai_activated(self.timestamp)
                .then_some(compute_withdrawals_root(&[])),
            blob_gas_used,
            excess_blob_gas,
            parent_beacon_block_root: self
                .config
                .is_cancun_activated(self.timestamp)
                .then_some(H256::zero()),
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

    use crate::types::INITIAL_BASE_FEE;

    use super::*;

    #[test]
    fn deserialize_genesis_file() {
        // Deserialize genesis file
        let file = File::open("../../test_data/genesis-kurtosis.json")
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
                &H256::from_str(
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
                &H256::from_str(
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
        let file = File::open("../../test_data/genesis-kurtosis.json")
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
        let file = File::open("../../test_data/genesis-kurtosis.json")
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
            File::open("../../test_data/genesis-hive.json").expect("Failed to open genesis file");
        let reader = BufReader::new(file);
        let _genesis: Genesis =
            serde_json::from_reader(reader).expect("Failed to deserialize genesis file");
    }

    #[test]
    fn read_and_compute_hive_hash() {
        let file =
            File::open("../../test_data/genesis-hive.json").expect("Failed to open genesis file");
        let reader = BufReader::new(file);
        let genesis: Genesis =
            serde_json::from_reader(reader).expect("Failed to deserialize genesis file");
        let computed_block_hash = genesis.get_block().hash();
        let genesis_block_hash =
            H256::from_str("0x30f516e34fc173bb5fc4daddcc7532c4aca10b702c7228f3c806b4df2646fb7e")
                .unwrap();
        assert_eq!(genesis_block_hash, computed_block_hash)
    }
}
