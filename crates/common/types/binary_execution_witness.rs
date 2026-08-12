//! The EIP-8297 execution witness: what `debug_executionWitnessV2` returns.
//!
//! # The contract, and where it differs from the MPT one
//!
//! [`RpcExecutionWitness`] is a flat list of MPT node *encodings* plus the
//! bytecodes and ancestor headers a re-execution needs; a verifier rebuilds a
//! trie from `keccak(encoding) -> node`, re-executes, and checks the resulting
//! root against `header.stateRoot`. This mirrors that, node encoding for node
//! encoding, with the binary trie's hash (BLAKE3) and node format in place of
//! the MPT's.
//!
//! **A witness is over node encodings, never over storage rows.** A node's
//! stored bytes *are* its BLAKE3 preimage, so they are consensus-visible and
//! the same on every node. `BINARY_TRIE_NODES` rows are a storage container
//! whose span is the datadir's group depth, which is unsettled and which
//! differs between nodes: a witness of rows would fail to verify against a peer
//! running another depth. See `ethrex_binary_trie::trie::witness`, which is
//! where that independence is enforced and tested.
//!
//! Two things differ from the V1 shape, both forced:
//!
//! - **an explicit `format` discriminator.** V1 has none, and does not need one
//!   — it is the only shape that method ever returns. Here there are now two
//!   witness formats in the same client for the same chain, distinguished by a
//!   *timestamp*, and a consumer that mistook one for the other would rebuild
//!   the wrong kind of trie out of bytes that happen to decode. `debug_execution
//!   Witness` is already documented as ethrex-specific and incompatible with
//!   other implementations (`crates/networking/rpc/clients/eth/mod.rs`), so
//!   there is no cross-client shape to match and no reason not to say what this
//!   is.
//! - **an explicit `preStateRoot`.** V1 recovers it from the parent header in
//!   `headers`, because a parent header's `stateRoot` *is* the pre-state root.
//!   That fails at exactly the block this method exists for: the first
//!   binary-committed block's parent is pre-flip and its header commits an MPT
//!   root, so the binary pre-state root appears in no header anywhere. It has
//!   to be carried. Nothing is taken on trust for it — a wrong `preStateRoot`
//!   produces a post-state root that does not match `header.stateRoot`, which
//!   is the check every consumer already has to make.

use bytes::Bytes;
use ethereum_types::H256;
use ethrex_binary_trie::trie::witness::{WitnessBinaryTrieDB, WitnessError};
use ethrex_binary_trie::trie::{BinaryTrie, EMPTY_TRIE_ROOT as BINARY_EMPTY_TRIE_ROOT};
use serde::{Deserialize, Serialize};

use crate::serde_utils;
use crate::types::{BlockHeader, ChainConfig};

/// The `format` string every `debug_executionWitnessV2` response carries.
///
/// Versioned separately from the method name so the shape can change without a
/// `V3`, and namespaced to ethrex because this witness is ethrex's own — see
/// the module docs.
pub const BINARY_WITNESS_FORMAT: &str = "ethrex-eip8297-binary-witness-v1";

/// Witness data produced by the client and consumed by a stateless verifier.
///
/// The internal counterpart of [`RpcBinaryExecutionWitness`], carrying the two
/// things the wire form leaves to the caller — the chain configuration and the
/// first block's number — exactly as [`ExecutionWitness`] does on the MPT side.
///
/// [`ExecutionWitness`]: crate::types::block_execution_witness::ExecutionWitness
#[derive(Clone, Debug, Default)]
pub struct BinaryExecutionWitness {
    /// Binary-trie node encodings: BLAKE3 preimages, in no particular order.
    pub nodes: Vec<Vec<u8>>,
    /// Contract bytecodes needed for stateless execution.
    pub codes: Vec<Vec<u8>>,
    /// RLP-encoded block headers needed for stateless execution: the parent of
    /// the first block, every ancestor a `BLOCKHASH` reached, and the blocks in
    /// between.
    pub block_headers_bytes: Vec<Vec<u8>>,
    /// The binary root the witness is *over* — the state the first block
    /// executes against. See the module docs for why this cannot be read off a
    /// header.
    pub pre_state_root: H256,
    /// The block number of the first block.
    pub first_block_number: u64,
    /// The chain configuration.
    pub chain_config: ChainConfig,
}

/// RPC-friendly representation of an EIP-8297 execution witness.
///
/// Field-for-field the V1 shape (`state`/`codes`/`headers` as byte lists) plus
/// the two fields the module docs explain, and minus V1's vestigial `keys`,
/// which is empty in every producer and is being removed from the spec.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcBinaryExecutionWitness {
    /// Always [`BINARY_WITNESS_FORMAT`]. Checked on the way in, not assumed.
    pub format: String,
    pub pre_state_root: H256,
    #[serde(
        serialize_with = "serde_utils::bytes::vec::serialize",
        deserialize_with = "serde_utils::bytes::vec::deserialize"
    )]
    pub state: Vec<Bytes>,
    #[serde(
        serialize_with = "serde_utils::bytes::vec::serialize",
        deserialize_with = "serde_utils::bytes::vec::deserialize"
    )]
    pub codes: Vec<Bytes>,
    #[serde(
        serialize_with = "serde_utils::bytes::vec::serialize",
        deserialize_with = "serde_utils::bytes::vec::deserialize"
    )]
    pub headers: Vec<Bytes>,
}

/// Why an [`RpcBinaryExecutionWitness`] could not be turned into a usable
/// pre-state.
#[derive(Debug, thiserror::Error, PartialEq, Eq, Clone)]
pub enum BinaryWitnessError {
    /// The `format` field is not [`BINARY_WITNESS_FORMAT`].
    ///
    /// Rejected before anything is decoded. The alternative — decoding first
    /// and letting the shape fail — is exactly the confusion the discriminator
    /// exists to prevent, since an MPT witness's `state` is also a list of byte
    /// strings and would simply come out as a tree with no nodes anything
    /// names.
    #[error("expected witness format {expected:?}, got {found:?}")]
    WrongFormat { expected: String, found: String },
    /// The node list is not a witness for `preStateRoot`.
    #[error(transparent)]
    Witness(#[from] WitnessError),
}

impl From<BinaryExecutionWitness> for RpcBinaryExecutionWitness {
    fn from(value: BinaryExecutionWitness) -> Self {
        // Canonical ordering, matching what `RpcExecutionWitness` does for the
        // MPT: nodes and codes sorted and deduplicated, headers ascending by
        // block number and deduplicated. Two witnesses for the same block must
        // serialize identically or nothing downstream can be compared.
        let mut nodes = value.nodes;
        nodes.sort();
        nodes.dedup();
        let mut codes = value.codes;
        codes.sort();
        codes.dedup();
        let mut headers = value.block_headers_bytes;
        headers.sort_by_cached_key(|bytes| {
            // Undecodable headers sort last; consumers reject them on decode.
            <BlockHeader as ethrex_rlp::decode::RLPDecode>::decode(bytes)
                .map(|header| header.number)
                .unwrap_or(u64::MAX)
        });
        headers.dedup();
        Self {
            format: BINARY_WITNESS_FORMAT.to_string(),
            pre_state_root: value.pre_state_root,
            state: nodes.into_iter().map(Bytes::from).collect(),
            codes: codes.into_iter().map(Bytes::from).collect(),
            headers: headers.into_iter().map(Bytes::from).collect(),
        }
    }
}

impl RpcBinaryExecutionWitness {
    /// Check the discriminator and index the nodes against `preStateRoot`.
    ///
    /// The returned trie reads exactly the state this witness proves and fails
    /// at its frontier; it holds no database. See
    /// [`WitnessBinaryTrieDB`](ethrex_binary_trie::trie::witness::WitnessBinaryTrieDB)
    /// for what is checked.
    ///
    /// # Errors
    ///
    /// [`BinaryWitnessError::WrongFormat`] if `format` is not
    /// [`BINARY_WITNESS_FORMAT`], and [`BinaryWitnessError::Witness`] for every
    /// way the node list can fail to be a witness for this root.
    pub fn into_pre_state_trie(&self) -> Result<BinaryTrie, BinaryWitnessError> {
        if self.format != BINARY_WITNESS_FORMAT {
            return Err(BinaryWitnessError::WrongFormat {
                expected: BINARY_WITNESS_FORMAT.to_string(),
                found: self.format.clone(),
            });
        }
        let nodes: Vec<Vec<u8>> = self.state.iter().map(|node| node.to_vec()).collect();
        let db = WitnessBinaryTrieDB::new(self.pre_state_root, &nodes)?;
        Ok(BinaryTrie::open(Box::new(db), self.pre_state_root))
    }

    /// Whether this witness claims the empty binary tree as its pre-state.
    ///
    /// Only genesis can honestly claim that, so a consumer wanting to reject it
    /// has a name for the question.
    pub fn is_over_the_empty_tree(&self) -> bool {
        self.pre_state_root == BINARY_EMPTY_TRIE_ROOT
    }
}
