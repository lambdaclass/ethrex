//! Shared types for the native rollup L2 PoC.

use bytes::Bytes;
use ethrex_common::{Address, H256, U256};
use ethrex_crypto::keccak::keccak_hash;

/// An L1 message recorded on the NativeRollup contract via sendL1Message().
///
/// The L1 watcher parses `L1MessageRecorded` events into this struct and
/// forwards them to the block producer via an `EnqueueL1Messages` message.
#[derive(Clone, Debug)]
pub struct L1Message {
    pub sender: Address,
    pub to: Address,
    pub nonce: U256,
    pub value: U256,
    pub gas_limit: u64,
    /// Full calldata bytes forwarded to the L2 target contract.
    pub data: Bytes,
    /// Precomputed keccak256(_data) — computed by the watcher at parse time
    /// so the block producer and advancer don't need to recompute it.
    pub data_hash: H256,
}

impl L1Message {
    /// Compute keccak256(abi.encodePacked(sender, to, value, gasLimit, dataHash, nonce))
    /// matching NativeRollup.sol `_recordL1Message`.
    pub fn compute_hash(&self) -> H256 {
        let mut preimage = Vec::with_capacity(168);
        preimage.extend_from_slice(self.sender.as_bytes()); // 20 bytes
        preimage.extend_from_slice(self.to.as_bytes()); // 20 bytes
        preimage.extend_from_slice(&self.value.to_big_endian()); // 32 bytes
        preimage.extend_from_slice(&U256::from(self.gas_limit).to_big_endian()); // 32 bytes
        preimage.extend_from_slice(self.data_hash.as_bytes()); // 32 bytes
        preimage.extend_from_slice(&self.nonce.to_big_endian()); // 32 bytes
        H256(keccak_hash(&preimage))
    }
}
