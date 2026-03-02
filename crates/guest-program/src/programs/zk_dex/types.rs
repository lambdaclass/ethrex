use rkyv::{Archive, Deserialize as RDeserialize, Serialize as RSerialize};
use serde::{Deserialize, Serialize};

/// Input for the ZK-DEX guest program.
///
/// Represents a batch of token transfers to be proven.  The prover must
/// demonstrate that every transfer is valid (sufficient balance, correct
/// nonce) and that the `initial_state_root` transitions deterministically
/// to the `final_state_root` computed by the execution function.
#[derive(Serialize, Deserialize, RSerialize, RDeserialize, Archive, Clone, Debug)]
pub struct DexProgramInput {
    /// Merkle root of the token-balance state before this batch.
    pub initial_state_root: [u8; 32],
    /// Ordered list of transfers in this batch.
    pub transfers: Vec<DexTransfer>,
}

/// A single token transfer inside a ZK-DEX batch.
#[derive(Serialize, Deserialize, RSerialize, RDeserialize, Archive, Clone, Debug)]
pub struct DexTransfer {
    /// Sender address (20 bytes).
    pub from: [u8; 20],
    /// Recipient address (20 bytes).
    pub to: [u8; 20],
    /// Token contract address (20 bytes).
    pub token: [u8; 20],
    /// Transfer amount (simplified to u64 for the demo).
    pub amount: u64,
    /// Sender's nonce for replay protection.
    pub nonce: u64,
}

/// Output of the ZK-DEX guest program.
///
/// Committed as public values by the zkVM so the L1 verifier can
/// check the state transition.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DexProgramOutput {
    /// State root before execution.
    pub initial_state_root: [u8; 32],
    /// State root after executing all transfers.
    pub final_state_root: [u8; 32],
    /// Number of transfers successfully processed.
    pub transfer_count: u64,
}

impl DexProgramOutput {
    /// Encode the output to bytes for L1 commitment verification.
    ///
    /// Layout: `initial_state_root (32) || final_state_root (32) || transfer_count (8 BE)`
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(72);
        buf.extend_from_slice(&self.initial_state_root);
        buf.extend_from_slice(&self.final_state_root);
        buf.extend_from_slice(&self.transfer_count.to_be_bytes());
        buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify the 72-byte layout: initial_root(32) + final_root(32) + transfer_count(8 BE).
    #[test]
    fn zk_dex_encode_layout() {
        let output = DexProgramOutput {
            initial_state_root: [0xAA; 32],
            final_state_root: [0xBB; 32],
            transfer_count: 42,
        };
        let encoded = output.encode();
        assert_eq!(encoded.len(), 72);
        // Field positions.
        assert_eq!(&encoded[0..32], &[0xAA; 32]);
        assert_eq!(&encoded[32..64], &[0xBB; 32]);
        // transfer_count is big-endian u64.
        assert_eq!(u64::from_be_bytes(encoded[64..72].try_into().unwrap()), 42);
    }

    #[test]
    fn zk_dex_encode_zero_values() {
        let output = DexProgramOutput {
            initial_state_root: [0; 32],
            final_state_root: [0; 32],
            transfer_count: 0,
        };
        let encoded = output.encode();
        assert_eq!(encoded.len(), 72);
        assert!(encoded.iter().all(|&b| b == 0));
    }

    #[test]
    fn zk_dex_encode_max_values() {
        let output = DexProgramOutput {
            initial_state_root: [0xFF; 32],
            final_state_root: [0xFF; 32],
            transfer_count: u64::MAX,
        };
        let encoded = output.encode();
        assert_eq!(encoded.len(), 72);
        assert!(encoded.iter().all(|&b| b == 0xFF));
    }
}
