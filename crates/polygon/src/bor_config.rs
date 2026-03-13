use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;

use ethereum_types::Address;
use ethrex_common::types::{BlockNumber, Fork, GenesisAccount};
use serde::{Deserialize, Deserializer, Serialize};

/// BorConfig is the consensus engine config for Polygon's Bor consensus.
///
/// Maps keyed by block number (as string in JSON, parsed to u64) store
/// parameters that change at specific fork blocks.
///
/// Reference: bor/params/config.go `type BorConfig struct`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BorConfig {
    /// Block time in seconds, per fork block.
    #[serde(deserialize_with = "deserialize_string_key_map")]
    pub period: BTreeMap<BlockNumber, u64>,

    /// Producer delay in seconds, per fork block.
    #[serde(deserialize_with = "deserialize_string_key_map")]
    pub producer_delay: BTreeMap<BlockNumber, u64>,

    /// Sprint length (number of blocks in a sprint), per fork block.
    #[serde(deserialize_with = "deserialize_string_key_map")]
    pub sprint: BTreeMap<BlockNumber, u64>,

    /// Backup multiplier for wiggle time, per fork block.
    #[serde(deserialize_with = "deserialize_string_key_map")]
    pub backup_multiplier: BTreeMap<BlockNumber, u64>,

    /// Validator set contract address.
    #[serde(deserialize_with = "deserialize_address_string")]
    pub validator_contract: Address,

    /// State receiver contract address.
    #[serde(deserialize_with = "deserialize_address_string")]
    pub state_receiver_contract: Address,

    /// Override state sync records at specific blocks.
    #[serde(default, deserialize_with = "deserialize_string_key_map_to_i64")]
    pub override_state_sync_records: BTreeMap<BlockNumber, i64>,

    /// Override state sync records within block ranges.
    #[serde(default)]
    pub override_state_sync_records_in_range: Vec<BlockRangeOverride>,

    /// Override validator set within block ranges.
    #[serde(default)]
    pub override_validator_set_in_range: Vec<BlockRangeOverrideValidatorSet>,

    /// Block alloc: maps block_number -> address -> GenesisAccount.
    /// This is typed loosely to match Bor's `map[string]interface{}` — we parse
    /// it into a nested structure.
    #[serde(default, deserialize_with = "deserialize_block_alloc")]
    pub block_alloc: BTreeMap<BlockNumber, HashMap<Address, GenesisAccount>>,

    /// Governance/burnt contract address, per fork block.
    #[serde(default, deserialize_with = "deserialize_string_key_address_map")]
    pub burnt_contract: BTreeMap<BlockNumber, Address>,

    /// Coinbase address override, per fork block.
    #[serde(default, deserialize_with = "deserialize_string_key_address_map")]
    pub coinbase: BTreeMap<BlockNumber, Address>,

    /// Blocks to skip validator byte check.
    #[serde(default)]
    pub skip_validator_byte_check: Vec<u64>,

    /// State sync confirmation delay in seconds, per fork block.
    #[serde(default, deserialize_with = "deserialize_string_key_map")]
    pub state_sync_confirmation_delay: BTreeMap<BlockNumber, u64>,

    // ---- EVM-level fork blocks (stored as block numbers in Bor) ----
    // Bor activates Shanghai/Cancun/Prague at block numbers, not timestamps.
    // These are needed for fork ID computation (EIP-2124).
    /// London (EIP-1559) activation block.
    #[serde(default)]
    pub london_block: Option<u64>,
    /// Shanghai activation block.
    #[serde(default)]
    pub shanghai_block: Option<u64>,
    /// Cancun activation block.
    #[serde(default)]
    pub cancun_block: Option<u64>,
    /// Prague activation block.
    #[serde(default)]
    pub prague_block: Option<u64>,

    // ---- Polygon fork blocks ----
    /// Jaipur fork block.
    pub jaipur_block: Option<u64>,
    /// Delhi fork block.
    pub delhi_block: Option<u64>,
    /// Indore fork block.
    pub indore_block: Option<u64>,
    /// Ahmedabad fork block.
    pub ahmedabad_block: Option<u64>,
    /// Bhilai fork block.
    pub bhilai_block: Option<u64>,
    /// Rio fork block.
    pub rio_block: Option<u64>,
    /// Madhugiri fork block.
    pub madhugiri_block: Option<u64>,
    /// MadhugiriPro fork block.
    pub madhugiri_pro_block: Option<u64>,
    /// Dandeli fork block.
    pub dandeli_block: Option<u64>,
    /// Lisovo fork block.
    pub lisovo_block: Option<u64>,
    /// LisovoPro fork block.
    pub lisovo_pro_block: Option<u64>,
    /// Giugliano fork block.
    pub giugliano_block: Option<u64>,
}

/// Override state sync records within a block range.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockRangeOverride {
    pub start_block: u64,
    pub end_block: u64,
    pub value: i64,
}

/// Override validator set within a block range.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockRangeOverrideValidatorSet {
    pub start_block: u64,
    pub end_block: u64,
    pub validators: Vec<Address>,
}

// ---- Block-number-indexed parameter lookup helpers (Task 11) ----

impl BorConfig {
    /// Returns the sprint size active at the given block number.
    ///
    /// Looks up the BTreeMap and returns the value for the highest key <= block_number.
    pub fn get_sprint_size(&self, block_number: BlockNumber) -> u64 {
        lookup_btree(&self.sprint, block_number)
    }

    /// Returns the block period (seconds between blocks) active at the given block number.
    pub fn get_period(&self, block_number: BlockNumber) -> u64 {
        lookup_btree(&self.period, block_number)
    }

    /// Returns the producer delay active at the given block number.
    pub fn get_producer_delay(&self, block_number: BlockNumber) -> u64 {
        lookup_btree(&self.producer_delay, block_number)
    }

    /// Returns the backup multiplier active at the given block number.
    pub fn get_backup_multiplier(&self, block_number: BlockNumber) -> u64 {
        lookup_btree(&self.backup_multiplier, block_number)
    }

    /// Returns the state sync confirmation delay active at the given block number.
    /// Returns 0 if no delay is configured for this block (pre-Indore on mainnet).
    pub fn get_state_sync_delay(&self, block_number: BlockNumber) -> u64 {
        lookup_btree_opt(&self.state_sync_confirmation_delay, block_number).unwrap_or(0)
    }

    /// Returns the burnt contract address active at the given block number, if any.
    pub fn get_burnt_contract(&self, block_number: BlockNumber) -> Option<Address> {
        lookup_btree_opt(&self.burnt_contract, block_number)
    }

    /// Returns the coinbase address active at the given block number.
    /// Defaults to Address::zero() if no coinbase is configured.
    pub fn get_coinbase(&self, block_number: BlockNumber) -> Address {
        lookup_btree_opt(&self.coinbase, block_number).unwrap_or_default()
    }

    // ---- Fork activation checks ----

    pub fn is_jaipur_active(&self, block_number: BlockNumber) -> bool {
        is_fork_active(self.jaipur_block, block_number)
    }

    pub fn is_delhi_active(&self, block_number: BlockNumber) -> bool {
        is_fork_active(self.delhi_block, block_number)
    }

    pub fn is_indore_active(&self, block_number: BlockNumber) -> bool {
        is_fork_active(self.indore_block, block_number)
    }

    pub fn is_ahmedabad_active(&self, block_number: BlockNumber) -> bool {
        is_fork_active(self.ahmedabad_block, block_number)
    }

    pub fn is_bhilai_active(&self, block_number: BlockNumber) -> bool {
        is_fork_active(self.bhilai_block, block_number)
    }

    pub fn is_rio_active(&self, block_number: BlockNumber) -> bool {
        is_fork_active(self.rio_block, block_number)
    }

    pub fn is_madhugiri_active(&self, block_number: BlockNumber) -> bool {
        is_fork_active(self.madhugiri_block, block_number)
    }

    pub fn is_madhugiri_pro_active(&self, block_number: BlockNumber) -> bool {
        is_fork_active(self.madhugiri_pro_block, block_number)
    }

    pub fn is_dandeli_active(&self, block_number: BlockNumber) -> bool {
        is_fork_active(self.dandeli_block, block_number)
    }

    pub fn is_lisovo_active(&self, block_number: BlockNumber) -> bool {
        is_fork_active(self.lisovo_block, block_number)
    }

    pub fn is_lisovo_pro_active(&self, block_number: BlockNumber) -> bool {
        is_fork_active(self.lisovo_pro_block, block_number)
    }

    pub fn is_giugliano_active(&self, block_number: BlockNumber) -> bool {
        is_fork_active(self.giugliano_block, block_number)
    }

    /// Returns true if block_number is the start of a sprint.
    pub fn is_sprint_start(&self, block_number: BlockNumber) -> bool {
        if block_number == 0 {
            return true;
        }
        let sprint = self.get_sprint_size(block_number);
        if sprint == 0 {
            return false;
        }
        block_number.is_multiple_of(sprint)
    }

    /// Returns true if block_number is the last block of a sprint
    /// (i.e., the block just before a sprint start).
    pub fn is_sprint_end(&self, block_number: BlockNumber) -> bool {
        match block_number.checked_add(1) {
            Some(next) => self.is_sprint_start(next),
            None => false, // u64::MAX cannot be a sprint end
        }
    }

    /// Returns the sprint-end block number for the sprint containing `block_number`.
    ///
    /// This is the last block of the current sprint.
    pub fn sprint_end_block(&self, block_number: BlockNumber) -> BlockNumber {
        let sprint = self.get_sprint_size(block_number);
        if sprint == 0 {
            return block_number;
        }
        // Next sprint start, minus 1.
        let next_sprint_start = (block_number / sprint + 1) * sprint;
        next_sprint_start - 1
    }

    /// Returns an approximate span ID for the given block number.
    ///
    /// Bor spans are nominally 6400 blocks (but actual boundaries come from
    /// Heimdall). This provides a rough estimate without querying Heimdall.
    pub fn span_id_at(&self, block_number: BlockNumber) -> u64 {
        // Span 0 covers blocks 0..=255 (256 blocks).
        // Each subsequent span covers 6400 blocks.
        if block_number <= 255 {
            return 0;
        }
        // After span 0 ends at block 255, span 1 starts at block 256.
        (block_number - 256) / 6400 + 1
    }

    /// Returns true if a span commit system call is needed at this block.
    ///
    /// In Bor, a new span is committed at the first block of the last sprint in
    /// the current span: `currentSpan.EndBlock - sprintLength + 1`.
    /// We trigger at sprint-start blocks where the sprint's end would cross
    /// into a new span.
    pub fn need_to_commit_span(&self, block_number: BlockNumber) -> bool {
        if !self.is_sprint_start(block_number) || block_number == 0 {
            return false;
        }
        let sprint = self.get_sprint_size(block_number);
        let current_span = self.span_id_at(block_number);
        // Check if this sprint's end would cross into a new span.
        let sprint_end = block_number + sprint - 1;
        let next_span = self.span_id_at(sprint_end + 1);
        next_span != current_span
    }

    /// Returns the highest active Polygon fork at the given block number.
    ///
    /// Polygon forks are block-number-activated (not timestamp). The returned
    /// Fork variant is numerically > all Ethereum EVM forks, so comparisons
    /// like `fork >= Fork::Prague` remain true for any Polygon fork.
    pub fn get_polygon_fork(&self, block_number: BlockNumber) -> Fork {
        if self.is_giugliano_active(block_number) {
            Fork::Giugliano
        } else if self.is_lisovo_pro_active(block_number) {
            Fork::LisovoPro
        } else if self.is_lisovo_active(block_number) {
            Fork::Lisovo
        } else if self.is_dandeli_active(block_number) {
            Fork::Dandeli
        } else if self.is_madhugiri_pro_active(block_number) {
            Fork::MadhugiriPro
        } else if self.is_madhugiri_active(block_number) {
            Fork::Madhugiri
        } else if self.is_rio_active(block_number) {
            Fork::Rio
        } else if self.is_bhilai_active(block_number) {
            Fork::Bhilai
        } else if self.is_ahmedabad_active(block_number) {
            Fork::Ahmedabad
        } else if self.is_indore_active(block_number) {
            Fork::Indore
        } else if self.is_delhi_active(block_number) {
            Fork::Delhi
        } else if self.is_jaipur_active(block_number) {
            Fork::Jaipur
        } else {
            // Pre-Jaipur: return Prague since all EVM forks are active
            Fork::Prague
        }
    }
}

/// Generic lookup for BTreeMap<BlockNumber, T>: returns the value for the
/// highest key that is <= `block_number`.
fn lookup_btree<T: Copy>(map: &BTreeMap<BlockNumber, T>, block_number: BlockNumber) -> T {
    map.range(..=block_number)
        .next_back()
        .map(|(_, v)| *v)
        .expect("BorConfig parameter map should have at least one entry (key 0)")
}

/// Same as lookup_btree but returns None if the map is empty.
fn lookup_btree_opt<T: Copy>(
    map: &BTreeMap<BlockNumber, T>,
    block_number: BlockNumber,
) -> Option<T> {
    map.range(..=block_number).next_back().map(|(_, v)| *v)
}

/// Returns true if a fork is active at the given block number.
fn is_fork_active(fork_block: Option<u64>, block_number: BlockNumber) -> bool {
    fork_block.is_some_and(|fb| fb <= block_number)
}

// ---- Custom deserialization helpers ----
// Bor's JSON uses string keys for block numbers in maps (e.g., "0": 2, "38189056": 4).

/// Deserializes a map with string keys into BTreeMap<u64, u64>.
fn deserialize_string_key_map<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<BlockNumber, u64>, D::Error>
where
    D: Deserializer<'de>,
{
    let map: HashMap<String, u64> = HashMap::deserialize(deserializer)?;
    let mut result = BTreeMap::new();
    for (k, v) in map {
        let key: u64 = k.parse().map_err(serde::de::Error::custom)?;
        result.insert(key, v);
    }
    Ok(result)
}

/// Deserializes a map with string keys into BTreeMap<u64, i64> (for override_state_sync_records).
fn deserialize_string_key_map_to_i64<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<BlockNumber, i64>, D::Error>
where
    D: Deserializer<'de>,
{
    let map: HashMap<String, i64> = HashMap::deserialize(deserializer)?;
    let mut result = BTreeMap::new();
    for (k, v) in map {
        let key: u64 = k.parse().map_err(serde::de::Error::custom)?;
        result.insert(key, v);
    }
    Ok(result)
}

/// Deserializes a map with string keys and address string values.
fn deserialize_string_key_address_map<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<BlockNumber, Address>, D::Error>
where
    D: Deserializer<'de>,
{
    let map: HashMap<String, String> = HashMap::deserialize(deserializer)?;
    let mut result = BTreeMap::new();
    for (k, v) in map {
        let key: u64 = k.parse().map_err(serde::de::Error::custom)?;
        let addr = Address::from_str(&v).map_err(serde::de::Error::custom)?;
        result.insert(key, addr);
    }
    Ok(result)
}

/// Deserializes an address from a hex string (with or without 0x prefix).
fn deserialize_address_string<'de, D>(deserializer: D) -> Result<Address, D::Error>
where
    D: Deserializer<'de>,
{
    let s: String = String::deserialize(deserializer)?;
    Address::from_str(&s).map_err(serde::de::Error::custom)
}

/// Deserializes block_alloc: map[string]interface{} in Bor's format.
/// The outer keys are block numbers (as strings). The inner map is
/// address -> GenesisAccount (with balance, code, storage, nonce).
///
/// Addresses in the alloc may or may not have a 0x prefix.
fn deserialize_block_alloc<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<BlockNumber, HashMap<Address, GenesisAccount>>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw: HashMap<String, HashMap<String, GenesisAccount>> = HashMap::deserialize(deserializer)?;
    let mut result = BTreeMap::new();
    for (block_str, accounts) in raw {
        let block_num: u64 = block_str.parse().map_err(serde::de::Error::custom)?;
        let mut account_map = HashMap::new();
        for (addr_str, account) in accounts {
            // Handle addresses with or without 0x prefix
            let addr_with_prefix = if addr_str.starts_with("0x") || addr_str.starts_with("0X") {
                addr_str
            } else {
                format!("0x{addr_str}")
            };
            let addr = Address::from_str(&addr_with_prefix).map_err(serde::de::Error::custom)?;
            account_map.insert(addr, account);
        }
        result.insert(block_num, account_map);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lookup_btree() {
        let mut map = BTreeMap::new();
        map.insert(0, 64u64);
        map.insert(100, 16u64);

        assert_eq!(lookup_btree(&map, 0), 64);
        assert_eq!(lookup_btree(&map, 50), 64);
        assert_eq!(lookup_btree(&map, 99), 64);
        assert_eq!(lookup_btree(&map, 100), 16);
        assert_eq!(lookup_btree(&map, 200), 16);
    }

    #[test]
    fn test_is_fork_active() {
        assert!(!is_fork_active(None, 100));
        assert!(is_fork_active(Some(0), 0));
        assert!(is_fork_active(Some(0), 100));
        assert!(!is_fork_active(Some(100), 50));
        assert!(is_fork_active(Some(100), 100));
        assert!(is_fork_active(Some(100), 200));
    }

    #[test]
    fn test_is_sprint_start() {
        let config = BorConfig {
            period: BTreeMap::from([(0, 2)]),
            producer_delay: BTreeMap::from([(0, 4)]),
            sprint: BTreeMap::from([(0, 16)]),
            backup_multiplier: BTreeMap::from([(0, 2)]),
            validator_contract: Address::from_str("0x0000000000000000000000000000000000001000")
                .expect("valid address"),
            state_receiver_contract: Address::from_str(
                "0x0000000000000000000000000000000000001001",
            )
            .expect("valid address"),
            override_state_sync_records: BTreeMap::new(),
            override_state_sync_records_in_range: vec![],
            override_validator_set_in_range: vec![],
            block_alloc: BTreeMap::new(),
            burnt_contract: BTreeMap::new(),
            coinbase: BTreeMap::new(),
            skip_validator_byte_check: vec![],
            state_sync_confirmation_delay: BTreeMap::new(),
            jaipur_block: Some(0),
            delhi_block: Some(0),
            indore_block: Some(0),
            ahmedabad_block: None,
            bhilai_block: None,
            rio_block: None,
            madhugiri_block: None,
            madhugiri_pro_block: None,
            dandeli_block: None,
            lisovo_block: None,
            lisovo_pro_block: None,
            giugliano_block: None,
            london_block: None,
            shanghai_block: None,
            cancun_block: None,
            prague_block: None,
        };

        assert!(config.is_sprint_start(0));
        assert!(!config.is_sprint_start(1));
        assert!(config.is_sprint_start(16));
        assert!(config.is_sprint_start(32));
        assert!(!config.is_sprint_start(15));
    }

    #[test]
    fn test_deserialize_bor_config_json() {
        let json = r#"{
            "period": {"0": 2, "80084800": 1},
            "producerDelay": {"0": 6, "38189056": 4},
            "sprint": {"0": 64, "38189056": 16},
            "backupMultiplier": {"0": 2},
            "validatorContract": "0x0000000000000000000000000000000000001000",
            "stateReceiverContract": "0x0000000000000000000000000000000000001001",
            "burntContract": {
                "23850000": "0x70bca57f4579f58670ab2d18ef16e02c17553c38"
            },
            "coinbase": {
                "0": "0x0000000000000000000000000000000000000000",
                "77414656": "0x7Ee41D8A25641000661B1EF5E6AE8A00400466B0"
            },
            "stateSyncConfirmationDelay": {"44934656": 128},
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
        }"#;

        let config: BorConfig = serde_json::from_str(json).expect("Failed to parse BorConfig");

        assert_eq!(config.get_period(0), 2);
        assert_eq!(config.get_period(80084799), 2);
        assert_eq!(config.get_period(80084800), 1);

        assert_eq!(config.get_sprint_size(0), 64);
        assert_eq!(config.get_sprint_size(38189056), 16);

        assert!(config.is_jaipur_active(23850000));
        assert!(!config.is_jaipur_active(23849999));
        assert!(config.is_lisovo_active(83756500));
        assert!(!config.is_lisovo_active(83756499));

        let burnt = config.get_burnt_contract(23850000);
        assert!(burnt.is_some());
        assert_eq!(
            burnt.expect("burnt contract should be set"),
            Address::from_str("0x70bca57f4579f58670ab2d18ef16e02c17553c38").expect("valid address")
        );

        let coinbase = config.get_coinbase(77414656);
        assert_eq!(
            coinbase,
            Address::from_str("0x7Ee41D8A25641000661B1EF5E6AE8A00400466B0").expect("valid address")
        );
        assert_eq!(config.get_coinbase(0), Address::zero());
    }

    /// Helper to build a mainnet-like BorConfig for get_polygon_fork tests.
    fn mainnet_like_config() -> BorConfig {
        serde_json::from_str(
            r#"{
                "period": {"0": 2},
                "producerDelay": {"0": 6},
                "sprint": {"0": 64, "38189056": 16},
                "backupMultiplier": {"0": 2},
                "validatorContract": "0x0000000000000000000000000000000000001000",
                "stateReceiverContract": "0x0000000000000000000000000000000000001001",
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
                "lisovoProBlock": 83756500,
                "giuglianoBlock": 90000000
            }"#,
        )
        .expect("valid config")
    }

    #[test]
    fn test_get_polygon_fork_pre_jaipur() {
        let config = mainnet_like_config();
        // Before Jaipur, should return Prague (all EVM forks active)
        assert_eq!(config.get_polygon_fork(0), Fork::Prague);
        assert_eq!(config.get_polygon_fork(23_849_999), Fork::Prague);
    }

    #[test]
    fn test_get_polygon_fork_at_jaipur() {
        let config = mainnet_like_config();
        assert_eq!(config.get_polygon_fork(23_850_000), Fork::Jaipur);
    }

    #[test]
    fn test_get_polygon_fork_at_delhi() {
        let config = mainnet_like_config();
        assert_eq!(config.get_polygon_fork(38_189_056), Fork::Delhi);
        // One before Delhi should be Jaipur
        assert_eq!(config.get_polygon_fork(38_189_055), Fork::Jaipur);
    }

    #[test]
    fn test_get_polygon_fork_at_giugliano() {
        let config = mainnet_like_config();
        assert_eq!(config.get_polygon_fork(90_000_000), Fork::Giugliano);
        assert_eq!(config.get_polygon_fork(100_000_000), Fork::Giugliano);
    }

    #[test]
    fn test_get_polygon_fork_all_transitions() {
        let config = mainnet_like_config();
        assert_eq!(config.get_polygon_fork(44_934_656), Fork::Indore);
        assert_eq!(config.get_polygon_fork(62_278_656), Fork::Ahmedabad);
        assert_eq!(config.get_polygon_fork(73_440_256), Fork::Bhilai);
        assert_eq!(config.get_polygon_fork(77_414_656), Fork::Rio);
        // Madhugiri and MadhugiriPro activate at same block; MadhugiriPro wins
        assert_eq!(config.get_polygon_fork(80_084_800), Fork::MadhugiriPro);
        assert_eq!(config.get_polygon_fork(81_424_000), Fork::Dandeli);
        // Lisovo and LisovoPro activate at same block; LisovoPro wins
        assert_eq!(config.get_polygon_fork(83_756_500), Fork::LisovoPro);
    }

    #[test]
    fn test_polygon_forks_greater_than_prague() {
        // All Polygon forks should be > Prague for EVM feature comparisons
        assert!(Fork::Jaipur > Fork::Prague);
        assert!(Fork::Delhi > Fork::Prague);
        assert!(Fork::Giugliano > Fork::Prague);
    }

    #[test]
    fn test_fork_activation_boundary_exact() {
        let config = mainnet_like_config();
        // One before each fork
        assert!(!config.is_delhi_active(38_189_055));
        assert!(config.is_delhi_active(38_189_056));
        assert!(config.is_delhi_active(38_189_057));

        assert!(!config.is_ahmedabad_active(62_278_655));
        assert!(config.is_ahmedabad_active(62_278_656));

        assert!(!config.is_giugliano_active(89_999_999));
        assert!(config.is_giugliano_active(90_000_000));
    }

    #[test]
    fn test_fork_activation_at_block_zero() {
        let config = mainnet_like_config();
        // Jaipur is at 23850000, not 0 — should not be active at 0
        assert!(!config.is_jaipur_active(0));
    }

    #[test]
    fn test_fork_activation_very_large_block() {
        let config = mainnet_like_config();
        assert!(config.is_giugliano_active(u64::MAX));
    }

    #[test]
    fn test_sprint_size_changes_at_delhi() {
        let config = mainnet_like_config();
        // Sprint changes from 64 to 16 at Delhi block (38189056)
        assert_eq!(config.get_sprint_size(38_189_055), 64);
        assert_eq!(config.get_sprint_size(38_189_056), 16);
        assert_eq!(config.get_sprint_size(38_189_057), 16);
    }

    #[test]
    fn test_lookup_btree_opt_empty_map() {
        let map: BTreeMap<BlockNumber, Address> = BTreeMap::new();
        assert!(lookup_btree_opt(&map, 100).is_none());
    }

    #[test]
    fn test_get_burnt_contract_before_any_entry() {
        let config = mainnet_like_config();
        // No burnt contract configured before any block
        assert!(config.get_burnt_contract(0).is_none());
    }

    // ---- Sprint boundary tests ----

    #[test]
    fn test_is_sprint_end() {
        let config = mainnet_like_config();
        // Sprint size is 64 pre-Delhi, 16 post-Delhi (38189056)
        // Pre-Delhi: sprint ends at 63, 127, 191, ...
        assert!(config.is_sprint_end(63));
        assert!(!config.is_sprint_end(64));
        assert!(config.is_sprint_end(127));

        // Post-Delhi (sprint=16): ends at ..., 38189071, ...
        assert!(config.is_sprint_end(38_189_071));
        assert!(!config.is_sprint_end(38_189_072));
    }

    #[test]
    fn test_sprint_end_block() {
        let config = mainnet_like_config();
        // Pre-Delhi: sprint=64. Block 0 is in sprint [0,63].
        assert_eq!(config.sprint_end_block(0), 63);
        assert_eq!(config.sprint_end_block(32), 63);
        assert_eq!(config.sprint_end_block(63), 63);
        assert_eq!(config.sprint_end_block(64), 127);

        // Post-Delhi: sprint=16.
        assert_eq!(config.sprint_end_block(38_189_056), 38_189_071);
        assert_eq!(config.sprint_end_block(38_189_060), 38_189_071);
    }

    #[test]
    fn test_span_id_at() {
        let config = mainnet_like_config();
        // Span 0: blocks 0..=255
        assert_eq!(config.span_id_at(0), 0);
        assert_eq!(config.span_id_at(255), 0);
        // Span 1: blocks 256..=6655
        assert_eq!(config.span_id_at(256), 1);
        assert_eq!(config.span_id_at(6655), 1);
        // Span 2: blocks 6656..=13055
        assert_eq!(config.span_id_at(6656), 2);
    }

    #[test]
    fn test_need_to_commit_span() {
        let config = mainnet_like_config();
        // Block 0 is explicitly excluded
        assert!(!config.need_to_commit_span(0));
        // Block 63 is a sprint end, not a sprint start → false
        assert!(!config.need_to_commit_span(63));
        // Block 64 is a sprint start (pre-Delhi, sprint=64)
        // Sprint 64..127, span 0 ends at 255 → no span crossing
        assert!(!config.need_to_commit_span(64));
        // Block 192 is a sprint start, sprint 192..255.
        // End of sprint is 255, next block 256 is span 1 → span crossing!
        assert!(config.need_to_commit_span(192));
        // Block 128 sprint 128..191, still in span 0 → no
        assert!(!config.need_to_commit_span(128));
    }
}
