use bytes::Bytes;
use ethereum_types::{Address, U256};
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};

/// Fixed number of extra-data prefix bytes reserved for signer vanity.
pub const EXTRA_VANITY_LENGTH: usize = 32;
/// Fixed number of extra-data suffix bytes reserved for signer seal (signature).
pub const EXTRA_SEAL_LENGTH: usize = 65;
/// Each validator entry is 20 bytes address + 20 bytes big-endian padded voting power.
const VALIDATOR_ENTRY_SIZE: usize = 40;

/// Parsed Bor block extra data (post-Lisovo RLP format).
///
/// The raw extra data field is:
///   [32 bytes vanity][RLP(BlockExtraData)][65 bytes signature]
///
/// Reference: bor/core/types/block.go `BlockExtraData`
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlockExtraData {
    /// Validator bytes: N*40 bytes at sprint-end blocks, empty otherwise.
    /// Each 40-byte chunk is [20-byte address][20-byte big-endian padded voting power].
    pub validator_bytes: Vec<u8>,
    /// Transaction dependency hints for parallel execution.
    pub tx_dependency: Vec<Vec<u64>>,
    /// EIP-1559 gas target (post-Giugliano).
    pub gas_target: Option<u64>,
    /// EIP-1559 base fee change denominator (post-Giugliano).
    pub base_fee_change_denominator: Option<u64>,
}

impl RLPEncode for BlockExtraData {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        let mut encoder = Encoder::new(buf)
            .encode_bytes(&self.validator_bytes)
            .encode_field(&self.tx_dependency);
        encoder = encoder.encode_optional_field(&self.gas_target);
        encoder = encoder.encode_optional_field(&self.base_fee_change_denominator);
        encoder.finish();
    }
}

impl RLPDecode for BlockExtraData {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        // Decode as Bytes (RLP string) then convert to Vec<u8> to match Go's []byte.
        let (validator_bytes_raw, decoder): (Bytes, _) = decoder.decode_field("validator_bytes")?;
        let validator_bytes = validator_bytes_raw.to_vec();
        let (tx_dependency, decoder): (Vec<Vec<u64>>, _) = decoder.decode_field("tx_dependency")?;
        let (gas_target, decoder) = decoder.decode_optional_field();
        let (base_fee_change_denominator, decoder) = decoder.decode_optional_field();
        let rest = decoder.finish_unchecked();
        Ok((
            BlockExtraData {
                validator_bytes,
                tx_dependency,
                gas_target,
                base_fee_change_denominator,
            },
            rest,
        ))
    }
}

/// A parsed validator entry: address + voting power.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatorEntry {
    pub address: Address,
    pub voting_power: U256,
}

/// Errors from parsing the extra data field.
#[derive(Debug, thiserror::Error)]
pub enum ExtraDataError {
    #[error("extra data too short: need at least {min} bytes, got {got}")]
    TooShort { min: usize, got: usize },
    #[error("failed to RLP-decode BlockExtraData: {0}")]
    RlpDecode(#[from] RLPDecodeError),
    #[error("invalid validator bytes length: {0} is not a multiple of 40")]
    InvalidValidatorBytesLength(usize),
}

/// Parses the raw `extra` field from a Bor block header (post-Lisovo RLP format).
///
/// **Important:** This only handles the post-Lisovo format where the middle
/// section is RLP-encoded `BlockExtraData`. Pre-Lisovo sprint-end blocks use
/// raw validator bytes (N*40 bytes) instead of RLP, which this function will
/// misdecode. Use only for blocks at or after the Lisovo fork.
///
/// Returns `(vanity, block_extra_data, signature)`.
pub fn parse_extra_data(
    extra: &[u8],
) -> Result<
    (
        [u8; EXTRA_VANITY_LENGTH],
        BlockExtraData,
        [u8; EXTRA_SEAL_LENGTH],
    ),
    ExtraDataError,
> {
    let min_len = EXTRA_VANITY_LENGTH + EXTRA_SEAL_LENGTH;
    if extra.len() < min_len {
        return Err(ExtraDataError::TooShort {
            min: min_len,
            got: extra.len(),
        });
    }

    let vanity: [u8; EXTRA_VANITY_LENGTH] = extra[..EXTRA_VANITY_LENGTH]
        .try_into()
        .expect("slice length checked");

    let signature = extract_signature(extra);

    let rlp_bytes = &extra[EXTRA_VANITY_LENGTH..extra.len() - EXTRA_SEAL_LENGTH];
    let block_extra_data = if rlp_bytes.is_empty() {
        BlockExtraData::default()
    } else {
        BlockExtraData::decode(rlp_bytes)?
    };

    Ok((vanity, block_extra_data, signature))
}

/// Returns the extra data with the last 65 signature bytes stripped.
/// Used for computing the seal hash.
pub fn strip_signature(extra: &[u8]) -> &[u8] {
    if extra.len() < EXTRA_SEAL_LENGTH {
        extra
    } else {
        &extra[..extra.len() - EXTRA_SEAL_LENGTH]
    }
}

/// Extracts the 65-byte signature from the end of the extra data.
///
/// # Panics
/// Panics if `extra.len() < EXTRA_SEAL_LENGTH`.
pub fn extract_signature(extra: &[u8]) -> [u8; EXTRA_SEAL_LENGTH] {
    let start = extra.len() - EXTRA_SEAL_LENGTH;
    extra[start..]
        .try_into()
        .expect("slice length checked above")
}

/// Parses raw validator bytes into (address, voting_power) pairs.
///
/// Each validator entry is 40 bytes: [20-byte address][20-byte big-endian padded power].
/// Reference: bor/consensus/bor/valset/validator.go `ParseValidators`
pub fn parse_validators(validator_bytes: &[u8]) -> Result<Vec<ValidatorEntry>, ExtraDataError> {
    if !validator_bytes.len().is_multiple_of(VALIDATOR_ENTRY_SIZE) {
        return Err(ExtraDataError::InvalidValidatorBytesLength(
            validator_bytes.len(),
        ));
    }

    let count = validator_bytes.len() / VALIDATOR_ENTRY_SIZE;
    let mut result = Vec::with_capacity(count);

    for i in 0..count {
        let offset = i * VALIDATOR_ENTRY_SIZE;
        let address = Address::from_slice(&validator_bytes[offset..offset + 20]);
        let voting_power = U256::from_big_endian(&validator_bytes[offset + 20..offset + 40]);
        result.push(ValidatorEntry {
            address,
            voting_power,
        });
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_rlp::encode::encode;

    #[test]
    fn test_roundtrip_empty_block_extra_data() {
        let data = BlockExtraData::default();
        let encoded = encode(data.clone());
        let decoded = BlockExtraData::decode(&encoded).expect("decode should succeed");
        assert_eq!(data, decoded);
    }

    #[test]
    fn test_roundtrip_with_validators() {
        let mut validator_bytes = vec![0u8; 80]; // 2 validators
        validator_bytes[19] = 0x01; // addr1 = 0x00..01
        validator_bytes[39] = 0x0a; // power1 = 10
        validator_bytes[59] = 0x02; // addr2 = 0x00..02
        validator_bytes[79] = 0x14; // power2 = 20

        let data = BlockExtraData {
            validator_bytes,
            tx_dependency: vec![vec![0, 1], vec![2]],
            gas_target: None,
            base_fee_change_denominator: None,
        };
        let encoded = encode(data.clone());
        let decoded = BlockExtraData::decode(&encoded).expect("decode should succeed");
        assert_eq!(data, decoded);
    }

    #[test]
    fn test_roundtrip_with_giugliano_fields() {
        let data = BlockExtraData {
            validator_bytes: vec![],
            tx_dependency: vec![],
            gas_target: Some(15_000_000),
            base_fee_change_denominator: Some(16),
        };
        let encoded = encode(data.clone());
        let decoded = BlockExtraData::decode(&encoded).expect("decode should succeed");
        assert_eq!(data, decoded);
    }

    #[test]
    fn test_parse_extra_data_minimal() {
        // 32 vanity + empty RLP list + 65 signature
        let mut extra = vec![0xAA; EXTRA_VANITY_LENGTH];
        let empty_extra = BlockExtraData::default();
        let rlp_bytes = encode(empty_extra.clone());
        extra.extend_from_slice(&rlp_bytes);
        extra.extend_from_slice(&[0xBB; EXTRA_SEAL_LENGTH]);

        let (vanity, data, sig) = parse_extra_data(&extra).expect("should parse");
        assert_eq!(vanity, [0xAA; EXTRA_VANITY_LENGTH]);
        assert_eq!(sig, [0xBB; EXTRA_SEAL_LENGTH]);
        assert_eq!(data, empty_extra);
    }

    #[test]
    fn test_parse_extra_data_too_short() {
        let extra = vec![0u8; 50]; // less than 97
        assert!(parse_extra_data(&extra).is_err());
    }

    #[test]
    fn test_strip_signature() {
        let extra = vec![0u8; 100];
        let stripped = strip_signature(&extra);
        assert_eq!(stripped.len(), 35);
    }

    #[test]
    fn test_extract_signature() {
        let mut extra = vec![0u8; 100];
        extra[99] = 0xFF;
        let sig = extract_signature(&extra);
        assert_eq!(sig[64], 0xFF);
        assert_eq!(sig.len(), 65);
    }

    #[test]
    fn test_parse_validators() {
        let mut bytes = vec![0u8; 80];
        // Validator 1: address 0x00..01, power 10
        bytes[19] = 0x01;
        bytes[39] = 0x0a;
        // Validator 2: address 0x00..02, power 20
        bytes[59] = 0x02;
        bytes[79] = 0x14;

        let validators = parse_validators(&bytes).expect("should parse");
        assert_eq!(validators.len(), 2);
        assert_eq!(validators[0].address, Address::from_low_u64_be(1));
        assert_eq!(validators[0].voting_power, U256::from(10));
        assert_eq!(validators[1].address, Address::from_low_u64_be(2));
        assert_eq!(validators[1].voting_power, U256::from(20));
    }

    #[test]
    fn test_parse_validators_invalid_length() {
        let bytes = vec![0u8; 41]; // not a multiple of 40
        assert!(parse_validators(&bytes).is_err());
    }

    #[test]
    fn test_parse_validators_empty() {
        let validators = parse_validators(&[]).expect("should parse empty");
        assert!(validators.is_empty());
    }

    #[test]
    fn test_parse_extra_data_sprint_end_with_validators() {
        // Build extra data for a sprint-end block: vanity + RLP(BlockExtraData with validators) + sig
        let mut validator_bytes = vec![0u8; 80]; // 2 validators
        validator_bytes[19] = 0x01; // addr1 = 0x00..01
        validator_bytes[39] = 0x0a; // power1 = 10
        validator_bytes[59] = 0x02; // addr2 = 0x00..02
        validator_bytes[79] = 0x14; // power2 = 20

        let block_extra = BlockExtraData {
            validator_bytes: validator_bytes.clone(),
            tx_dependency: vec![],
            gas_target: None,
            base_fee_change_denominator: None,
        };
        let rlp_bytes = encode(block_extra.clone());

        let mut extra = vec![0xAA; EXTRA_VANITY_LENGTH];
        extra.extend_from_slice(&rlp_bytes);
        extra.extend_from_slice(&[0xBB; EXTRA_SEAL_LENGTH]);

        let (vanity, data, sig) = parse_extra_data(&extra).expect("should parse");
        assert_eq!(vanity, [0xAA; EXTRA_VANITY_LENGTH]);
        assert_eq!(sig, [0xBB; EXTRA_SEAL_LENGTH]);
        assert_eq!(data, block_extra);

        // Parse validators from the extracted data
        let validators = parse_validators(&data.validator_bytes).expect("should parse validators");
        assert_eq!(validators.len(), 2);
        assert_eq!(validators[0].address, Address::from_low_u64_be(1));
        assert_eq!(validators[0].voting_power, U256::from(10));
        assert_eq!(validators[1].address, Address::from_low_u64_be(2));
        assert_eq!(validators[1].voting_power, U256::from(20));
    }

    #[test]
    fn test_parse_extra_data_exactly_minimum_length() {
        // Exactly 97 bytes: 32 vanity + 0 RLP + 65 signature
        let mut extra = vec![0u8; EXTRA_VANITY_LENGTH + EXTRA_SEAL_LENGTH];
        extra[0] = 0xAA;
        extra[96] = 0xBB;
        let (vanity, data, sig) = parse_extra_data(&extra).expect("should parse");
        assert_eq!(vanity[0], 0xAA);
        assert_eq!(sig[64], 0xBB);
        // Empty RLP region -> default BlockExtraData
        assert_eq!(data, BlockExtraData::default());
    }

    #[test]
    fn test_parse_extra_data_96_bytes_rejected() {
        let extra = vec![0u8; 96]; // one less than minimum
        let err = parse_extra_data(&extra);
        assert!(err.is_err());
        match err.unwrap_err() {
            ExtraDataError::TooShort { min, got } => {
                assert_eq!(min, 97);
                assert_eq!(got, 96);
            }
            e => panic!("expected TooShort, got {e:?}"),
        }
    }

    #[test]
    fn test_parse_extra_data_empty_rejected() {
        assert!(parse_extra_data(&[]).is_err());
    }

    #[test]
    fn test_roundtrip_full_extra_data() {
        // Construct extra data, parse it back, verify equality
        let block_extra = BlockExtraData {
            validator_bytes: vec![],
            tx_dependency: vec![vec![1, 2], vec![3]],
            gas_target: Some(30_000_000),
            base_fee_change_denominator: Some(8),
        };
        let rlp_bytes = encode(block_extra.clone());

        let vanity_in = [0x42u8; EXTRA_VANITY_LENGTH];
        let sig_in = [0x99u8; EXTRA_SEAL_LENGTH];

        let mut extra = Vec::new();
        extra.extend_from_slice(&vanity_in);
        extra.extend_from_slice(&rlp_bytes);
        extra.extend_from_slice(&sig_in);

        let (vanity_out, data_out, sig_out) = parse_extra_data(&extra).expect("should parse");
        assert_eq!(vanity_out, vanity_in);
        assert_eq!(sig_out, sig_in);
        assert_eq!(data_out, block_extra);
    }

    #[test]
    fn test_parse_validators_not_multiple_of_40() {
        // 41 bytes is not a multiple of 40
        let bytes = vec![0u8; 41];
        let err = parse_validators(&bytes).unwrap_err();
        match err {
            ExtraDataError::InvalidValidatorBytesLength(len) => assert_eq!(len, 41),
            e => panic!("expected InvalidValidatorBytesLength, got {e:?}"),
        }
    }

    #[test]
    fn test_parse_validators_single() {
        let mut bytes = vec![0u8; 40];
        // Address: 0xdeadbeef...
        bytes[16] = 0xDE;
        bytes[17] = 0xAD;
        bytes[18] = 0xBE;
        bytes[19] = 0xEF;
        // Voting power: 1000
        bytes[38] = 0x03;
        bytes[39] = 0xE8;

        let validators = parse_validators(&bytes).expect("should parse");
        assert_eq!(validators.len(), 1);
        assert_eq!(validators[0].voting_power, U256::from(1000));
    }

    // ====================================================================
    // Cross-validation tests against real Polygon Bor mainnet data
    // ====================================================================

    /// Helper to decode a hex string (with or without 0x prefix) into bytes.
    fn hex_bytes(s: &str) -> Vec<u8> {
        let s = s.strip_prefix("0x").unwrap_or(s);
        hex::decode(s).expect("valid hex")
    }

    /// Parse real extra_data from a normal (non-sprint-end) Polygon mainnet block.
    ///
    /// Block 50,000,000 has 97 bytes of extra_data: 32 vanity + 0 middle + 65 seal.
    /// Since the RLP region is empty, parse_extra_data should return BlockExtraData::default().
    #[test]
    fn crosscheck_normal_block_extra_data_parsing() {
        let extra = hex_bytes(
            "d78301000683626f7288676f312e32302e38856c696e75780000000000000000\
             b6c6f13270d722c179f577c7669e775841f090e7dc37640e3da15ba28dbaef47\
             2f1b963fc7b3ca1b884cc23cb2b302ec1ce6e8bf0a9164d8181407e0d7a2f79801",
        );
        assert_eq!(
            extra.len(),
            97,
            "block 50,000,000 extra_data should be 97 bytes"
        );

        let (vanity, block_extra, sig) = parse_extra_data(&extra).expect("should parse");

        // Vanity starts with Bor version info: d7 83 01 00 06 83 "bor" 88 "go1.20.8" 85 "linux"
        assert_eq!(vanity[0], 0xd7);

        // No validators in normal block
        assert_eq!(
            block_extra,
            BlockExtraData::default(),
            "normal block should have empty BlockExtraData"
        );
        assert!(
            block_extra.validator_bytes.is_empty(),
            "no validator bytes in non-sprint-end block"
        );

        // Signature should be 65 bytes
        assert_eq!(sig.len(), EXTRA_SEAL_LENGTH);
        // Last byte of signature (recovery id)
        assert_eq!(sig[64], 0x01);
    }

    /// Parse real extra_data from another normal block (50,000,001).
    #[test]
    fn crosscheck_normal_block_50000001_extra_data() {
        let extra = hex_bytes(
            "d78301000683626f7288676f312e32302e38856c696e75780000000000000000\
             bf08661ce53449e03a43e3dd0de6ee56181cc5320145f2d50247417eaa5cef70\
             0c125e2a9151fd5b6d53e31789fedffdbebb66df59a2b43e285e8e09367a026c01",
        );
        assert_eq!(extra.len(), 97);

        let (vanity, block_extra, sig) = parse_extra_data(&extra).expect("should parse");

        // Same vanity prefix (same Bor version)
        assert_eq!(vanity[0], 0xd7);
        assert_eq!(block_extra, BlockExtraData::default());
        assert_eq!(sig[64], 0x01); // recovery id
    }

    /// Verify extract_signature and parse_validators with real sprint-end block data.
    ///
    /// Block 49,999,999 is a sprint-end block (49999999 % 16 == 15, post-Delhi sprint=16).
    /// It contains 21 validators in pre-Giugliano format (raw validator bytes, not RLP-wrapped).
    ///
    /// Note: parse_extra_data() expects post-Giugliano RLP format and won't decode
    /// pre-Giugliano sprint-end blocks correctly. We test the lower-level helpers instead.
    #[test]
    fn crosscheck_sprint_end_block_validators() {
        use std::str::FromStr;

        // Exact hex from Polygon RPC for block 49,999,999 extraData.
        let extra = hex_bytes(
            "d88301010083626f7289676f312e32302e3130856c696e757800000000000000048cfedf907c4c9ddd11ff882380906e78e84bbe00000000000000000000000000000000000000011efecb61a2f80aa34d3b9218b564a64d059462900000000000000000000000000000000000000002272cc48bb89900689c7e51c6a3ab039ec5cc80f900000000000000000000000000000000000000012c74ca71679cf1299936d6104d825c965448907b000000000000000000000000000000000000000143c7c14d94197a30a44dab27bfb3eee9e05496d4000000000000000000000000000000000000000146a3a41bd932244dd08186e4c19f1a7e48cbcdf4000000000000000000000000000000000000000767b94473d81d0cd00849d563c94d0432ac988b49000000000000000000000000000000000000000773d378cfeaa5cbe8daed64128ebdc91322aa586b0000000000000000000000000000000000000002794e44d1334a56fea7f4df12633b88820d0c588800000000000000000000000000000000000000037c7379531b2aee82e4ca06d4175d13b9cbeafd4900000000000000000000000000000000000000068842ea85732f94feeb9cf1ccc7d357c63658e7a4000000000000000000000000000000000000000198c27cc3f0301b6272049dc3f972e2f54278062900000000000000000000000000000000000000019ead03f7136fc6b4bdb0780b00a1c14ae5a8b6d00000000000000000000000000000000000000006a8b52f02108aa5f4b675bdcc973760022d7c60200000000000000000000000000000000000000002bdbd4347b082d9d6bdf2da4555a37ce52a2e21200000000000000000000000000000000000000001c6869257205e20c2a43cb31345db534aecb49f6e0000000000000000000000000000000000000001e63727cb2b3a8d6e3a2d1df4990f441938b67a340000000000000000000000000000000000000001e7e2cb8c81c10ff191a73fe266788c9ce62ec7540000000000000000000000000000000000000001ec20607aa654d823dd01beb8780a44863c57ed070000000000000000000000000000000000000001eedba2484aaf940f37cd3cd21a5d7c4a7dafbfc00000000000000000000000000000000000000002f0245f6251bef9447a08766b9da2b07b28ad80b00000000000000000000000000000000000000002047c5f98ba5626525f9a49f8980f993e67d1fcd84eaec54197fbb17adabdef1a729284686be02443416f019210ab224d3fd522fa694a59b553fb0370663151cd00",
        );

        // 32 vanity + 21*40 validator bytes + 65 seal = 937
        assert_eq!(extra.len(), 937, "sprint-end block should be 937 bytes");

        // extract_signature works on any format
        let sig = extract_signature(&extra);
        assert_eq!(sig[64], 0x00); // recovery id

        // Extract raw validator bytes (between vanity and seal)
        let validator_bytes = &extra[EXTRA_VANITY_LENGTH..extra.len() - EXTRA_SEAL_LENGTH];
        assert_eq!(validator_bytes.len(), 840, "21 validators x 40 bytes = 840");

        let validators = parse_validators(validator_bytes).expect("should parse 21 validators");
        assert_eq!(
            validators.len(),
            21,
            "block 49,999,999 should have 21 validators"
        );

        // Verify first validator
        assert_eq!(
            validators[0].address,
            Address::from_str("0x048cfedf907c4c9ddd11ff882380906e78e84bbe").unwrap(),
            "first validator address"
        );
        assert_eq!(
            validators[0].voting_power,
            U256::from(1),
            "first validator power"
        );

        // Verify last validator
        assert_eq!(
            validators[20].address,
            Address::from_str("0xf0245f6251bef9447a08766b9da2b07b28ad80b0").unwrap(),
            "last validator address"
        );
        assert_eq!(
            validators[20].voting_power,
            U256::from(2),
            "last validator power"
        );

        // Verify a high-power validator (0x46a3a41b... has power 7)
        assert_eq!(
            validators[5].address,
            Address::from_str("0x46a3a41bd932244dd08186e4c19f1a7e48cbcdf4").unwrap(),
            "validator #5 address"
        );
        assert_eq!(
            validators[5].voting_power,
            U256::from(7),
            "validator #5 power"
        );

        // Total voting power: 1+2+1+1+1+7+7+2+3+6+1+1+6+2+1+1+1+1+1+2+2 = 50
        let total_power = validators
            .iter()
            .fold(U256::zero(), |acc, v| acc + v.voting_power);
        assert_eq!(total_power, U256::from(50), "total voting power");
    }

    /// Parse real extra_data from a post-Lisovo/post-Cancun sprint-end block.
    ///
    /// Block 83,838,496 (0x4FE4620) uses the post-Cancun RLP BlockExtraData format.
    /// The fact-checker confirmed: validators are ALWAYS empty on mainnet (validator set
    /// comes from Heimdall). The TxDependency field is populated with transaction
    /// dependency hints for parallel execution.
    #[test]
    fn crosscheck_post_cancun_sprint_end_extra_data() {
        // Full extra_data from block 83,838,496 via Polygon RPC.
        let extra = hex_bytes(
            "d78301100883626f7288676f312e32352e37856c696e75780000000000000000f901f780f901f3c0c180c101c0c101c0c105c0c0c20706c0c0c10bc10cc0c0c109c110c104c112c113c114c115c10ac117c118c119c20216c11bc11cc11dc21006c11ec120c121c122c123c124c125c21d1bc126c22728c129c12ac11fc12cc12dc12ec12fc130c111c23111c133c134c20e35c136c137c138c139c13ac13bc13cc13dc13ec13fc140c141c142c143c144c145c146c147c148c149c14ac14bc14cc14dc14ec14fc150c151c152c153c154c155c156c157c158c159c15ac15bc75c1f46383b4050c15cc25d5ec15fc160c161c162c163c12bc164c166c22965c167c169c16ac16bc16cc16dc16ec16fc170c171c172c173c174c168c8207624801214161cc175c178c179c17ac132c17bc17dc17ec17fc28180c28181c28182c28183c28184c28185c28186c28187c28188c177c28189cd60627a818b3850474e7e5c3d40c2818cc2818dc17cc2818ec28190c28191c28192c28193c28194c376818ac28195c48197818bc28198c28199c2819ac2819bc2819cc2818fc2819dc2819fc281a0c281a1c281a2c2819ec281a3c0c281a4c281a7c32381a8c28196c481a981a7c0c281abc281a5c0c281aec108c281b0c281b1c281b2c0c106c281b4c281b7ca81b681b881ad81b081b4c281b9c281b9ca121c247680141620818ac281bcc281bdc281bec281bfc281c0c281c1c281c2c281c3c281c4dbb5af28089606322194e84c9c339abcabc383fb9d0ca3a06c2e640fd59a79a04a5ba58b05c62121228fc3e5aad3f1bf86363164142df407cbfc7915e6f5b75a01",
        );

        let (vanity, block_extra, sig) =
            parse_extra_data(&extra).expect("should parse post-Cancun extra_data");

        // Vanity: Bor v1.1.0, go1.25.7, linux
        assert_eq!(vanity[0], 0xd7);

        // Validator bytes are always empty on Polygon mainnet
        assert!(
            block_extra.validator_bytes.is_empty(),
            "validator_bytes must be empty on Polygon mainnet (validator set comes from Heimdall)"
        );

        // TxDependency should be populated (transaction dependency graph for parallel execution)
        assert!(
            !block_extra.tx_dependency.is_empty(),
            "tx_dependency should be populated in post-Cancun blocks"
        );

        // Signature: last byte is recovery id
        assert_eq!(sig[64], 0x01);
    }

    /// Parse real extra_data from a post-Lisovo non-sprint block.
    ///
    /// Block 0x4FE4621 — non-sprint, post-Lisovo.
    /// Real data from Polygon mainnet RPC (674 bytes).
    #[test]
    fn crosscheck_post_cancun_non_sprint_extra_data() {
        let extra = hex_bytes(
            "d78301100883626f7288676f312e32352e37856c696e75780000000000000000\
             f9023e80f9023ac0c180c101c0c102c104c105c106c107c108c103c0c0c109c1\
             05c10dc10fc2100ec111c112c113c114c115c116c117c0c3141118c118c11bc1\
             1cc0c11ec0c120c121c122c11dc124c125c126c11ac406020f26c0c10ac12bc1\
             2cc12dc12ec3230c1fc130c131c132c133c134c135c136c137c138c0c139c13b\
             c13cc13dc13ec13fc140c141c142c143c144c145c146c147c148c149c13ac14a\
             c14cc14dc14ec14fc150c151c152c153c154c155c156c157c158c159c139c15a\
             c15cc15dc15ec15fc160c161c162c163c25b64c165c166c167c168c169c16ac1\
             6bc16cc56d2140485ec16ec16fc127c170c172c173c174c175c176c177c27128\
             c178c17ac17bc17cc17dc17ec17fc28180c28181c28182c28183c28184c28185\
             c28186c28187c28188c179c28189c2818bc2818cc2818dc2818ec2818ac2818f\
             c28191c28192c28193c28194c28195c28196c28197c28198c28199c2819ac281\
             9bc2819cc2819dc2819ec2819fc281a0c281a1c28190c281a2c281a4c32981a3\
             c481a56d4bc281a7c281a8c281a9c281aac281abc281acc281adc281a6c281ae\
             c281b0c281b1c281b2c281b3c281b4c281b5c281b6c281b7c281b8c281b9c2\
             81bac281bbc281bcc281bdc281bec281bfc281c0c281c1c281c2c281c3c0c281\
             afc281c6c281c7c0c0c281a7c281cbc281ccc281cdc281c4c0d7818981a181ae\
             81b581b981bb81cf6d81a4819e81ac81c0c281c8c0c12fc281d1c281cfc281d4\
             c281d7c281cec281d9c0c281d5c281dcc481d681ddc281dec481d981dfc481df\
             81a519ca6e70943d1e3e5a63417049dee0a98abd34958a9153fad339539b18fa\
             fd7e67fe3307b80593399a3cfe4442bf07bc34b507a8b25ced9d9008ecfc6a12\
             114d01",
        );

        let (vanity, block_extra, sig) =
            parse_extra_data(&extra).expect("should parse post-Cancun extra_data");

        // Same Bor version vanity
        assert_eq!(vanity[0], 0xd7);

        // Validators empty on non-sprint blocks too
        assert!(
            block_extra.validator_bytes.is_empty(),
            "validator_bytes must be empty on non-sprint block"
        );

        // TxDependency populated
        assert!(
            !block_extra.tx_dependency.is_empty(),
            "tx_dependency should be populated"
        );

        // Signature recovery id
        assert_eq!(sig[64], 0x01);
    }

    /// Verify that strip_signature works correctly on real extra_data.
    #[test]
    fn crosscheck_strip_signature_real_data() {
        let extra = hex_bytes(
            "d78301000683626f7288676f312e32302e38856c696e75780000000000000000\
             b6c6f13270d722c179f577c7669e775841f090e7dc37640e3da15ba28dbaef47\
             2f1b963fc7b3ca1b884cc23cb2b302ec1ce6e8bf0a9164d8181407e0d7a2f79801",
        );

        let stripped = strip_signature(&extra);
        // Should be just the 32-byte vanity (97 - 65 = 32)
        assert_eq!(stripped.len(), 32);
        // Vanity bytes preserved
        assert_eq!(stripped[0], 0xd7);
    }

    /// Verify strip_signature on post-Cancun extra_data preserves vanity + RLP payload.
    #[test]
    fn crosscheck_strip_signature_post_cancun() {
        // Block 83,838,496 extra_data
        let extra = hex_bytes(
            "d78301100883626f7288676f312e32352e37856c696e75780000000000000000f901f780f901f3c0c180c101c0c101c0c105c0c0c20706c0c0c10bc10cc0c0c109c110c104c112c113c114c115c10ac117c118c119c20216c11bc11cc11dc21006c11ec120c121c122c123c124c125c21d1bc126c22728c129c12ac11fc12cc12dc12ec12fc130c111c23111c133c134c20e35c136c137c138c139c13ac13bc13cc13dc13ec13fc140c141c142c143c144c145c146c147c148c149c14ac14bc14cc14dc14ec14fc150c151c152c153c154c155c156c157c158c159c15ac15bc75c1f46383b4050c15cc25d5ec15fc160c161c162c163c12bc164c166c22965c167c169c16ac16bc16cc16dc16ec16fc170c171c172c173c174c168c8207624801214161cc175c178c179c17ac132c17bc17dc17ec17fc28180c28181c28182c28183c28184c28185c28186c28187c28188c177c28189cd60627a818b3850474e7e5c3d40c2818cc2818dc17cc2818ec28190c28191c28192c28193c28194c376818ac28195c48197818bc28198c28199c2819ac2819bc2819cc2818fc2819dc2819fc281a0c281a1c281a2c2819ec281a3c0c281a4c281a7c32381a8c28196c481a981a7c0c281abc281a5c0c281aec108c281b0c281b1c281b2c0c106c281b4c281b7ca81b681b881ad81b081b4c281b9c281b9ca121c247680141620818ac281bcc281bdc281bec281bfc281c0c281c1c281c2c281c3c281c4dbb5af28089606322194e84c9c339abcabc383fb9d0ca3a06c2e640fd59a79a04a5ba58b05c62121228fc3e5aad3f1bf86363164142df407cbfc7915e6f5b75a01",
        );

        let stripped = strip_signature(&extra);
        // Should be vanity(32) + RLP payload
        assert_eq!(stripped.len(), extra.len() - EXTRA_SEAL_LENGTH);
        // Starts with vanity
        assert_eq!(stripped[0], 0xd7);
        // RLP payload starts after vanity with f9 prefix (long list)
        assert_eq!(stripped[32], 0xf9);
    }

    #[test]
    fn test_strip_signature_shorter_than_seal() {
        let extra = vec![0u8; 10]; // shorter than 65
        let stripped = strip_signature(&extra);
        assert_eq!(stripped, &extra[..]); // returns full slice
    }

    #[test]
    fn test_block_extra_data_with_giugliano_fields_roundtrip() {
        let data = BlockExtraData {
            validator_bytes: vec![0u8; 40], // 1 validator
            tx_dependency: vec![vec![0]],
            gas_target: Some(15_000_000),
            base_fee_change_denominator: Some(16),
        };
        let encoded = encode(data.clone());
        let decoded = BlockExtraData::decode(&encoded).expect("decode should succeed");
        assert_eq!(decoded.gas_target, Some(15_000_000));
        assert_eq!(decoded.base_fee_change_denominator, Some(16));
        assert_eq!(decoded.validator_bytes.len(), 40);
        assert_eq!(data, decoded);
    }
}
