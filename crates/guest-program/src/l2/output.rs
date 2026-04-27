use ethrex_common::types::balance_diff::BalanceDiff;
use ethrex_common::{H256, U256};
use serde::{Deserialize, Serialize};

/// Output of the L2 stateless validation program.
#[derive(Serialize, Deserialize)]
pub struct ProgramOutput {
    /// Initial state trie root hash.
    pub initial_state_hash: H256,
    /// Final state trie root hash.
    pub final_state_hash: H256,
    /// Merkle root of all L1 output messages in a batch.
    pub l1_out_messages_merkle_root: H256,
    /// Rolling hash of all deposit transactions included in a batch.
    pub l1_in_messages_rolling_hash: H256,
    /// Rolling hash of all L2 in messages included in a batch (per chain ID).
    pub l2_in_message_rolling_hashes: Vec<(u64, H256)>,
    /// Blob commitment versioned hash.
    pub blob_versioned_hash: H256,
    /// Hash of the last block in the batch.
    pub last_block_hash: H256,
    /// Chain ID of the network.
    pub chain_id: U256,
    /// Number of non-privileged transactions in the batch.
    pub non_privileged_count: U256,
    /// Balance diffs for each chain ID.
    pub balance_diffs: Vec<BalanceDiff>,
}

impl ProgramOutput {
    /// Encode the output to bytes for commitment.
    pub fn encode(&self) -> Vec<u8> {
        let mut encoded = [
            self.initial_state_hash.to_fixed_bytes(),
            self.final_state_hash.to_fixed_bytes(),
            self.l1_out_messages_merkle_root.to_fixed_bytes(),
            self.l1_in_messages_rolling_hash.to_fixed_bytes(),
            self.blob_versioned_hash.to_fixed_bytes(),
            self.last_block_hash.to_fixed_bytes(),
            self.chain_id.to_big_endian(),
            self.non_privileged_count.to_big_endian(),
        ]
        .concat();

        for balance_diff in &self.balance_diffs {
            encoded.extend_from_slice(&balance_diff.chain_id.to_big_endian());
            encoded.extend_from_slice(&balance_diff.value.to_big_endian());
            for value_per_token in &balance_diff.value_per_token {
                encoded.extend_from_slice(&value_per_token.token_l1.to_fixed_bytes());
                encoded.extend_from_slice(&value_per_token.token_src_l2.to_fixed_bytes());
                encoded.extend_from_slice(&value_per_token.token_dst_l2.to_fixed_bytes());
                encoded.extend_from_slice(&value_per_token.value.to_big_endian());
            }
            encoded.extend(
                balance_diff
                    .message_hashes
                    .iter()
                    .flat_map(|h| h.to_fixed_bytes()),
            );
        }

        for (chain_id, hash) in &self.l2_in_message_rolling_hashes {
            encoded.extend_from_slice(&chain_id.to_be_bytes());
            encoded.extend_from_slice(&hash.to_fixed_bytes());
        }

        encoded
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::types::balance_diff::BalanceDiff;

    /// Reproducer for an L1↔guest public-input encoding mismatch.
    ///
    /// The L1 verifier in `OnChainProposer.sol::_getPublicInputsFromCommitment`
    /// reconstructs the public inputs and `sha256`-hashes them. For each entry in
    /// `currentBatch.l2InMessageRollingHashes`, the contract appends:
    ///
    /// ```solidity
    /// abi.encodePacked(
    ///     publicInputs,
    ///     bytes32(rh.chainId),   // 32 bytes — chainId is uint256 in the struct
    ///     rh.rollingHash         // 32 bytes
    /// );
    /// ```
    ///
    /// — i.e. **64 bytes per entry**.
    ///
    /// The guest-side `ProgramOutput::encode` (used as the SP1/RISC0 public
    /// commitment via `commit_slice(&output.encode())`) appends:
    ///
    /// ```ignore
    /// for (chain_id, hash) in &self.l2_in_message_rolling_hashes {
    ///     encoded.extend_from_slice(&chain_id.to_be_bytes()); // u64 → 8 bytes
    ///     encoded.extend_from_slice(&hash.to_fixed_bytes());  // 32 bytes
    /// }
    /// ```
    ///
    /// — only **40 bytes per entry**, because `chain_id` is typed as `u64`
    /// (`Vec<(u64, H256)>`) and `u64::to_be_bytes()` returns 8 bytes.
    ///
    /// Consequence: as soon as a batch contains any L2-in privileged
    /// transactions (i.e. any L2-to-L2 messaging is exercised), the prover
    /// commits to a public input that is shorter than the one the L1
    /// reconstructs, the two `sha256` hashes diverge, and the proof fails
    /// verification — bricking L2-to-L2 batches. The `l1_in_messages_rolling_hash`
    /// path doesn't have this issue (it's a single bytes32). The
    /// `BalanceDiff::chain_id` and the top-level `chain_id` fields are fine
    /// because both are `U256` and round-trip through `to_big_endian()` as 32
    /// bytes — so the bug is local to the `(u64, H256)` typing of
    /// `l2_in_message_rolling_hashes`.
    ///
    /// This test pins the current (buggy) byte layout. Two reasonable shapes
    /// for the fix:
    /// 1. Change the field type to `Vec<(U256, H256)>` and use
    ///    `chain_id.to_big_endian()` like the other fields.
    /// 2. Keep the type but pad on encode: write 24 zero bytes followed by
    ///    `chain_id.to_be_bytes()` so the wire format is 32 bytes.
    /// Both make the per-entry contribution 64 bytes, matching L1.
    #[test]
    fn l2_in_message_rolling_hashes_chain_id_is_only_8_bytes_not_32() {
        // Build an output where every other field is empty / zero so we can
        // isolate the `l2_in_message_rolling_hashes` contribution.
        let chain_id: u64 = 0x1234_5678_9abc_def0;
        let rolling_hash = H256([0xCC; 32]);

        let output = ProgramOutput {
            initial_state_hash: H256::zero(),
            final_state_hash: H256::zero(),
            l1_out_messages_merkle_root: H256::zero(),
            l1_in_messages_rolling_hash: H256::zero(),
            l2_in_message_rolling_hashes: vec![(chain_id, rolling_hash)],
            blob_versioned_hash: H256::zero(),
            last_block_hash: H256::zero(),
            chain_id: U256::zero(),
            non_privileged_count: U256::zero(),
            balance_diffs: Vec::<BalanceDiff>::new(),
        };

        let encoded = output.encode();

        // Fixed prefix: 8 H256/U256 fields × 32 bytes = 256 bytes.
        const PREFIX_LEN: usize = 32 * 8;

        // Current (buggy) per-entry size: 8 (u64) + 32 (H256) = 40 bytes.
        // L1's expected per-entry size: 32 (bytes32 chainId) + 32 (H256) = 64 bytes.
        assert_eq!(
            encoded.len(),
            PREFIX_LEN + 40,
            "Guest commits 40 bytes per (chainId, rollingHash) entry; L1 reads 64. \
             If this assertion now fails because encoded.len() == PREFIX_LEN + 64, \
             the chain_id encoding was widened to bytes32 — update the assertion."
        );

        // Verify the chain_id bytes sit at the expected offset and are exactly
        // u64::to_be_bytes (8 bytes, no padding).
        let chain_id_bytes = &encoded[PREFIX_LEN..PREFIX_LEN + 8];
        assert_eq!(
            chain_id_bytes,
            &chain_id.to_be_bytes(),
            "chain_id should be the bare 8-byte big-endian u64 — confirming the bug",
        );

        // After the chain_id (offset PREFIX_LEN+8), the rolling hash starts
        // immediately. L1 expects it at PREFIX_LEN+32.
        let hash_bytes = &encoded[PREFIX_LEN + 8..PREFIX_LEN + 8 + 32];
        assert_eq!(
            hash_bytes, &rolling_hash.0,
            "rollingHash starts right after the 8-byte chain_id, with no left-pad",
        );

        // Build what L1's `abi.encodePacked(bytes32(chainId), rollingHash)` would
        // produce so the diff is explicit in the test output.
        let mut expected_l1_per_entry = [0u8; 64];
        // bytes32(uint256(chainId)): left-pad u64 to 32 bytes.
        expected_l1_per_entry[24..32].copy_from_slice(&chain_id.to_be_bytes());
        expected_l1_per_entry[32..64].copy_from_slice(&rolling_hash.0);

        let guest_per_entry = &encoded[PREFIX_LEN..];
        assert_ne!(
            guest_per_entry,
            expected_l1_per_entry.as_slice(),
            "Guest and L1 disagree on the per-entry layout — this is the bug. \
             After the fix, this assertion will flip to assert_eq!.",
        );
    }
}
