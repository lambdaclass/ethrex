use std::collections::BTreeMap;
use std::str::FromStr;

use ethereum_types::{Address, Bloom, H256, U256};

use ethrex_common::constants::DEFAULT_OMMERS_HASH;
use ethrex_common::types::{
    Block, BlockBody, BlockHeader, ChainConfig, Genesis, GenesisAccount, compute_receipts_root,
    compute_transactions_root,
};
use ethrex_crypto::NativeCrypto;

use crate::bor_config::BorConfig;

/// Polygon mainnet chain ID.
pub const POLYGON_MAINNET_CHAIN_ID: u64 = 137;
/// Amoy testnet chain ID.
pub const AMOY_CHAIN_ID: u64 = 80002;

/// Returns true if the chain ID is a known Polygon PoS network.
pub fn is_polygon_chain(chain_id: u64) -> bool {
    chain_id == POLYGON_MAINNET_CHAIN_ID || chain_id == AMOY_CHAIN_ID
}

/// Expected genesis hashes from Bor source.
pub const POLYGON_MAINNET_GENESIS_HASH: &str =
    "a9c28ce2141b56c474f1dc504bee9b01eb1bd7d1a507580d5519d4437a97de1b";
pub const AMOY_GENESIS_HASH: &str =
    "7202b2b53c5a0836e773e319d18922cc756dd67432f9a1f65352b61f4406c697";

/// Raw JSON alloc data embedded at compile time.
const POLYGON_MAINNET_ALLOC: &str = include_str!("../allocs/bor_mainnet.json");
const AMOY_ALLOC: &str = include_str!("../allocs/amoy.json");

/// Parses an alloc JSON file (address -> GenesisAccount) into a BTreeMap.
fn parse_alloc(json: &str) -> BTreeMap<Address, GenesisAccount> {
    let raw: std::collections::HashMap<String, GenesisAccount> =
        serde_json::from_str(json).expect("alloc JSON should be valid");
    let mut alloc = BTreeMap::new();
    for (addr_str, account) in raw {
        let addr_with_prefix = if addr_str.starts_with("0x") || addr_str.starts_with("0X") {
            addr_str
        } else {
            format!("0x{addr_str}")
        };
        let addr = Address::from_str(&addr_with_prefix).expect("alloc address should be valid");
        alloc.insert(addr, account);
    }
    alloc
}

/// Constructs the BorConfig for Polygon mainnet (chain 137).
///
/// Reference: bor/params/config.go `BorMainnetChainConfig`
pub fn polygon_mainnet_bor_config() -> BorConfig {
    serde_json::from_str(
        r#"{
            "period": {"0": 2, "80084800": 1},
            "producerDelay": {"0": 6, "38189056": 4},
            "sprint": {"0": 64, "38189056": 16},
            "backupMultiplier": {"0": 2},
            "validatorContract": "0x0000000000000000000000000000000000001000",
            "stateReceiverContract": "0x0000000000000000000000000000000000001001",
            "burntContract": {
                "23850000": "0x70bca57f4579f58670ab2d18ef16e02c17553c38",
                "50523000": "0x7A8ed27F4C30512326878652d20fC85727401854",
                "83756500": "0x3ef57def668054dd750bd260526105c4eeef104f"
            },
            "coinbase": {
                "0": "0x0000000000000000000000000000000000000000",
                "77414656": "0x7Ee41D8A25641000661B1EF5E6AE8A00400466B0"
            },
            "stateSyncConfirmationDelay": {"44934656": 128},
            "istanbulBlock": 3395000,
            "berlinBlock": 14750000,
            "londonBlock": 23850000,
            "shanghaiBlock": 50523000,
            "cancunBlock": 54876000,
            "pragueBlock": 73440256,
            "jaipurBlock": 23850000,
            "delhiBlock": 38189056,
            "indoreBlock": 44934656,
            "ahmedabadBlock": 62278656,
            "bhilaiBlock": 73440256,
            "rioBlock": 77414656,
            "madhugiriBlock": 80084800,
            "madhugiriProBlock": 80084800,
            "dandeliBlock": 81424000,
            "lisovoBlock": 83756500,
            "lisovoProBlock": 83756500
        }"#,
    )
    .expect("mainnet BorConfig JSON should be valid")
}

/// Constructs the BorConfig for Amoy testnet (chain 80002).
///
/// Reference: bor/params/config.go `AmoyChainConfig`
pub fn amoy_bor_config() -> BorConfig {
    serde_json::from_str(
        r#"{
            "period": {"0": 2, "28899616": 1},
            "producerDelay": {"0": 4},
            "sprint": {"0": 16},
            "backupMultiplier": {"0": 2},
            "validatorContract": "0x0000000000000000000000000000000000001000",
            "stateReceiverContract": "0x0000000000000000000000000000000000001001",
            "burntContract": {
                "0": "0x000000000000000000000000000000000000dead",
                "73100": "0xeCDD77cE6f146cCf5dab707941d318Bd50eeD2C9"
            },
            "coinbase": {
                "0": "0x0000000000000000000000000000000000000000",
                "26272256": "0x7Ee41D8A25641000661B1EF5E6AE8A00400466B0"
            },
            "stateSyncConfirmationDelay": {"0": 128},
            "istanbulBlock": 0,
            "berlinBlock": 0,
            "londonBlock": 73100,
            "shanghaiBlock": 73100,
            "cancunBlock": 5423600,
            "pragueBlock": 22765056,
            "jaipurBlock": 73100,
            "delhiBlock": 73100,
            "indoreBlock": 73100,
            "ahmedabadBlock": 11865856,
            "bhilaiBlock": 22765056,
            "rioBlock": 26272256,
            "madhugiriBlock": 28899616,
            "madhugiriProBlock": 29287400,
            "dandeliBlock": 31890000,
            "lisovoBlock": 33634700,
            "lisovoProBlock": 34062000
        }"#,
    )
    .expect("amoy BorConfig JSON should be valid")
}

/// Constructs the full Genesis struct for Polygon mainnet.
///
/// Reference: bor/core/genesis.go `DefaultBorMainnetGenesisBlock()`
///
/// Genesis header values:
///   - Timestamp: 1590824836
///   - GasLimit: 10_000_000
///   - Difficulty: 1
///   - Nonce: 0
///   - Coinbase: 0x0
///   - MixHash: 0x0
pub fn polygon_mainnet_genesis() -> Genesis {
    Genesis {
        config: polygon_mainnet_chain_config(),
        alloc: parse_alloc(POLYGON_MAINNET_ALLOC),
        coinbase: Address::zero(),
        difficulty: U256::from(1),
        extra_data: bytes::Bytes::new(),
        gas_limit: 10_000_000,
        nonce: 0,
        mix_hash: H256::zero(),
        timestamp: 1_590_824_836,
        base_fee_per_gas: None,
        blob_gas_used: None,
        excess_blob_gas: None,
        requests_hash: None,
        block_access_list_hash: None,
        slot_number: None,
    }
}

/// Constructs the full Genesis struct for Amoy testnet.
///
/// Reference: bor/core/genesis.go `DefaultAmoyGenesisBlock()`
///
/// Genesis header values:
///   - Timestamp: 1700225065
///   - GasLimit: 10_000_000
///   - Difficulty: 1
///   - Nonce: 0
///   - Coinbase: 0x0
///   - MixHash: 0x0
pub fn amoy_genesis() -> Genesis {
    Genesis {
        config: amoy_chain_config(),
        alloc: parse_alloc(AMOY_ALLOC),
        coinbase: Address::zero(),
        difficulty: U256::from(1),
        extra_data: bytes::Bytes::new(),
        gas_limit: 10_000_000,
        nonce: 0,
        mix_hash: H256::zero(),
        timestamp: 1_700_225_065,
        base_fee_per_gas: None,
        blob_gas_used: None,
        excess_blob_gas: None,
        requests_hash: None,
        block_access_list_hash: None,
        slot_number: None,
    }
}

/// Constructs the correct genesis Block for a Polygon Genesis.
///
/// Unlike Ethereum PoS, Polygon (Bor) genesis blocks do NOT include post-merge
/// header fields (withdrawals_root, blob_gas_used, excess_blob_gas,
/// parent_beacon_block_root, requests_hash). Our ChainConfig sets
/// shanghai/cancun/prague timestamps to 0 for EVM opcode purposes, but
/// `Genesis::get_block()` would incorrectly add those header fields.
///
/// This function builds the header without those fields, producing a block
/// whose hash matches the real Polygon genesis.
pub fn polygon_genesis_block(genesis: &Genesis) -> Block {
    // base_fee_per_gas: only set if London is active at block 0
    let base_fee_per_gas = if genesis.config.is_london_activated(0) {
        Some(
            genesis
                .base_fee_per_gas
                .unwrap_or(ethrex_common::types::INITIAL_BASE_FEE),
        )
    } else {
        genesis.base_fee_per_gas
    };

    let header = BlockHeader {
        parent_hash: H256::zero(),
        ommers_hash: *DEFAULT_OMMERS_HASH,
        coinbase: genesis.coinbase,
        state_root: genesis.compute_state_root(),
        transactions_root: compute_transactions_root(&[], &NativeCrypto),
        receipts_root: compute_receipts_root(&[], &NativeCrypto),
        logs_bloom: Bloom::zero(),
        difficulty: genesis.difficulty,
        number: 0,
        gas_limit: genesis.gas_limit,
        gas_used: 0,
        timestamp: genesis.timestamp,
        extra_data: genesis.extra_data.clone(),
        prev_randao: genesis.mix_hash,
        nonce: genesis.nonce,
        base_fee_per_gas,
        // Polygon does not use any post-merge header fields
        withdrawals_root: None,
        blob_gas_used: None,
        excess_blob_gas: None,
        parent_beacon_block_root: None,
        requests_hash: None,
        block_access_list_hash: None,
        slot_number: None,
        ..Default::default()
    };

    let body = BlockBody {
        transactions: vec![],
        ommers: vec![],
        withdrawals: None,
    };

    Block::new(header, body)
}

/// ChainConfig for Polygon mainnet.
///
/// Note: In Bor, the EVM fork activations are stored as block numbers
/// (ShanghaiBlock, CancunBlock, PragueBlock) rather than timestamps.
/// Since our ChainConfig uses timestamps for post-merge forks, we set
/// all pre-Istanbul forks to block 0 (always active) and set the
/// post-merge timestamp fields to Some(0) (always active) so that EVM
/// opcode gates work correctly — the Polygon fork schedule is
/// tracked via BorConfig instead.
fn polygon_mainnet_chain_config() -> ChainConfig {
    ChainConfig {
        chain_id: POLYGON_MAINNET_CHAIN_ID,
        homestead_block: Some(0),
        eip150_block: Some(0),
        eip155_block: Some(0),
        eip158_block: Some(0),
        byzantium_block: Some(0),
        constantinople_block: Some(0),
        petersburg_block: Some(0),
        istanbul_block: Some(3_395_000),
        muir_glacier_block: Some(3_395_000),
        berlin_block: Some(14_750_000),
        london_block: Some(23_850_000),
        // Post-Lisovo: all Ethereum EVM forks are always active.
        // Set timestamp-based forks to 0 so chain_config.fork()
        // returns at least Prague for EVM feature checks.
        shanghai_time: Some(0),
        cancun_time: Some(0),
        prague_time: Some(0),
        ..Default::default()
    }
}

/// ChainConfig for Amoy testnet.
fn amoy_chain_config() -> ChainConfig {
    ChainConfig {
        chain_id: AMOY_CHAIN_ID,
        homestead_block: Some(0),
        eip150_block: Some(0),
        eip155_block: Some(0),
        eip158_block: Some(0),
        byzantium_block: Some(0),
        constantinople_block: Some(0),
        petersburg_block: Some(0),
        istanbul_block: Some(0),
        muir_glacier_block: Some(0),
        berlin_block: Some(0),
        london_block: Some(73100),
        // Post-Lisovo: all Ethereum EVM forks always active.
        shanghai_time: Some(0),
        cancun_time: Some(0),
        prague_time: Some(0),
        ..Default::default()
    }
}

/// Returns a reference to the BorConfig for a given chain ID, if it's a known Polygon network.
///
/// The config is parsed from JSON once and cached for subsequent calls.
/// Returns a `&'static` reference — no cloning overhead.
pub fn bor_config_for_chain(chain_id: u64) -> Option<&'static BorConfig> {
    use std::sync::OnceLock;
    static MAINNET: OnceLock<BorConfig> = OnceLock::new();
    static AMOY: OnceLock<BorConfig> = OnceLock::new();

    match chain_id {
        POLYGON_MAINNET_CHAIN_ID => Some(MAINNET.get_or_init(polygon_mainnet_bor_config)),
        AMOY_CHAIN_ID => Some(AMOY.get_or_init(amoy_bor_config)),
        _ => None,
    }
}

/// Returns the Genesis for a given chain ID, if it's a known Polygon network.
pub fn genesis_for_chain(chain_id: u64) -> Option<Genesis> {
    match chain_id {
        POLYGON_MAINNET_CHAIN_ID => Some(polygon_mainnet_genesis()),
        AMOY_CHAIN_ID => Some(amoy_genesis()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_polygon_mainnet_alloc_parses() {
        let alloc = parse_alloc(POLYGON_MAINNET_ALLOC);
        // Polygon mainnet alloc should have the validator contract
        let validator_addr =
            Address::from_str("0x0000000000000000000000000000000000001000").expect("valid address");
        assert!(
            alloc.contains_key(&validator_addr),
            "alloc should contain validator contract"
        );
    }

    #[test]
    fn test_amoy_alloc_parses() {
        let alloc = parse_alloc(AMOY_ALLOC);
        let validator_addr =
            Address::from_str("0x0000000000000000000000000000000000001000").expect("valid address");
        assert!(
            alloc.contains_key(&validator_addr),
            "alloc should contain validator contract"
        );
    }

    #[test]
    fn test_polygon_mainnet_bor_config() {
        let config = polygon_mainnet_bor_config();
        assert_eq!(config.get_sprint_size(0), 64);
        assert_eq!(config.get_sprint_size(38_189_056), 16);
        assert!(config.is_lisovo_active(83_756_500));
    }

    #[test]
    fn test_amoy_bor_config() {
        let config = amoy_bor_config();
        assert_eq!(config.get_sprint_size(0), 16);
        assert!(config.is_lisovo_active(33_634_700));
    }

    #[test]
    fn test_polygon_mainnet_genesis_builds() {
        let genesis = polygon_mainnet_genesis();
        assert_eq!(genesis.config.chain_id, 137);
        assert_eq!(genesis.timestamp, 1_590_824_836);
        assert_eq!(genesis.gas_limit, 10_000_000);
        assert_eq!(genesis.difficulty, U256::from(1));
    }

    #[test]
    fn test_amoy_genesis_builds() {
        let genesis = amoy_genesis();
        assert_eq!(genesis.config.chain_id, 80002);
        assert_eq!(genesis.timestamp, 1_700_225_065);
    }

    #[test]
    fn test_polygon_mainnet_genesis_difficulty_one() {
        let genesis = polygon_mainnet_genesis();
        assert_eq!(genesis.difficulty, U256::from(1));
    }

    #[test]
    fn test_amoy_genesis_difficulty_one() {
        let genesis = amoy_genesis();
        assert_eq!(genesis.difficulty, U256::from(1));
    }

    #[test]
    fn test_polygon_mainnet_system_contracts_in_alloc() {
        let genesis = polygon_mainnet_genesis();
        let validator =
            Address::from_str("0x0000000000000000000000000000000000001000").expect("valid address");
        let state_receiver =
            Address::from_str("0x0000000000000000000000000000000000001001").expect("valid address");
        assert!(
            genesis.alloc.contains_key(&validator),
            "mainnet alloc must contain validator contract (0x1000)"
        );
        assert!(
            genesis.alloc.contains_key(&state_receiver),
            "mainnet alloc must contain state receiver contract (0x1001)"
        );
    }

    #[test]
    fn test_amoy_system_contracts_in_alloc() {
        let genesis = amoy_genesis();
        let validator =
            Address::from_str("0x0000000000000000000000000000000000001000").expect("valid address");
        let state_receiver =
            Address::from_str("0x0000000000000000000000000000000000001001").expect("valid address");
        assert!(
            genesis.alloc.contains_key(&validator),
            "amoy alloc must contain validator contract (0x1000)"
        );
        assert!(
            genesis.alloc.contains_key(&state_receiver),
            "amoy alloc must contain state receiver contract (0x1001)"
        );
    }

    #[test]
    fn test_polygon_mainnet_evm_forks_always_active() {
        let genesis = polygon_mainnet_genesis();
        assert_eq!(genesis.config.shanghai_time, Some(0));
        assert_eq!(genesis.config.cancun_time, Some(0));
        assert_eq!(genesis.config.prague_time, Some(0));
    }

    #[test]
    fn test_amoy_evm_forks_always_active() {
        let genesis = amoy_genesis();
        assert_eq!(genesis.config.shanghai_time, Some(0));
        assert_eq!(genesis.config.cancun_time, Some(0));
        assert_eq!(genesis.config.prague_time, Some(0));
    }

    #[test]
    fn test_bor_config_for_chain_mainnet() {
        let config = bor_config_for_chain(137);
        assert!(config.is_some());
        assert_eq!(config.unwrap().get_sprint_size(0), 64);
    }

    #[test]
    fn test_bor_config_for_chain_amoy() {
        let config = bor_config_for_chain(80002);
        assert!(config.is_some());
        assert_eq!(config.unwrap().get_sprint_size(0), 16);
    }

    #[test]
    fn test_bor_config_for_chain_unknown() {
        assert!(bor_config_for_chain(1).is_none());
        assert!(bor_config_for_chain(0).is_none());
    }

    #[test]
    fn test_genesis_for_chain_mainnet() {
        let genesis = genesis_for_chain(137);
        assert!(genesis.is_some());
        assert_eq!(genesis.unwrap().config.chain_id, 137);
    }

    #[test]
    fn test_genesis_for_chain_amoy() {
        let genesis = genesis_for_chain(80002);
        assert!(genesis.is_some());
        assert_eq!(genesis.unwrap().config.chain_id, 80002);
    }

    #[test]
    fn test_genesis_for_chain_unknown() {
        assert!(genesis_for_chain(1).is_none());
    }

    // ====================================================================
    // Cross-validation tests against real Polygon Bor genesis data
    // ====================================================================

    /// Verify that our Polygon mainnet genesis matches the exact field values from Bor source.
    ///
    /// Reference: bor/core/genesis.go `DefaultBorMainnetGenesisBlock()`
    /// Known genesis hash from Bor: 0xa9c28ce2141b56c474f1dc504bee9b01eb1bd7d1a507580d5519d4437a97de1b
    ///
    /// Note: Full genesis hash verification requires computing the state root from the
    /// alloc (building a Merkle Patricia Trie from all genesis accounts). This test
    /// verifies all header-level fields match Bor's source exactly.
    #[test]
    fn crosscheck_polygon_mainnet_genesis_fields_match_bor() {
        let genesis = polygon_mainnet_genesis();

        // Chain config
        assert_eq!(genesis.config.chain_id, 137, "chain ID must be 137");

        // Header fields from Bor source
        assert_eq!(genesis.timestamp, 1_590_824_836, "timestamp: 0x5ED20F84");
        assert_eq!(genesis.gas_limit, 10_000_000, "gasLimit: 0x989680");
        assert_eq!(genesis.difficulty, U256::from(1), "difficulty must be 1");
        assert_eq!(genesis.nonce, 0, "nonce must be 0");
        assert_eq!(genesis.coinbase, Address::zero(), "coinbase must be zero");
        assert_eq!(genesis.mix_hash, H256::zero(), "mixHash must be zero");
        assert!(genesis.extra_data.is_empty(), "extraData must be empty");

        // Post-merge fields must be absent
        assert!(
            genesis.base_fee_per_gas.is_none(),
            "no baseFeePerGas (pre-London genesis)"
        );
        assert!(genesis.blob_gas_used.is_none(), "no blob_gas_used");
        assert!(genesis.excess_blob_gas.is_none(), "no excess_blob_gas");
        assert!(genesis.requests_hash.is_none(), "no requests_hash");

        // Verify the expected genesis hash constant is correctly declared
        assert_eq!(
            POLYGON_MAINNET_GENESIS_HASH,
            "a9c28ce2141b56c474f1dc504bee9b01eb1bd7d1a507580d5519d4437a97de1b",
            "genesis hash constant must match Bor source"
        );

        // EVM fork schedule: all post-Istanbul forks must be set
        assert_eq!(genesis.config.istanbul_block, Some(3_395_000));
        assert_eq!(genesis.config.berlin_block, Some(14_750_000));
        assert_eq!(genesis.config.london_block, Some(23_850_000));
        // Shanghai/Cancun/Prague set to 0 (always active for EVM features)
        assert_eq!(genesis.config.shanghai_time, Some(0));
        assert_eq!(genesis.config.cancun_time, Some(0));
        assert_eq!(genesis.config.prague_time, Some(0));
    }

    /// Verify that our Amoy testnet genesis matches the exact field values from Bor source.
    ///
    /// Reference: bor/core/genesis.go `DefaultAmoyGenesisBlock()`
    /// Known genesis hash from Bor: 0x7202b2b53c5a0836e773e319d18922cc756dd67432f9a1f65352b61f4406c697
    #[test]
    fn crosscheck_amoy_genesis_fields_match_bor() {
        let genesis = amoy_genesis();

        // Chain config
        assert_eq!(genesis.config.chain_id, 80002, "chain ID must be 80002");

        // Header fields from Bor source
        assert_eq!(genesis.timestamp, 1_700_225_065, "timestamp: 0x65576029");
        assert_eq!(genesis.gas_limit, 10_000_000, "gasLimit: 0x989680");
        assert_eq!(genesis.difficulty, U256::from(1), "difficulty must be 1");
        assert_eq!(genesis.nonce, 0, "nonce must be 0");
        assert_eq!(genesis.coinbase, Address::zero(), "coinbase must be zero");
        assert_eq!(genesis.mix_hash, H256::zero(), "mixHash must be zero");
        assert!(genesis.extra_data.is_empty(), "extraData must be empty");

        // Post-merge fields must be absent
        assert!(
            genesis.base_fee_per_gas.is_none(),
            "no baseFeePerGas (pre-London genesis)"
        );

        // Verify the expected genesis hash constant
        assert_eq!(
            AMOY_GENESIS_HASH, "7202b2b53c5a0836e773e319d18922cc756dd67432f9a1f65352b61f4406c697",
            "genesis hash constant must match Bor source"
        );

        // All pre-Istanbul forks at block 0 (always active)
        assert_eq!(genesis.config.homestead_block, Some(0));
        assert_eq!(genesis.config.istanbul_block, Some(0));
        assert_eq!(genesis.config.berlin_block, Some(0));
        assert_eq!(genesis.config.london_block, Some(73100));
    }

    #[test]
    fn test_polygon_mainnet_genesis_coinbase_zero() {
        let genesis = polygon_mainnet_genesis();
        assert_eq!(genesis.coinbase, Address::zero());
    }

    #[test]
    fn test_polygon_mainnet_genesis_mix_hash_zero() {
        let genesis = polygon_mainnet_genesis();
        assert_eq!(genesis.mix_hash, H256::zero());
    }

    #[test]
    fn test_polygon_mainnet_genesis_no_blob_fields() {
        let genesis = polygon_mainnet_genesis();
        assert!(genesis.blob_gas_used.is_none());
        assert!(genesis.excess_blob_gas.is_none());
        assert!(genesis.requests_hash.is_none());
    }

    /// Verify that our Polygon mainnet genesis block hash matches the known value from Bor.
    ///
    /// Known hash: 0xa9c28ce2141b56c474f1dc504bee9b01eb1bd7d1a507580d5519d4437a97de1b
    ///
    /// Uses `polygon_genesis_block()` which builds the header without post-merge
    /// fields (withdrawals_root, blob_gas_used, etc.) that the generic
    /// `Genesis::get_block()` would incorrectly add.
    #[test]
    fn verify_polygon_mainnet_genesis_hash() {
        let genesis = polygon_mainnet_genesis();
        let block = polygon_genesis_block(&genesis);
        let computed_hash = block.hash();
        let expected_hash =
            H256::from_str(&format!("0x{POLYGON_MAINNET_GENESIS_HASH}")).expect("valid hash");
        assert_eq!(
            computed_hash, expected_hash,
            "Polygon mainnet genesis hash mismatch: computed {computed_hash:?}, expected {expected_hash:?}"
        );
    }

    /// Verify that our Amoy testnet genesis block hash matches the known value from Bor.
    ///
    /// Known hash: 0x7202b2b53c5a0836e773e319d18922cc756dd67432f9a1f65352b61f4406c697
    #[test]
    fn verify_amoy_genesis_hash() {
        let genesis = amoy_genesis();
        let block = polygon_genesis_block(&genesis);
        let computed_hash = block.hash();
        let expected_hash = H256::from_str(&format!("0x{AMOY_GENESIS_HASH}")).expect("valid hash");
        assert_eq!(
            computed_hash, expected_hash,
            "Amoy genesis hash mismatch: computed {computed_hash:?}, expected {expected_hash:?}"
        );
    }
}
