use crate::rpc::{RpcApiContext, RpcHandler};
use crate::types::block_identifier::BlockIdentifierOrHash;
use crate::utils::RpcErr;
use ethrex_crypto::keccak::keccak_hash;
use serde_json::Value;
use tracing::debug;

/// bor_getAuthor — recover the block signer from the header's seal signature.
pub struct BorGetAuthor {
    pub block: BlockIdentifierOrHash,
}

/// bor_getSnapshot — return the validator set snapshot at a given block.
/// Requires SnapshotCache integration (pending BorEngine wiring into RpcApiContext).
pub struct BorGetSnapshot;

/// bor_getSignersAtHash — return the validator addresses from the snapshot at a given hash.
/// Requires SnapshotCache integration.
pub struct BorGetSignersAtHash;

/// bor_getCurrentValidators — return the current span's validator set.
/// Requires SnapshotCache integration.
pub struct BorGetCurrentValidators;

/// bor_getCurrentProposer — return the current block proposer.
/// Requires SnapshotCache integration.
pub struct BorGetCurrentProposer;

/// bor_getRootHash — compute the Merkle root of block hashes in a range [start, end].
/// Used by Bor for checkpoint verification.
pub struct BorGetRootHash {
    pub start: u64,
    pub end: u64,
}

impl RpcHandler for BorGetAuthor {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.is_empty() {
            return Err(RpcErr::BadParams("Expected 1 param".to_owned()));
        }
        let block = BlockIdentifierOrHash::parse(params[0].clone(), 0)?;
        Ok(Self { block })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let storage = &context.storage;
        let header = self
            .block
            .resolve_block_header(storage)
            .await?
            .ok_or(RpcErr::Internal("Block not found".to_owned()))?;

        debug!("bor_getAuthor for block {}", header.number);

        let signer = ethrex_polygon::consensus::seal::recover_signer(&header)
            .map_err(|e| RpcErr::Internal(format!("Failed to recover signer: {e}")))?;

        serde_json::to_value(format!("{signer:?}")).map_err(|e| RpcErr::Internal(e.to_string()))
    }
}

impl RpcHandler for BorGetSnapshot {
    fn parse(_params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(Self)
    }

    async fn handle(&self, _context: RpcApiContext) -> Result<Value, RpcErr> {
        // Snapshot retrieval requires the SnapshotCache to be wired into RpcApiContext
        // via the BorEngine. This will be available after BorEngine integration.
        Err(RpcErr::Internal(
            "bor_getSnapshot requires snapshot cache (pending BorEngine integration)".to_owned(),
        ))
    }
}

impl RpcHandler for BorGetSignersAtHash {
    fn parse(_params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(Self)
    }

    async fn handle(&self, _context: RpcApiContext) -> Result<Value, RpcErr> {
        Err(RpcErr::Internal(
            "bor_getSignersAtHash requires snapshot cache (pending BorEngine integration)"
                .to_owned(),
        ))
    }
}

impl RpcHandler for BorGetCurrentValidators {
    fn parse(_params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(Self)
    }

    async fn handle(&self, _context: RpcApiContext) -> Result<Value, RpcErr> {
        Err(RpcErr::Internal(
            "bor_getCurrentValidators requires snapshot cache (pending BorEngine integration)"
                .to_owned(),
        ))
    }
}

impl RpcHandler for BorGetCurrentProposer {
    fn parse(_params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(Self)
    }

    async fn handle(&self, _context: RpcApiContext) -> Result<Value, RpcErr> {
        Err(RpcErr::Internal(
            "bor_getCurrentProposer requires snapshot cache (pending BorEngine integration)"
                .to_owned(),
        ))
    }
}

impl RpcHandler for BorGetRootHash {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 2 {
            return Err(RpcErr::BadParams(
                "Expected 2 params (start, end)".to_owned(),
            ));
        }
        let start = parse_block_number_param(&params[0], 0)?;
        let end = parse_block_number_param(&params[1], 1)?;
        if start > end {
            return Err(RpcErr::BadParams(
                "start block must be <= end block".to_owned(),
            ));
        }
        Ok(Self { start, end })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let storage = &context.storage;

        debug!("bor_getRootHash for range [{}, {}]", self.start, self.end);

        // Collect block hashes for the range
        let count = (self.end - self.start + 1) as usize;
        let mut hashes = Vec::with_capacity(count);
        for block_num in self.start..=self.end {
            let header = storage
                .get_block_header(block_num)
                .map_err(|e| RpcErr::Internal(format!("Storage error: {e}")))?
                .ok_or(RpcErr::Internal(format!("Block {block_num} not found")))?;
            hashes.push(header.hash());
        }

        // Compute binary Merkle tree root over keccak256(blockHash) leaves
        let root = compute_root_hash(&hashes);
        let root_hex = hex::encode(root);

        serde_json::to_value(root_hex).map_err(|e| RpcErr::Internal(e.to_string()))
    }
}

/// Parse a block number from a JSON param (accepts both hex string "0x..." and integer).
fn parse_block_number_param(value: &Value, arg_index: u64) -> Result<u64, RpcErr> {
    // Try as integer first
    if let Some(n) = value.as_u64() {
        return Ok(n);
    }
    // Try as hex string
    if let Some(s) = value.as_str() {
        let s = s.strip_prefix("0x").unwrap_or(s);
        return u64::from_str_radix(s, 16).map_err(|_| RpcErr::BadHexFormat(arg_index));
    }
    Err(RpcErr::BadParams(format!(
        "param {arg_index}: expected block number"
    )))
}

/// Compute the Bor checkpoint root hash.
///
/// Builds a binary Merkle tree where each leaf is `keccak256(block_hash_bytes)`
/// and each internal node is `keccak256(left || right)`.
/// If the number of elements at any level is odd, the last element is promoted as-is.
fn compute_root_hash(block_hashes: &[ethrex_common::H256]) -> [u8; 32] {
    if block_hashes.is_empty() {
        return [0u8; 32];
    }

    // Leaves: keccak256 of each block hash
    let mut level: Vec<[u8; 32]> = block_hashes
        .iter()
        .map(|h| keccak_hash(h.as_bytes()))
        .collect();

    // Build tree bottom-up
    while level.len() > 1 {
        let mut next_level = Vec::with_capacity((level.len() + 1) / 2);
        let mut i = 0;
        while i + 1 < level.len() {
            let mut combined = [0u8; 64];
            combined[..32].copy_from_slice(&level[i]);
            combined[32..].copy_from_slice(&level[i + 1]);
            next_level.push(keccak_hash(&combined));
            i += 2;
        }
        // Odd element: promote as-is
        if i < level.len() {
            next_level.push(level[i]);
        }
        level = next_level;
    }

    level[0]
}
