use ethrex_common::types::BlockHeader;

/// Computes the millisecond-precision timestamp for a BSC block header.
///
/// Post-Lorentz, BSC stores a sub-second offset (0–999 ms) in the
/// `mix_digest` (`prev_randao`) field of the block header:
///
/// ```text
/// milli_timestamp = header.timestamp * 1000 + uint256(header.mix_digest)
/// ```
///
/// The `mix_digest` value is a big-endian 256-bit integer whose last 8 bytes
/// encode the millisecond part. For pre-Lorentz headers the field is zero,
/// so `milli_timestamp` degrades to `timestamp * 1000`.
///
/// Reference: bnb-chain/bsc `consensus/parlia/parlia.go` — `systemTxs` and
/// the Lorentz milli-timestamp encoding.
pub fn milli_timestamp(header: &BlockHeader) -> u64 {
    // Extract the sub-second millisecond offset encoded in the last 8 bytes of
    // mix_digest (prev_randao) as a big-endian u64. The value is expected to
    // be in 0..1000. For pre-Lorentz blocks the field is all-zeros, so ms_part
    // evaluates to 0 and the result degrades to timestamp * 1000.
    let bytes = header.prev_randao.as_bytes();
    // SAFETY: H256 is exactly 32 bytes long; [24..32] is always valid.
    let mut arr = [0u8; 8];
    arr.copy_from_slice(&bytes[24..32]);
    let ms_part = u64::from_be_bytes(arr);

    header
        .timestamp
        .saturating_mul(1000)
        .saturating_add(ms_part)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::H256;

    #[test]
    fn milli_timestamp_zero_mix_digest() {
        // Zero mix_digest: ms_part = 0, result = timestamp * 1000.
        let header = BlockHeader {
            timestamp: 1_000_000,
            prev_randao: H256::zero(),
            ..Default::default()
        };
        assert_eq!(milli_timestamp(&header), 1_000_000_000);
    }

    #[test]
    fn milli_timestamp_with_ms_offset() {
        // Encode ms_part = 750 in last 8 bytes (big-endian).
        let mut bytes = [0u8; 32];
        bytes[24..32].copy_from_slice(&750u64.to_be_bytes());
        let header = BlockHeader {
            timestamp: 1_000_000,
            prev_randao: H256::from_slice(&bytes),
            ..Default::default()
        };
        assert_eq!(milli_timestamp(&header), 1_000_000_750);
    }

    #[test]
    fn milli_timestamp_saturates_on_overflow() {
        // Should not panic — saturating arithmetic.
        let header = BlockHeader {
            timestamp: u64::MAX / 1000 + 1,
            prev_randao: H256::zero(),
            ..Default::default()
        };
        let _ = milli_timestamp(&header);
    }
}
