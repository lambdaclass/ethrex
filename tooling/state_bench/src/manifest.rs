//! Manifest emitted by `gen-state` and consumed by later phases.
//!
//! The manifest is the single source of truth that ties a generated datadir to
//! the deterministic parameters used to build it. Later subcommands (`gen-workload`,
//! `run`) read it back to reconstruct the accessor ABI, the funded signer, and the
//! set of addresses/slots that were seeded, without re-deriving them independently.

use serde::{Deserialize, Serialize};

/// File name written into the datadir alongside the RocksDB directory.
pub const MANIFEST_FILENAME: &str = "state-bench-manifest.json";

/// On-disk (SST) byte sizes of the four state column families, measured after
/// the datadir is fully written and closed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateCfSizes {
    pub account_trie_nodes: u64,
    pub storage_trie_nodes: u64,
    pub account_flatkeyvalue: u64,
    pub storage_flatkeyvalue: u64,
}

/// Everything a downstream phase needs to know about a generated datadir.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// `ethrex_storage::STORE_SCHEMA_VERSION` the datadir was built against.
    pub schema_version: u64,
    /// Deterministic seed all derivations hang off of.
    pub seed: u64,
    /// Worker count resolved at generation time (`--jobs` or ambient CPUs).
    pub jobs: usize,
    pub num_small_accounts: u64,
    pub slots_per_account: u64,
    pub mega_account_gb: f64,

    /// Address of the shared accessor contract (0x-prefixed hex).
    pub accessor_contract_address: String,
    /// Human-readable description of the accessor's calldata ABI.
    pub accessor_calldata_abi: String,
    /// The accessor's deployed bytecode (0x-prefixed hex).
    pub accessor_bytecode: String,

    /// Address of the single mega storage account (0x-prefixed hex).
    pub mega_account_address: String,
    /// Exact rule used to derive small-account addresses, slots, and values.
    pub small_account_derivation_rule: String,

    /// Deterministically-minted, funded EOA that signs Phase-3 workload txs.
    pub funded_signer_private_key: String,
    pub funded_signer_address: String,

    /// Final state root of the generated state (0x-prefixed hex).
    pub computed_state_root: String,

    /// On-disk sizes of the four state CFs (bytes).
    pub state_cf_sizes: StateCfSizes,

    /// Total on-disk `STORAGE_TRIE_NODES` CF bytes (mega account dominates at
    /// default scale; also includes the small accounts' storage nodes). Used as
    /// the mega-size proxy for the ±10% target check.
    pub mega_storage_bytes_achieved: u64,
    /// Target storage-trie-node bytes for the mega account.
    pub mega_target_bytes: u64,
    /// Achieved / target, as a percentage.
    pub mega_percent_of_target: f64,
}
