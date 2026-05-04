use ethrex_common::{Address, H256, U256};
use serde::Serialize;
use serde::Serializer;

/// Binary trie execution witness for a block.
///
/// Contains all binary trie proofs needed for stateless verification:
/// every accessed account, storage slot, and code hash gets a proof
/// (sibling hashes from root to leaf) against the pre-execution state root.
///
/// Verification flow:
/// 1. Check that all proofs verify against `pre_state_root`
/// 2. Reconstruct pre-execution state from the proven values
/// 3. Re-execute the block against that state
/// 4. Verify the resulting state root matches the block header
#[derive(Debug, Clone, Default, Serialize)]
pub struct BinaryTrieWitness {
    /// Block number this witness is for.
    pub block_number: u64,
    /// Block hash this witness is for.
    pub block_hash: H256,
    /// Binary trie state root BEFORE block execution.
    /// All proofs in this witness verify against this root.
    #[serde(serialize_with = "serialize_hash")]
    pub pre_state_root: [u8; 32],
    /// Proofs for accessed account basic_data and code_hash keys.
    pub account_proofs: Vec<AccountWitnessEntry>,
    /// Proofs for accessed storage slot keys.
    pub storage_proofs: Vec<StorageWitnessEntry>,
    /// Accessed contract bytecodes.
    pub codes: Vec<CodeWitnessEntry>,
    /// Block headers needed for BLOCKHASH opcode (RLP-encoded, 0x-prefixed hex).
    #[serde(serialize_with = "serialize_bytes_vec")]
    pub block_headers: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AccountWitnessEntry {
    pub address: Address,
    /// Pre-execution balance.
    pub balance: U256,
    /// Pre-execution nonce.
    pub nonce: u64,
    /// Pre-execution code hash.
    pub code_hash: H256,
    /// Proof for the basic_data key.
    pub basic_data_proof: ProofEntry,
    /// Proof for the code_hash key.
    pub code_hash_proof: ProofEntry,
}

#[derive(Debug, Clone, Serialize)]
pub struct StorageWitnessEntry {
    pub address: Address,
    pub slot: H256,
    /// Pre-execution storage value.
    pub value: U256,
    pub proof: ProofEntry,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodeWitnessEntry {
    pub code_hash: H256,
    #[serde(serialize_with = "serialize_bytes")]
    pub bytecode: Vec<u8>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProofEntry {
    /// Sibling hashes from root to leaf.
    #[serde(serialize_with = "serialize_hash_vec")]
    pub siblings: Vec<[u8; 32]>,
    /// Depth at which the StemNode was found.
    pub stem_depth: usize,
    /// The leaf value, if present.
    #[serde(serialize_with = "serialize_optional_hash")]
    pub value: Option<[u8; 32]>,
}

// ── Serde helpers ──────────────────────────────────────────────────────────

fn serialize_hash<S: Serializer>(bytes: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&format!("0x{}", hex::encode(bytes)))
}

fn serialize_optional_hash<S: Serializer>(opt: &Option<[u8; 32]>, s: S) -> Result<S::Ok, S::Error> {
    match opt {
        Some(bytes) => s.serialize_some(&format!("0x{}", hex::encode(bytes))),
        None => s.serialize_none(),
    }
}

fn serialize_hash_vec<S: Serializer>(hashes: &[[u8; 32]], s: S) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut seq = s.serialize_seq(Some(hashes.len()))?;
    for h in hashes {
        seq.serialize_element(&format!("0x{}", hex::encode(h)))?;
    }
    seq.end()
}

fn serialize_bytes<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&format!("0x{}", hex::encode(bytes)))
}

fn serialize_bytes_vec<S: Serializer>(items: &[Vec<u8>], s: S) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut seq = s.serialize_seq(Some(items.len()))?;
    for b in items {
        seq.serialize_element(&format!("0x{}", hex::encode(b)))?;
    }
    seq.end()
}
