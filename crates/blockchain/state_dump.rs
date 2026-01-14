use ethrex_common::{Address, H256};
use ethrex_trie::Nibbles;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

type StateTracker = Arc<Mutex<StateAccessTracker>>;

#[derive(Debug, Clone)]
pub struct StateDumpConfig {
    pub enabled: bool,
    pub dump_dir: PathBuf,
    pub auto_save: bool,
}

impl StateDumpConfig {
    pub fn new(dump_dir: PathBuf) -> Self {
        Self {
            enabled: true,
            dump_dir,
            auto_save: true,
        }
    }
}

/// Tracks state access during block execution for diagnostic purposes
#[derive(Debug, Clone, Default)]
pub struct StateAccessTracker {
    /// Accounts accessed during execution
    pub accounts_accessed: HashSet<Address>,
    /// Storage slots accessed (account -> set of keys)
    pub storage_accessed: HashMap<Address, HashSet<H256>>,
    /// Trie nodes accessed (path -> node data)
    pub trie_nodes_accessed: HashMap<Nibbles, Vec<u8>>,
    /// Whether the full key-value store was accessed
    pub fkv_accessed: bool,
    /// Block hashes accessed
    pub block_hashes_accessed: HashSet<u64>,
    /// Code hashes accessed
    pub code_hashes_accessed: HashSet<H256>,
    /// Timestamp when tracking started
    pub start_time: Option<SystemTime>,
}

impl StateAccessTracker {
    pub fn new() -> Self {
        Self {
            start_time: Some(SystemTime::now()),
            ..Default::default()
        }
    }

    pub fn record_account_access(&mut self, address: Address) {
        self.accounts_accessed.insert(address);
    }

    pub fn record_storage_access(&mut self, address: Address, key: H256) {
        self.storage_accessed.entry(address).or_default().insert(key);
    }

    pub fn record_trie_node_access(&mut self, path: Nibbles, data: Vec<u8>) {
        self.trie_nodes_accessed.insert(path, data);
    }

    pub fn record_fkv_access(&mut self) {
        self.fkv_accessed = true;
    }

    pub fn record_block_hash_access(&mut self, block_number: u64) {
        self.block_hashes_accessed.insert(block_number);
    }

    pub fn record_code_access(&mut self, code_hash: H256) {
        self.code_hashes_accessed.insert(code_hash);
    }
}

/// Serializable dump of state access information
#[derive(Debug, Serialize, Deserialize)]
pub struct StateAccessDump {
    pub block_number: u64,
    pub block_hash: H256,
    pub error_type: String,
    pub timestamp: u64,
    pub accounts_accessed: Vec<String>,
    pub storage_accessed: HashMap<String, Vec<String>>,
    pub trie_nodes_count: usize,
    pub trie_node_paths: Vec<String>,
    pub fkv_accessed: bool,
    pub block_hashes_accessed: Vec<u64>,
    pub code_hashes_accessed: Vec<String>,
    pub execution_duration_ms: Option<u64>,
}

impl StateAccessDump {
    pub fn from_tracker(
        tracker: &StateAccessTracker,
        block_number: u64,
        block_hash: H256,
        error_type: String,
    ) -> Self {
        let execution_duration_ms = tracker.start_time.and_then(|start| {
            SystemTime::now()
                .duration_since(start)
                .ok()
                .map(|d| d.as_millis() as u64)
        });

        Self {
            block_number,
            block_hash,
            error_type,
            timestamp: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("System time should be after UNIX_EPOCH")
                .as_secs(),
            accounts_accessed: tracker
                .accounts_accessed
                .iter()
                .map(|a| format!("{:?}", a))
                .collect(),
            storage_accessed: tracker
                .storage_accessed
                .iter()
                .map(|(addr, keys)| {
                    (
                        format!("{:?}", addr),
                        keys.iter().map(|k| format!("{:?}", k)).collect(),
                    )
                })
                .collect(),
            trie_nodes_count: tracker.trie_nodes_accessed.len(),
            trie_node_paths: tracker
                .trie_nodes_accessed
                .keys()
                .map(|n| hex::encode(n.as_ref()))
                .collect(),
            fkv_accessed: tracker.fkv_accessed,
            block_hashes_accessed: tracker.block_hashes_accessed.iter().copied().collect(),
            code_hashes_accessed: tracker
                .code_hashes_accessed
                .iter()
                .map(|h| format!("{:?}", h))
                .collect(),
            execution_duration_ms,
        }
    }

    pub fn save_to_file(&self, dir: &Path) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(dir)?;
        let filename = format!(
            "block_{}_{}_{}.json",
            self.block_number,
            self.error_type.replace(' ', "_"),
            self.timestamp
        );
        let path = dir.join(filename);
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        std::fs::write(&path, json)?;
        Ok(path)
    }
}

/// Generate and save dump to file
pub fn generate_and_save_dump(
    tracker: &StateTracker,
    block_number: u64,
    block_hash: H256,
    error: &str,
    dump_dir: &Path,
) -> Option<PathBuf> {
    tracker.lock().ok().and_then(|t| {
        StateAccessDump::from_tracker(&t, block_number, block_hash, error.to_string())
            .save_to_file(dump_dir)
            .ok()
    })
}
