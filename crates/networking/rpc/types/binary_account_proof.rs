//! The response type of `eth_getProofV2`: an EIP-8297 binary-trie proof.
//!
//! # Why this is not `eth_getProof`'s shape
//!
//! Past the EIP-8297 activation the state commitment is a BLAKE3 binary radix
//! trie over one flat tree-key space. There is no account trie and no
//! per-account storage trie, so `eth_getProof`'s response — a list of
//! RLP-encoded MPT nodes plus a `storageHash` naming a storage subtrie — has no
//! referent, and `eth_getProof` refuses rather than improvise one (see
//! `crates/networking/rpc/eth/account.rs`). This is the separate method that
//! serves the binary shape, so that the standard method's schema stays free
//! until execution-apis settles one.
//!
//! # The shape, and the reasoning behind each choice
//!
//! **`proofFormat` is mandatory and comes first.** On a chain that flips
//! mid-history both an MPT proof and a binary proof are legitimately servable
//! at different block numbers of the same chain, so a consumer cannot tell them
//! apart except by out-of-band knowledge of the fork schedule. The
//! discriminator removes that, and it is what lets this format be superseded:
//! a future response carrying a different string is a different format, loudly.
//!
//! **`accountProof` is a list of per-tree-key walks, not a list of nodes.**
//! There is no "the account leaf" to prove. The embedding scatters an account
//! across sub-indices of the header stem `0x00 || key_hash(address)`: basic
//! data at sub-index 0, the code hash at 1, the EIP-7702 delegation indicator
//! at 2. So an account proof is a multi-key proof, and this field says so
//! structurally — it is an array of objects exactly as EIP-1186's
//! `storageProof` is, and a consumer that treats it as an array of hex node
//! strings fails on the element type rather than silently.
//!
//! **All three header keys are always proven, including the absent ones.** The
//! code hash and the delegation indicator are mutually exclusive: every
//! existing account holds exactly one. A response that simply omitted the one
//! it does not hold would report absence without proving it, and a delegated
//! account would come back with nothing at all about the leaf that determines
//! its code. Each entry therefore carries a real walk, and an absent key's walk
//! is its *exclusion* proof.
//!
//! **There is no `storageHash`, and no other summary of storage.** It has no
//! referent: the design has no per-account storage root at all. Keeping
//! `eth_getProof`'s schema would force a choice between dropping the field and
//! zeroing it; this method has no EIP-1186 consumer to satisfy, so the field
//! simply does not exist. A zero would be a value that means nothing, which is
//! worse than a missing key on a response whose format is already declared.
//!
//! **Every storage entry says which zone its slot lives in.** Slots `0..=63`
//! live in the account's header stem and every other slot lives under a
//! different stem in the storage zone, so the two are proven at different
//! places in the tree. `zone` and `treeKey` make that visible instead of
//! leaving the verifier to re-derive it and hope it agrees with us — the
//! verifier *must* still re-derive the tree key from the address and the slot
//! and compare, because a `treeKey` we chose is not a `treeKey` it trusts.
//!
//! **Nothing is deduplicated.** The three header walks share a stem and
//! therefore share their leading nodes, and a slot below 64 shares them too.
//! Each entry nevertheless carries its whole walk from the root. That is
//! redundant on the wire and it is deliberate: it is exactly the input
//! `verify_walk` takes, so verification is per-entry with no reassembly step
//! and no new proof machinery. A deduplicated multiproof is a different format
//! and can carry a different `proofFormat`.
//!
//! # How a client verifies this
//!
//! Against the block header's own `stateRoot`, obtained independently — the
//! `stateRoot` echoed here is a convenience, and a client that verifies against
//! it rather than against a header it fetched itself has verified nothing.
//!
//! For each entry, with `key` the tree key the client re-derived for itself:
//!
//! ```text
//! let (_steps, end) = verify_walk(header.state_root, key, entry.proof)?;
//! match end {
//!     WalkEnd::AtLeaf { key: found, value } if found == key => present(value),
//!     WalkEnd::AtLeaf { .. } | WalkEnd::Diverged { .. } | WalkEnd::Empty => absent,
//! }
//! ```
//!
//! That second arm is the exclusion judgement, and it is sound because
//! `verify_walk` recomputes the descent from node bytes the hash chain pins:
//! the walk ended where the target key's own descent must end, so had the
//! target been present the descent would have continued. `verify_walk`
//! deliberately leaves this judgement to the caller — "ended at somebody
//! else's leaf" is a *successful* walk — which is why the rule is written down
//! here rather than assumed.
//!
//! The `value`, `balance`, `nonce` and `codeHash` fields beside the proofs are
//! conveniences derived from the proven leaves. A verifier decodes them from
//! the walk terminals; it does not read them off the response.

use ethrex_common::{Address, H256, U256, serde_utils};
use serde::{Serialize, Serializer};

use super::account_proof::serialize_proofs;

/// The `proofFormat` discriminator this module emits.
///
/// Namespaced to ethrex on purpose: this is one client's shape for a format no
/// EIP covers, and it must not be mistaken for a standard one. When
/// execution-apis settles a shape, that shape gets its own string.
pub const BINARY_PROOF_FORMAT: &str = "ethrex-eip8297-walk-v1";

/// Which of the account's header leaves an [`AccountFieldProof`] is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AccountField {
    /// Sub-index 0: version, code size, nonce and balance, packed.
    BasicData,
    /// Sub-index 1: the code hash. Absent on a delegated account.
    CodeHash,
    /// Sub-index 2: the EIP-7702 delegation indicator. Absent on an
    /// undelegated one.
    Delegation,
}

/// Where in the tree a storage slot lives.
///
/// Not cosmetic: the two zones have different key lengths and different stems,
/// so a verifier that re-derives the tree key needs to know which derivation
/// the server used — and needs to be able to disagree with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StorageZone {
    /// Slots `0..=63`, at sub-indices `64..=127` of the account's header stem.
    AccountHeader,
    /// Every slot from 64 up, under `0xFF || key_hash(address) || …`.
    Storage,
}

/// One header leaf of the account, proven present or absent.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountFieldProof {
    pub field: AccountField,
    /// The tree key this walk is for. The verifier re-derives it and compares;
    /// it is on the wire so a mismatch is diagnosable, not so it is trusted.
    #[serde(serialize_with = "serialize_hex")]
    pub tree_key: Vec<u8>,
    /// The 32-byte leaf value, or `null` when the walk proves absence.
    #[serde(serialize_with = "serialize_optional_hex")]
    pub value: Option<[u8; 32]>,
    /// The walk: stored-node encodings from the root to the terminal, root
    /// first.
    #[serde(serialize_with = "serialize_proofs")]
    pub proof: Vec<Vec<u8>>,
}

/// One storage slot, proven present or absent.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BinaryStorageProof {
    /// The slot the caller asked for, as EIP-1186 encodes it.
    pub key: U256,
    /// The tree key that slot maps to under the EIP-8297 embedding.
    #[serde(serialize_with = "serialize_hex")]
    pub tree_key: Vec<u8>,
    pub zone: StorageZone,
    /// The slot's value. Zero when the walk proves absence, which is the same
    /// convention `eth_getProof` and the EVM already use: a slot written to
    /// zero is stored as absent, so absence and zero are one state.
    pub value: U256,
    #[serde(serialize_with = "serialize_proofs")]
    pub proof: Vec<Vec<u8>>,
}

/// An `eth_getProofV2` response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BinaryAccountProof {
    /// Always [`BINARY_PROOF_FORMAT`]. First field so it is the first thing a
    /// reader of the raw JSON sees.
    pub proof_format: &'static str,
    pub address: Address,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub block_number: u64,
    pub block_hash: H256,
    /// The root every walk in this response verifies against, echoed from the
    /// header. A client checks it against a header it obtained itself.
    pub state_root: H256,
    pub balance: U256,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub nonce: u64,
    /// The account's code hash as the binary trie reports it: the code-hash
    /// leaf when there is one, `keccak(indicator)` when the account is
    /// delegated, and the empty-code hash when it holds neither.
    pub code_hash: H256,
    /// The header leaves that together constitute the account: basic data,
    /// code hash, delegation — in that order, always all three.
    pub account_proof: Vec<AccountFieldProof>,
    pub storage_proof: Vec<BinaryStorageProof>,
}

fn serialize_hex<S>(value: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&format!("0x{}", hex::encode(value)))
}

fn serialize_optional_hex<S>(value: &Option<[u8; 32]>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match value {
        Some(bytes) => serializer.serialize_str(&format!("0x{}", hex::encode(bytes))),
        None => serializer.serialize_none(),
    }
}
