//! The execution witness of the block the t8n builds: storing the ancestor
//! headers it needs, converting it to the result's format, and validating
//! the block statelessly against it.

use std::collections::BTreeMap;
use std::sync::Arc;

use bytes::Bytes;
use ethrex_common::H256;
use ethrex_common::types::block_execution_witness::{
    ExecutionWitness, RpcExecutionWitness, decode_witness_headers, validate_witness_headers_chain,
};
use ethrex_common::types::{Block, BlockHeader, ChainConfig};
use ethrex_crypto::NativeCrypto;
use ethrex_guest_program::l1::validate_blocks_statelessly;
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::Store;
use serde_json::{Value, json};

use super::error::T8nError;

/// The witness as it goes in the result: `state` and `codes` sorted
/// lexicographically, and `headers` ascending by block number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WitnessOutput {
    pub state: Vec<Bytes>,
    pub codes: Vec<Bytes>,
    pub headers: Vec<Bytes>,
}

impl WitnessOutput {
    pub fn new(witness: ExecutionWitness) -> Result<Self, T8nError> {
        // The conversion sorts the three lists the way the result needs them.
        let RpcExecutionWitness {
            state,
            codes,
            headers,
            ..
        } = RpcExecutionWitness::try_from(witness).map_err(|e| T8nError::Witness(e.to_string()))?;
        Ok(Self {
            state,
            codes,
            headers,
        })
    }

    pub fn to_json(&self) -> Value {
        let hex_list = |items: &[Bytes]| -> Vec<String> {
            items
                .iter()
                .map(|item| format!("0x{}", hex::encode(item)))
                .collect()
        };
        json!({
            "state": hex_list(&self.state),
            "codes": hex_list(&self.codes),
            "headers": hex_list(&self.headers),
        })
    }

    /// Execute `block` with nothing but this witness, the way a stateless
    /// client would, and say why it fails if it does.
    ///
    /// The witness is rebuilt from the lists that go in the result, and not
    /// from the tries the node generated it from, so a node missing from
    /// `state` fails here as it would for whoever reads the result.
    pub fn validate(&self, block: &Block, chain_config: ChainConfig) -> Result<(), String> {
        let headers = decode_witness_headers(&self.headers).map_err(|e| e.to_string())?;
        validate_witness_headers_chain(&headers, &NativeCrypto).map_err(|e| e.to_string())?;
        let witness = RpcExecutionWitness {
            state: self.state.clone(),
            keys: Vec::new(),
            codes: self.codes.clone(),
            headers: self.headers.clone(),
        }
        .into_execution_witness(chain_config, block.header.number, &headers, &NativeCrypto)
        .map_err(|e| e.to_string())?;
        // Decode the block again so it is validated as its consumer gets it,
        // without anything the build cached in it.
        let block = Block::decode(&block.encode_to_vec()).map_err(|e| e.to_string())?;
        validate_blocks_statelessly(&[block], witness, Arc::new(NativeCrypto))
            .map_err(|e| e.to_string())
    }
}

/// Store the ancestor headers from the environment so the node can put them
/// in the witness, and return the hash of the parent.
///
/// `pre_state_root` is the root of the pre-state alloc. The parent header has
/// to commit to it, since the witness proves the pre-state against the state
/// root of that header.
pub async fn store_ancestor_headers(
    store: &Store,
    ancestors: BTreeMap<u64, BlockHeader>,
    block_number: u64,
    pre_state_root: H256,
) -> Result<H256, T8nError> {
    let parent = block_number
        .checked_sub(1)
        .and_then(|parent_number| ancestors.get(&parent_number))
        .ok_or_else(|| {
            T8nError::Witness(format!(
                "blockHeaders has no header for the parent of block {block_number}"
            ))
        })?;
    if parent.state_root != pre_state_root {
        return Err(T8nError::Witness(format!(
            "blockHeaders[{}] has state root {:#x}, but the root of the pre-state alloc is \
             {pre_state_root:#x}",
            parent.number, parent.state_root
        )));
    }
    let parent_hash = parent.hash();
    for header in ancestors.into_values() {
        store
            .add_block_header(header.hash(), header)
            .await
            .map_err(|e| T8nError::Store(e.to_string()))?;
    }
    Ok(parent_hash)
}
