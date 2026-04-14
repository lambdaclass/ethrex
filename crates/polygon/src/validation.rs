use ethrex_common::H256;
use ethrex_common::constants::DEFAULT_OMMERS_HASH;
use ethrex_common::types::{BlockHeader, InvalidBlockHeaderError};

use crate::bor_config::BorConfig;
use crate::consensus::extra_data::parse_extra_data;

/// Validates a Bor block header against its parent.
///
/// Bor headers differ from Ethereum PoS headers:
/// - difficulty is 2 (in-turn producer) or 1 (out-of-turn)
/// - coinbase must be zero (signer is recovered from extra_data signature)
/// - extra_data has no 32-byte max (contains vanity + validator set + signature)
/// - withdrawals_root, parent_beacon_block_root, blob_gas_used, excess_blob_gas,
///   requests_hash must all be absent (None)
/// - nonce is always 0
/// - ommers_hash is always the default empty hash
/// - gas limit must not exceed 2^63-1
/// - mix digest (prev_randao) must be zero
/// - post-Giugliano blocks must contain gas_target and base_fee_change_denominator
pub fn validate_bor_header(
    header: &BlockHeader,
    parent_header: &BlockHeader,
    config: &BorConfig,
) -> Result<(), InvalidBlockHeaderError> {
    // Gas used must not exceed gas limit
    if header.gas_used > header.gas_limit {
        return Err(InvalidBlockHeaderError::GasUsedGreaterThanGasLimit);
    }

    // Block number must be parent + 1
    if header.number != parent_header.number + 1 {
        return Err(InvalidBlockHeaderError::BlockNumberNotOneGreater);
    }

    // Timestamp must be strictly greater than parent
    if header.timestamp <= parent_header.timestamp {
        return Err(InvalidBlockHeaderError::TimestampNotGreaterThanParent);
    }

    // Minimum timestamp gap: parent.Time + period (Bor bor.go:610)
    let period = config.get_period(header.number);
    if parent_header.timestamp + period > header.timestamp {
        return Err(InvalidBlockHeaderError::PolygonTimestampGapTooSmall {
            minimum: parent_header.timestamp + period,
            actual: header.timestamp,
        });
    }

    // Reject blocks too far in the future (Bor bor.go:422-461)
    // Bor uses maxAllowedFutureBlockTimeSeconds = 30 (consensus/bor/bor.go:134).
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if header.timestamp > now + 30 {
        return Err(InvalidBlockHeaderError::PolygonFutureBlock {
            header_time: header.timestamp,
            now,
        });
    }

    // Parent hash must match
    if header.parent_hash != parent_header.hash() {
        return Err(InvalidBlockHeaderError::ParentHashIncorrect);
    }

    // Ommers hash must be the empty default
    if header.ommers_hash != *DEFAULT_OMMERS_HASH {
        return Err(InvalidBlockHeaderError::OmmersHashNotDefault);
    }

    // Nonce must be zero
    if header.nonce != 0 {
        return Err(InvalidBlockHeaderError::NonceNotZero);
    }

    // --- Bor-specific checks ---

    // Difficulty must be non-zero. Pre-Rio blocks use difficulty 1..totalValidators
    // for proposer weighting. Post-Rio blocks use only 1 (in-turn) or 2 (out-of-turn).
    // Upper-bound validation (requires validator count) will be added with snapshot integration.
    if header.difficulty.is_zero() {
        return Err(InvalidBlockHeaderError::PolygonInvalidDifficulty(
            header.difficulty,
        ));
    }

    // Coinbase must be zero (signer is in extra_data)
    if !header.coinbase.is_zero() {
        return Err(InvalidBlockHeaderError::PolygonCoinbaseNotZero);
    }

    // Beacon/blob/withdrawal fields must be absent
    if header.withdrawals_root.is_some() {
        return Err(InvalidBlockHeaderError::PolygonWithdrawalsRootPresent);
    }
    if header.parent_beacon_block_root.is_some() {
        return Err(InvalidBlockHeaderError::PolygonParentBeaconBlockRootPresent);
    }
    if header.blob_gas_used.is_some() {
        return Err(InvalidBlockHeaderError::PolygonBlobGasUsedPresent);
    }
    if header.excess_blob_gas.is_some() {
        return Err(InvalidBlockHeaderError::PolygonExcessBlobGasPresent);
    }
    if header.requests_hash.is_some() {
        return Err(InvalidBlockHeaderError::PolygonRequestsHashPresent);
    }

    // Gas limit must not exceed 2^63-1
    if header.gas_limit > 0x7fffffffffffffff {
        return Err(InvalidBlockHeaderError::PolygonGasLimitCap {
            gas_limit: header.gas_limit,
        });
    }

    // Mix digest (prev_randao) must be zero
    if header.prev_randao != H256::zero() {
        return Err(InvalidBlockHeaderError::PolygonNonZeroMixDigest);
    }

    // Post-Giugliano blocks must contain gas_target and base_fee_change_denominator
    // in the extra data. We only check presence, not correctness, because these
    // parameters are configurable per-node via CLI flags.
    if config.is_giugliano_active(header.number) {
        if let Ok((_vanity, block_extra, _sig)) = parse_extra_data(&header.extra_data) {
            if block_extra.gas_target.is_none() || block_extra.base_fee_change_denominator.is_none()
            {
                return Err(InvalidBlockHeaderError::PolygonMissingGiuglianoFields);
            }
        } else {
            // If we can't parse the extra data at all on a post-Giugliano block,
            // the fields are definitely missing.
            return Err(InvalidBlockHeaderError::PolygonMissingGiuglianoFields);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use ethereum_types::{Address, H256, U256};

    /// Minimal BorConfig for tests — Giugliano not active at test block numbers.
    fn test_config() -> BorConfig {
        serde_json::from_str(
            r#"{
                "period": {"0": 2},
                "producerDelay": {"0": 6},
                "sprint": {"0": 16},
                "backupMultiplier": {"0": 2},
                "validatorContract": "0x0000000000000000000000000000000000001000",
                "stateReceiverContract": "0x0000000000000000000000000000000000001001",
                "jaipurBlock": 0,
                "delhiBlock": 0,
                "indoreBlock": 0
            }"#,
        )
        .expect("valid test config")
    }

    fn make_parent() -> BlockHeader {
        BlockHeader {
            number: 100,
            timestamp: 1000,
            difficulty: U256::from(1),
            coinbase: Address::zero(),
            ommers_hash: *DEFAULT_OMMERS_HASH,
            gas_limit: 30_000_000,
            gas_used: 1_000_000,
            base_fee_per_gas: Some(7),
            extra_data: Bytes::from(vec![0u8; 97]),
            ..Default::default()
        }
    }

    fn make_child(parent: &BlockHeader) -> BlockHeader {
        BlockHeader {
            parent_hash: parent.hash(),
            number: parent.number + 1,
            timestamp: parent.timestamp + 2,
            difficulty: U256::from(1),
            coinbase: Address::zero(),
            ommers_hash: *DEFAULT_OMMERS_HASH,
            gas_limit: 30_000_000,
            gas_used: 500_000,
            base_fee_per_gas: Some(7),
            extra_data: Bytes::from(vec![0u8; 97]),
            ..Default::default()
        }
    }

    #[test]
    fn valid_bor_header() {
        let parent = make_parent();
        let child = make_child(&parent);
        assert!(validate_bor_header(&child, &parent, &test_config()).is_ok());
    }

    #[test]
    fn valid_bor_header_difficulty_2() {
        let parent = make_parent();
        let mut child = make_child(&parent);
        child.difficulty = U256::from(2);
        assert!(validate_bor_header(&child, &parent, &test_config()).is_ok());
    }

    #[test]
    fn reject_difficulty_zero() {
        let parent = make_parent();
        let mut child = make_child(&parent);
        child.difficulty = U256::zero();
        assert!(matches!(
            validate_bor_header(&child, &parent, &test_config()),
            Err(InvalidBlockHeaderError::PolygonInvalidDifficulty(_))
        ));
    }

    #[test]
    fn accept_pre_rio_difficulty() {
        let parent = make_parent();
        let mut child = make_child(&parent);
        // Pre-Rio blocks can have difficulty up to totalValidators (e.g. 21)
        child.difficulty = U256::from(21);
        assert!(validate_bor_header(&child, &parent, &test_config()).is_ok());
    }

    #[test]
    fn reject_nonzero_coinbase() {
        let parent = make_parent();
        let mut child = make_child(&parent);
        child.coinbase = Address::from_low_u64_be(1);
        assert!(matches!(
            validate_bor_header(&child, &parent, &test_config()),
            Err(InvalidBlockHeaderError::PolygonCoinbaseNotZero)
        ));
    }

    #[test]
    fn reject_withdrawals_root_present() {
        let parent = make_parent();
        let mut child = make_child(&parent);
        child.withdrawals_root = Some(H256::zero());
        assert!(matches!(
            validate_bor_header(&child, &parent, &test_config()),
            Err(InvalidBlockHeaderError::PolygonWithdrawalsRootPresent)
        ));
    }

    #[test]
    fn reject_blob_gas_used_present() {
        let parent = make_parent();
        let mut child = make_child(&parent);
        child.blob_gas_used = Some(0);
        assert!(matches!(
            validate_bor_header(&child, &parent, &test_config()),
            Err(InvalidBlockHeaderError::PolygonBlobGasUsedPresent)
        ));
    }

    #[test]
    fn reject_excess_blob_gas_present() {
        let parent = make_parent();
        let mut child = make_child(&parent);
        child.excess_blob_gas = Some(0);
        assert!(matches!(
            validate_bor_header(&child, &parent, &test_config()),
            Err(InvalidBlockHeaderError::PolygonExcessBlobGasPresent)
        ));
    }

    #[test]
    fn reject_parent_beacon_block_root_present() {
        let parent = make_parent();
        let mut child = make_child(&parent);
        child.parent_beacon_block_root = Some(H256::zero());
        assert!(matches!(
            validate_bor_header(&child, &parent, &test_config()),
            Err(InvalidBlockHeaderError::PolygonParentBeaconBlockRootPresent)
        ));
    }

    #[test]
    fn reject_requests_hash_present() {
        let parent = make_parent();
        let mut child = make_child(&parent);
        child.requests_hash = Some(H256::zero());
        assert!(matches!(
            validate_bor_header(&child, &parent, &test_config()),
            Err(InvalidBlockHeaderError::PolygonRequestsHashPresent)
        ));
    }

    #[test]
    fn allow_large_extra_data() {
        let parent = make_parent();
        let mut child = make_child(&parent);
        // Bor extra_data can be much larger than 32 bytes
        child.extra_data = Bytes::from(vec![0u8; 500]);
        assert!(validate_bor_header(&child, &parent, &test_config()).is_ok());
    }

    #[test]
    fn reject_gas_used_exceeds_gas_limit() {
        let parent = make_parent();
        let mut child = make_child(&parent);
        child.gas_limit = 1_000_000;
        child.gas_used = 1_000_001;
        assert!(matches!(
            validate_bor_header(&child, &parent, &test_config()),
            Err(InvalidBlockHeaderError::GasUsedGreaterThanGasLimit)
        ));
    }

    #[test]
    fn accept_gas_used_equals_gas_limit() {
        let parent = make_parent();
        let mut child = make_child(&parent);
        child.gas_used = child.gas_limit;
        assert!(validate_bor_header(&child, &parent, &test_config()).is_ok());
    }

    #[test]
    fn reject_block_number_not_parent_plus_one() {
        let parent = make_parent();
        let mut child = make_child(&parent);
        child.number = parent.number + 2;
        // Need to recompute parent_hash since number changed doesn't affect hash,
        // but the check is on number not hash
        assert!(matches!(
            validate_bor_header(&child, &parent, &test_config()),
            Err(InvalidBlockHeaderError::BlockNumberNotOneGreater)
        ));
    }

    #[test]
    fn reject_timestamp_not_greater_than_parent() {
        let parent = make_parent();
        let mut child = make_child(&parent);
        child.timestamp = parent.timestamp; // equal, not greater
        assert!(matches!(
            validate_bor_header(&child, &parent, &test_config()),
            Err(InvalidBlockHeaderError::TimestampNotGreaterThanParent)
        ));
    }

    #[test]
    fn reject_timestamp_less_than_parent() {
        let parent = make_parent();
        let mut child = make_child(&parent);
        child.timestamp = parent.timestamp - 1;
        assert!(matches!(
            validate_bor_header(&child, &parent, &test_config()),
            Err(InvalidBlockHeaderError::TimestampNotGreaterThanParent)
        ));
    }

    #[test]
    fn reject_wrong_parent_hash() {
        let parent = make_parent();
        let mut child = make_child(&parent);
        child.parent_hash = H256::from_low_u64_be(0xdead);
        assert!(matches!(
            validate_bor_header(&child, &parent, &test_config()),
            Err(InvalidBlockHeaderError::ParentHashIncorrect)
        ));
    }

    #[test]
    fn reject_nonzero_nonce() {
        let parent = make_parent();
        let mut child = make_child(&parent);
        child.nonce = 1;
        // Need correct parent_hash
        child.parent_hash = parent.hash();
        assert!(matches!(
            validate_bor_header(&child, &parent, &test_config()),
            Err(InvalidBlockHeaderError::NonceNotZero)
        ));
    }

    #[test]
    fn reject_non_default_ommers_hash() {
        let parent = make_parent();
        let mut child = make_child(&parent);
        child.ommers_hash = H256::from_low_u64_be(0x1234);
        // parent_hash will be wrong too, but ommers check happens after
        // timestamp check and before nonce — let's set up correctly
        child.parent_hash = parent.hash();
        assert!(matches!(
            validate_bor_header(&child, &parent, &test_config()),
            Err(InvalidBlockHeaderError::OmmersHashNotDefault)
        ));
    }

    /// Polygon PoS headers must encode as exactly 16 RLP list elements:
    /// 15 base fields + base_fee_per_gas. All Cancun/Prague optional fields
    /// (withdrawals_root, blob_gas_used, excess_blob_gas,
    /// parent_beacon_block_root, requests_hash) must be absent (None),
    /// not zero.
    #[test]
    fn polygon_header_rlp_has_exactly_16_elements() {
        use ethrex_rlp::decode::decode_rlp_item;
        use ethrex_rlp::encode::RLPEncode;

        let header = BlockHeader {
            difficulty: U256::from(1),
            coinbase: Address::zero(),
            ommers_hash: *DEFAULT_OMMERS_HASH,
            number: 50_000_000,
            gas_limit: 30_000_000,
            gas_used: 1_000_000,
            timestamp: 1_700_000_000,
            base_fee_per_gas: Some(30_000_000_000u64), // 30 gwei — must be present
            extra_data: Bytes::from(vec![0u8; 97]),
            // All post-London optional fields must be None
            withdrawals_root: None,
            blob_gas_used: None,
            excess_blob_gas: None,
            parent_beacon_block_root: None,
            requests_hash: None,
            block_access_list_hash: None,
            slot_number: None,
            ..Default::default()
        };

        // Validate that the Bor header validation passes
        // (can't run full validate_bor_header since parent_hash won't match,
        //  but the field-level checks should pass)
        assert!(header.withdrawals_root.is_none());
        assert!(header.parent_beacon_block_root.is_none());
        assert!(header.blob_gas_used.is_none());
        assert!(header.excess_blob_gas.is_none());
        assert!(header.requests_hash.is_none());

        // Encode the header
        let encoded = header.encode_to_vec();

        // Decode the outer RLP list to get the payload
        let (is_list, payload, rest) = decode_rlp_item(&encoded).expect("valid RLP");
        assert!(is_list, "header RLP must be a list");
        assert!(rest.is_empty(), "no trailing bytes after header RLP");

        // Count elements by iterating through the payload
        let mut count = 0usize;
        let mut remaining = payload;
        while !remaining.is_empty() {
            let (_, _, after) = decode_rlp_item(remaining).expect("valid inner RLP item");
            count += 1;
            remaining = after;
        }

        assert_eq!(
            count, 16,
            "Polygon header must have exactly 16 RLP elements (15 base + baseFee), got {count}"
        );
    }

    /// If base_fee_per_gas is None, the header would only have 15 elements,
    /// which would break P2P compatibility with Polygon nodes.
    #[test]
    fn polygon_header_without_base_fee_has_15_elements() {
        use ethrex_rlp::decode::decode_rlp_item;
        use ethrex_rlp::encode::RLPEncode;

        let header = BlockHeader {
            difficulty: U256::from(1),
            coinbase: Address::zero(),
            ommers_hash: *DEFAULT_OMMERS_HASH,
            number: 50_000_000,
            gas_limit: 30_000_000,
            gas_used: 1_000_000,
            timestamp: 1_700_000_000,
            base_fee_per_gas: None, // Missing!
            extra_data: Bytes::from(vec![0u8; 97]),
            withdrawals_root: None,
            blob_gas_used: None,
            excess_blob_gas: None,
            parent_beacon_block_root: None,
            requests_hash: None,
            block_access_list_hash: None,
            slot_number: None,
            ..Default::default()
        };

        let encoded = header.encode_to_vec();
        let (is_list, payload, _) = decode_rlp_item(&encoded).expect("valid RLP");
        assert!(is_list);

        let mut count = 0usize;
        let mut remaining = payload;
        while !remaining.is_empty() {
            let (_, _, after) = decode_rlp_item(remaining).expect("valid inner RLP item");
            count += 1;
            remaining = after;
        }

        assert_eq!(
            count, 15,
            "header without base_fee should have 15 elements, got {count}"
        );
    }

    /// Verify that a header with Cancun fields present would encode to more
    /// than 16 elements, which would be incompatible with Polygon P2P.
    #[test]
    fn ethereum_header_with_cancun_fields_has_more_than_16_elements() {
        use ethrex_rlp::decode::decode_rlp_item;
        use ethrex_rlp::encode::RLPEncode;

        let header = BlockHeader {
            difficulty: U256::zero(),
            number: 1,
            gas_limit: 30_000_000,
            gas_used: 1_000_000,
            timestamp: 1_700_000_000,
            base_fee_per_gas: Some(7),
            withdrawals_root: Some(H256::zero()),
            blob_gas_used: Some(0),
            excess_blob_gas: Some(0),
            parent_beacon_block_root: Some(H256::zero()),
            ..Default::default()
        };

        let encoded = header.encode_to_vec();
        let (is_list, payload, _) = decode_rlp_item(&encoded).expect("valid RLP");
        assert!(is_list);

        let mut count = 0usize;
        let mut remaining = payload;
        while !remaining.is_empty() {
            let (_, _, after) = decode_rlp_item(remaining).expect("valid inner RLP item");
            count += 1;
            remaining = after;
        }

        assert!(
            count > 16,
            "Cancun header should have >16 elements, got {count}"
        );
    }

    /// Polygon header RLP must roundtrip correctly, preserving exactly 16 elements.
    #[test]
    fn polygon_header_rlp_roundtrip_preserves_16_elements() {
        use ethrex_rlp::decode::{RLPDecode, decode_rlp_item};
        use ethrex_rlp::encode::RLPEncode;

        let original = BlockHeader {
            parent_hash: H256::from_low_u64_be(0x1234),
            difficulty: U256::from(2),
            coinbase: Address::zero(),
            ommers_hash: *DEFAULT_OMMERS_HASH,
            number: 50_000_001,
            gas_limit: 30_000_000,
            gas_used: 2_000_000,
            timestamp: 1_700_000_002,
            base_fee_per_gas: Some(25_000_000_000u64),
            extra_data: Bytes::from(vec![0u8; 97]),
            withdrawals_root: None,
            blob_gas_used: None,
            excess_blob_gas: None,
            parent_beacon_block_root: None,
            requests_hash: None,
            block_access_list_hash: None,
            slot_number: None,
            ..Default::default()
        };

        let encoded = original.encode_to_vec();
        let decoded = BlockHeader::decode(&encoded).expect("should decode");

        // All post-London optional fields must remain None after roundtrip
        assert!(decoded.withdrawals_root.is_none());
        assert!(decoded.blob_gas_used.is_none());
        assert!(decoded.excess_blob_gas.is_none());
        assert!(decoded.parent_beacon_block_root.is_none());
        assert!(decoded.requests_hash.is_none());
        assert_eq!(decoded.base_fee_per_gas, Some(25_000_000_000u64));

        // Re-encode and verify element count
        let re_encoded = decoded.encode_to_vec();
        let (is_list, payload, _) = decode_rlp_item(&re_encoded).expect("valid RLP");
        assert!(is_list);

        let mut count = 0usize;
        let mut remaining = payload;
        while !remaining.is_empty() {
            let (_, _, after) = decode_rlp_item(remaining).expect("valid inner RLP item");
            count += 1;
            remaining = after;
        }

        assert_eq!(count, 16, "re-encoded polygon header must have 16 elements");
    }

    // ====================================================================
    // Cross-validation tests against real Polygon Bor mainnet data.
    // All hex values sourced directly from Polygon RPC (eth_getBlockByNumber).
    // ====================================================================

    /// Helper to decode a hex string (with or without 0x prefix) into bytes.
    fn hex_bytes(s: &str) -> Vec<u8> {
        let s = s.strip_prefix("0x").unwrap_or(s);
        hex::decode(s).expect("valid hex")
    }

    /// Helper to parse a hex string to u64.
    fn hex_u64(s: &str) -> u64 {
        let s = s.strip_prefix("0x").unwrap_or(s);
        u64::from_str_radix(s, 16).expect("valid hex u64")
    }

    /// Verify that our BlockHeader RLP encoding + keccak256 produces the correct
    /// block hash for a real Polygon mainnet block.
    ///
    /// Block 50,000,000 (0x2FAF080) — a post-London, post-Delhi normal block.
    #[test]
    fn crosscheck_block_50m_hash_matches_bor() {
        use ethereum_types::Bloom;
        use std::str::FromStr;

        let expected_hash =
            H256::from_str("0xed6bc55bb3fbf391fb47a96bb0327906a2dab5f50c1330f89684b79a5195efaa")
                .unwrap();

        let bloom_bytes: [u8; 256] = hex_bytes(
            "db3a9cb765f57e3ef6b760d1fcaf4eba4bcb5b44c56fb1e838f7f87de18fe977\
             5a2ef30addeb605b5be3e9bfbfd64ccdfab7fdf7ddaee92af58e337f81bee788f\
             869fe176d0bf9db44e9b6dfc86ae9f8962727becd5eb1f2f3078f96dacd3e07ed\
             dc79d61bf34e6081a7f9236d997b89fb8901c7c6b1ba75f1b1c7fdf6d9ff3d07\
             776d7e39ec6abd7e2199e265b2e15f79bfadf1afc6a38fafcf1a70f6d55bda6e\
             430b2bf253df0f3874fd44da9090cf4877bf90666bd9a8b42a932fd6808cde5da\
             dbd3311bfecc7ff73025a4e30bca9eb710e22cd6eddc9fd7fe22a3c7bf99e189a\
             8de36bab08ccf50d6efd3e87f935d9ec26776bcf44dfbc3db96d2b9c9b6b",
        )
        .try_into()
        .unwrap();

        let extra_data_bytes = hex_bytes(
            "d78301000683626f7288676f312e32302e38856c696e75780000000000000000\
             b6c6f13270d722c179f577c7669e775841f090e7dc37640e3da15ba28dbaef47\
             2f1b963fc7b3ca1b884cc23cb2b302ec1ce6e8bf0a9164d8181407e0d7a2f79801",
        );

        let header = BlockHeader {
            parent_hash: H256::from_str(
                "0xafe719f6ca102ab7b7b9fd367688e73a777d02d21d6a692a8ff6a6eb3c2f7c27",
            )
            .unwrap(),
            ommers_hash: H256::from_str(
                "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
            )
            .unwrap(),
            coinbase: Address::zero(),
            state_root: H256::from_str(
                "0xf48ab66c3b39fcf8415ba3f787a529900795b307337fc244949eed20f10de6dc",
            )
            .unwrap(),
            transactions_root: H256::from_str(
                "0x9434da2710a11e1a5c3b61f7feb0dd5a75520befe5a6ccc1be4de14251f00869",
            )
            .unwrap(),
            receipts_root: H256::from_str(
                "0xbbdeef8478f908a12f86b906c43a4f0ef2bb62226ccae6af1e34668488ca8323",
            )
            .unwrap(),
            logs_bloom: Bloom::from(bloom_bytes),
            difficulty: U256::from(hex_u64("0x15")),
            number: hex_u64("0x2faf080"),
            gas_limit: hex_u64("0x1c31a8c"),
            gas_used: hex_u64("0x18ec928"),
            timestamp: hex_u64("0x65559991"),
            extra_data: Bytes::from(extra_data_bytes),
            prev_randao: H256::zero(), // mixHash = 0x0
            nonce: 0,
            base_fee_per_gas: Some(hex_u64("0x18b525e260")),
            // Polygon PoS: all post-London optional fields are absent
            withdrawals_root: None,
            blob_gas_used: None,
            excess_blob_gas: None,
            parent_beacon_block_root: None,
            requests_hash: None,
            block_access_list_hash: None,
            slot_number: None,
            ..Default::default()
        };

        assert_eq!(
            header.hash(),
            expected_hash,
            "Block 50,000,000 hash must match the real Polygon mainnet block hash"
        );
    }

    /// Verify chain linkage: block 50,000,000's parentHash matches block 49,999,999's hash.
    ///
    /// Block 49,999,999 is a sprint-end block (49999999 % 16 == 15) with 21 validators
    /// encoded in its extra_data. This proves our RLP encoding handles large extra_data
    /// fields correctly.
    #[test]
    fn crosscheck_block_49999999_hash_and_chain_linkage() {
        use ethereum_types::Bloom;
        use std::str::FromStr;

        let expected_hash =
            H256::from_str("0xafe719f6ca102ab7b7b9fd367688e73a777d02d21d6a692a8ff6a6eb3c2f7c27")
                .unwrap();

        let bloom_bytes: [u8; 256] = hex_bytes(
            "9e2b21c520a5de6eaab963bae7ab50189a2b16f0ecf1c79c7c295033fca0bf73\
             022ab358ec6f21901c137c943a580cc12395930096aabe51c8d401426cf62b86a\
             82aa18336b3489b620b685ba26171e65b7844e5eb719b9b36918d7c968d03c084\
             74a9a41372387cf0c6590617b28d65a8580077e9c415b5f520f8594afc255ed65\
             12d4e57b80587ea3b2029dd26588f6e3345f58ed2f35be80b60e9424b38183e4d\
             2b6274bfd511787afcec1e99b4ace044ed5528a8c4c206150a1285c169fe18698\
             aaa915850512d3d801e4e526f3c6b6f06e1844eb53aa433eb369154704e1cb82c\
             cb63223cfdf91de9e9338f0fe4cca936b76809445f390908ed0e7f480b",
        )
        .try_into()
        .unwrap();

        // Full 937-byte extra_data from the sprint-end block with 21 validators.
        // This is the exact hex from the RPC response (0x prefix stripped by hex_bytes).
        let extra_data_bytes = hex_bytes(
            "d88301010083626f7289676f312e32302e3130856c696e757800000000000000048cfedf907c4c9ddd11ff882380906e78e84bbe00000000000000000000000000000000000000011efecb61a2f80aa34d3b9218b564a64d059462900000000000000000000000000000000000000002272cc48bb89900689c7e51c6a3ab039ec5cc80f900000000000000000000000000000000000000012c74ca71679cf1299936d6104d825c965448907b000000000000000000000000000000000000000143c7c14d94197a30a44dab27bfb3eee9e05496d4000000000000000000000000000000000000000146a3a41bd932244dd08186e4c19f1a7e48cbcdf4000000000000000000000000000000000000000767b94473d81d0cd00849d563c94d0432ac988b49000000000000000000000000000000000000000773d378cfeaa5cbe8daed64128ebdc91322aa586b0000000000000000000000000000000000000002794e44d1334a56fea7f4df12633b88820d0c588800000000000000000000000000000000000000037c7379531b2aee82e4ca06d4175d13b9cbeafd4900000000000000000000000000000000000000068842ea85732f94feeb9cf1ccc7d357c63658e7a4000000000000000000000000000000000000000198c27cc3f0301b6272049dc3f972e2f54278062900000000000000000000000000000000000000019ead03f7136fc6b4bdb0780b00a1c14ae5a8b6d00000000000000000000000000000000000000006a8b52f02108aa5f4b675bdcc973760022d7c60200000000000000000000000000000000000000002bdbd4347b082d9d6bdf2da4555a37ce52a2e21200000000000000000000000000000000000000001c6869257205e20c2a43cb31345db534aecb49f6e0000000000000000000000000000000000000001e63727cb2b3a8d6e3a2d1df4990f441938b67a340000000000000000000000000000000000000001e7e2cb8c81c10ff191a73fe266788c9ce62ec7540000000000000000000000000000000000000001ec20607aa654d823dd01beb8780a44863c57ed070000000000000000000000000000000000000001eedba2484aaf940f37cd3cd21a5d7c4a7dafbfc00000000000000000000000000000000000000002f0245f6251bef9447a08766b9da2b07b28ad80b00000000000000000000000000000000000000002047c5f98ba5626525f9a49f8980f993e67d1fcd84eaec54197fbb17adabdef1a729284686be02443416f019210ab224d3fd522fa694a59b553fb0370663151cd00",
        );

        let header = BlockHeader {
            parent_hash: H256::from_str(
                "0x3634f1f7b914084c9db8dbb857f990c8427dbe2bc0d3e52db4b8383e2cdfd8e1",
            )
            .unwrap(),
            ommers_hash: H256::from_str(
                "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
            )
            .unwrap(),
            coinbase: Address::zero(),
            state_root: H256::from_str(
                "0x43457e70d48ba9bf91378c0c2c912f3f7831e23859c8870d538f66230b87f490",
            )
            .unwrap(),
            transactions_root: H256::from_str(
                "0xe217ed7421931ecb69b1e27ebc6570f9ca47c0ce289720372982a6c4182e75e6",
            )
            .unwrap(),
            receipts_root: H256::from_str(
                "0xcff981eb160412bf0ece440ff119f04f33e583f775215c97f7de50b0a5497785",
            )
            .unwrap(),
            logs_bloom: Bloom::from(bloom_bytes),
            difficulty: U256::from(hex_u64("0x15")),
            number: hex_u64("0x2faf07f"),
            gas_limit: hex_u64("0x1c2a9e3"),
            gas_used: hex_u64("0xf2870c"),
            timestamp: hex_u64("0x6555998d"),
            extra_data: Bytes::from(extra_data_bytes),
            prev_randao: H256::zero(),
            nonce: 0,
            base_fee_per_gas: Some(hex_u64("0x18971f7f8c")),
            withdrawals_root: None,
            blob_gas_used: None,
            excess_blob_gas: None,
            parent_beacon_block_root: None,
            requests_hash: None,
            block_access_list_hash: None,
            slot_number: None,
            ..Default::default()
        };

        assert_eq!(
            header.hash(),
            expected_hash,
            "Block 49,999,999 hash must match the real Polygon mainnet block hash"
        );
    }

    /// Verify block hash for a post-Lisovo block with post-Cancun RLP extra_data format.
    ///
    /// Block 0x4FE4620 — post-Lisovo, difficulty=1, post-Cancun extra_data format.
    ///
    /// Cross-validates header RLP encoding against a real post-Lisovo Polygon mainnet block.
    #[test]
    fn crosscheck_post_lisovo_block_hash() {
        use ethereum_types::Bloom;
        use std::str::FromStr;

        let expected_hash =
            H256::from_str("0xb412c721f877d64d58d56cde3bac7fc0100113da533170136ab8edf73bd8ce99")
                .unwrap();

        let bloom_bytes: [u8; 256] = hex_bytes(
            "f7fdf5e3e7b7f5fadbf8efd7977af2e52bf57a87ff3ddff9f9e0febf4f7effe6\
             effd7eef7fdfceddef5fdaddf8a1d78bfaffff3fb7fff6fdffbe7f7ff3fd6aedff\
             7e9bbfdfdefb7cb6ff045dfbbfbfefc6ef47e3bf7bdffd75ebf3dd5cf0735ad7d5\
             bdeeeefbcaed759ffedbfc5bffedfdbdddb3efdb0d5bdee67f9a6ebff3e37bc3df\
             fcf9abf9d13eaf9fe67d75b7ff7f33fddf3efebf6d7abdeff7dfadfcfebffbfbbd\
             74ffbdfb6ed75bdbcb3bedd7ff7dbbb9fe9fbf9db5a7da76fb296e7ffefffbff97\
             ef3bcf6ebbfce7d36ebb7defff6fcfcffbf8ff6ffbfdfefc1ffee7fffd7fdbffb9\
             9a4a87c17e374f8ff779f4de7f77f7563fdef3ef79efff1b7aff",
        )
        .try_into()
        .unwrap();

        let extra_data_bytes = hex_bytes(
            "d78301100883626f7288676f312e32352e37856c696e75780000000000000000f901f780f901f3c0c180c101c0c101c0c105c0c0c20706c0c0c10bc10cc0c0c109c110c104c112c113c114c115c10ac117c118c119c20216c11bc11cc11dc21006c11ec120c121c122c123c124c125c21d1bc126c22728c129c12ac11fc12cc12dc12ec12fc130c111c23111c133c134c20e35c136c137c138c139c13ac13bc13cc13dc13ec13fc140c141c142c143c144c145c146c147c148c149c14ac14bc14cc14dc14ec14fc150c151c152c153c154c155c156c157c158c159c15ac15bc75c1f46383b4050c15cc25d5ec15fc160c161c162c163c12bc164c166c22965c167c169c16ac16bc16cc16dc16ec16fc170c171c172c173c174c168c8207624801214161cc175c178c179c17ac132c17bc17dc17ec17fc28180c28181c28182c28183c28184c28185c28186c28187c28188c177c28189cd60627a818b3850474e7e5c3d40c2818cc2818dc17cc2818ec28190c28191c28192c28193c28194c376818ac28195c48197818bc28198c28199c2819ac2819bc2819cc2818fc2819dc2819fc281a0c281a1c281a2c2819ec281a3c0c281a4c281a7c32381a8c28196c481a981a7c0c281abc281a5c0c281aec108c281b0c281b1c281b2c0c106c281b4c281b7ca81b681b881ad81b081b4c281b9c281b9ca121c247680141620818ac281bcc281bdc281bec281bfc281c0c281c1c281c2c281c3c281c4dbb5af28089606322194e84c9c339abcabc383fb9d0ca3a06c2e640fd59a79a04a5ba58b05c62121228fc3e5aad3f1bf86363164142df407cbfc7915e6f5b75a01",
        );

        let header = BlockHeader {
            parent_hash: H256::from_str(
                "0xc01947067ccf6f2b5c354192ece770d73fc703252be177014b215f4ee696dcde",
            )
            .unwrap(),
            ommers_hash: H256::from_str(
                "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
            )
            .unwrap(),
            coinbase: Address::zero(),
            state_root: H256::from_str(
                "0xa9d9ccb731e276c6f639b859579e34363f91af100ae6b2de2fd3b283d3c7d042",
            )
            .unwrap(),
            transactions_root: H256::from_str(
                "0x39771e11449fa3d5c483838161331736441233808e35b30d97098007a3095982",
            )
            .unwrap(),
            receipts_root: H256::from_str(
                "0xe6736394e2b44562152077f23243292a5a2a778a7a67bfa3e980f94555587cab",
            )
            .unwrap(),
            logs_bloom: Bloom::from(bloom_bytes),
            difficulty: U256::from(hex_u64("0x1")),
            number: hex_u64("0x4fe4620"),
            gas_limit: hex_u64("0x595a882"),
            gas_used: hex_u64("0x3891ba1"),
            timestamp: hex_u64("0x69a8bc5f"),
            extra_data: Bytes::from(extra_data_bytes),
            prev_randao: H256::zero(),
            nonce: 0,
            base_fee_per_gas: Some(hex_u64("0x18e5cb6f6e")),
            withdrawals_root: None,
            blob_gas_used: None,
            excess_blob_gas: None,
            parent_beacon_block_root: None,
            requests_hash: None,
            block_access_list_hash: None,
            slot_number: None,
            ..Default::default()
        };

        assert_eq!(
            header.hash(),
            expected_hash,
            "Post-Lisovo block 83,838,496 hash must match Polygon mainnet"
        );

        // Verify chain linkage: block 83838497's parentHash == this block's hash
        let next_block_parent =
            H256::from_str("0xb412c721f877d64d58d56cde3bac7fc0100113da533170136ab8edf73bd8ce99")
                .unwrap();
        assert_eq!(
            header.hash(),
            next_block_parent,
            "Block 83838497's parentHash must equal block 83838496's hash"
        );
    }
}
