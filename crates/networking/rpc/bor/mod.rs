use crate::rpc::{RpcApiContext, RpcHandler};
use crate::types::block_identifier::BlockIdentifierOrHash;
use crate::utils::RpcErr;
use ethrex_common::types::BlockHeader;
use ethrex_crypto::keccak::keccak_hash;
use serde_json::Value;
use tracing::debug;

/// Maximum number of blocks in a checkpoint range (2^15 = 32768).
const MAX_CHECKPOINT_LENGTH: u64 = 1 << 15;

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

        let length = self.end - self.start + 1;
        if length > MAX_CHECKPOINT_LENGTH {
            return Err(RpcErr::BadParams(format!(
                "checkpoint range length {length} exceeds max {MAX_CHECKPOINT_LENGTH}"
            )));
        }

        // Collect block headers for the range
        let mut headers = Vec::with_capacity(length as usize);
        for block_num in self.start..=self.end {
            let header = storage
                .get_block_header(block_num)
                .map_err(|e| RpcErr::Internal(format!("Storage error: {e}")))?
                .ok_or(RpcErr::Internal(format!("Block {block_num} not found")))?;
            headers.push(header);
        }

        // Compute binary Merkle tree root over header-derived leaves
        let root = compute_root_hash(&headers);
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
/// For each block header, the leaf is:
///   `keccak256(number_bytes32 || timestamp_bytes32 || tx_hash_bytes32 || receipt_hash_bytes32)`
/// where each field is left-zero-padded to 32 bytes.
///
/// The leaf array is padded to the next power of two with zero-filled `[u8; 32]` entries,
/// then a complete binary Merkle tree is built where each internal node is `keccak256(left || right)`.
fn compute_root_hash(headers: &[BlockHeader]) -> [u8; 32] {
    if headers.is_empty() {
        return [0u8; 32];
    }

    let padded_len = (headers.len() as u64).next_power_of_two() as usize;

    // Compute leaves from header fields
    let mut level: Vec<[u8; 32]> = Vec::with_capacity(padded_len);
    for header in headers {
        let mut data = [0u8; 128];
        // number: left-zero-padded to 32 bytes (big-endian u64 at offset 24)
        data[24..32].copy_from_slice(&header.number.to_be_bytes());
        // timestamp: left-zero-padded to 32 bytes
        data[56..64].copy_from_slice(&header.timestamp.to_be_bytes());
        // transactions_root: already 32 bytes
        data[64..96].copy_from_slice(header.transactions_root.as_bytes());
        // receipts_root: already 32 bytes
        data[96..128].copy_from_slice(header.receipts_root.as_bytes());
        level.push(keccak_hash(data));
    }

    // Pad to next power of two with zero entries
    level.resize(padded_len, [0u8; 32]);

    // Build complete binary tree bottom-up
    while level.len() > 1 {
        let mut next_level = Vec::with_capacity(level.len() / 2);
        for pair in level.chunks_exact(2) {
            let mut combined = [0u8; 64];
            combined[..32].copy_from_slice(&pair[0]);
            combined[32..].copy_from_slice(&pair[1]);
            next_level.push(keccak_hash(combined));
        }
        level = next_level;
    }

    level[0]
}
