//! Fuzz target for RLP roundtrip testing.
//!
//! This fuzzer generates structured data and verifies that
//! encode(decode(data)) == data when decode succeeds.

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

use bytes::Bytes;
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethereum_types::{Address, H256, U256};

/// A structured input for testing RLP roundtrips
#[derive(Debug, Arbitrary)]
enum RlpTestInput {
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    U128(u128),
    Bool(bool),
    String(String),
    Bytes(Vec<u8>),
    U256Bytes([u8; 32]),
    AddressBytes([u8; 20]),
    H256Bytes([u8; 32]),
    VecU8(Vec<u8>),
    VecU64(Vec<u64>),
    NestedVec(Vec<Vec<u8>>),
    TupleU8U8(u8, u8),
    TupleU64U64(u64, u64),
    TupleU64String(u64, String),
}

fuzz_target!(|input: RlpTestInput| {
    match input {
        RlpTestInput::U8(v) => {
            let encoded = v.encode_to_vec();
            let decoded = u8::decode(&encoded).unwrap();
            assert_eq!(v, decoded);
        }
        RlpTestInput::U16(v) => {
            let encoded = v.encode_to_vec();
            let decoded = u16::decode(&encoded).unwrap();
            assert_eq!(v, decoded);
        }
        RlpTestInput::U32(v) => {
            let encoded = v.encode_to_vec();
            let decoded = u32::decode(&encoded).unwrap();
            assert_eq!(v, decoded);
        }
        RlpTestInput::U64(v) => {
            let encoded = v.encode_to_vec();
            let decoded = u64::decode(&encoded).unwrap();
            assert_eq!(v, decoded);
        }
        RlpTestInput::U128(v) => {
            let encoded = v.encode_to_vec();
            let decoded = u128::decode(&encoded).unwrap();
            assert_eq!(v, decoded);
        }
        RlpTestInput::Bool(v) => {
            let encoded = v.encode_to_vec();
            let decoded = bool::decode(&encoded).unwrap();
            assert_eq!(v, decoded);
        }
        RlpTestInput::String(v) => {
            let encoded = v.encode_to_vec();
            let decoded = String::decode(&encoded).unwrap();
            assert_eq!(v, decoded);
        }
        RlpTestInput::Bytes(v) => {
            let encoded = v.as_slice().encode_to_vec();
            let decoded = Bytes::decode(&encoded).unwrap();
            assert_eq!(v.as_slice(), decoded.as_ref());
        }
        RlpTestInput::U256Bytes(bytes) => {
            let v = U256::from_big_endian(&bytes);
            let encoded = v.encode_to_vec();
            let decoded = U256::decode(&encoded).unwrap();
            assert_eq!(v, decoded);
        }
        RlpTestInput::AddressBytes(bytes) => {
            let v = Address::from_slice(&bytes);
            let encoded = v.encode_to_vec();
            let decoded = Address::decode(&encoded).unwrap();
            assert_eq!(v, decoded);
        }
        RlpTestInput::H256Bytes(bytes) => {
            let v = H256::from_slice(&bytes);
            let encoded = v.encode_to_vec();
            let decoded = H256::decode(&encoded).unwrap();
            assert_eq!(v, decoded);
        }
        RlpTestInput::VecU8(v) => {
            let encoded = v.encode_to_vec();
            let decoded = Vec::<u8>::decode(&encoded).unwrap();
            assert_eq!(v, decoded);
        }
        RlpTestInput::VecU64(v) => {
            let encoded = v.encode_to_vec();
            let decoded = Vec::<u64>::decode(&encoded).unwrap();
            assert_eq!(v, decoded);
        }
        RlpTestInput::NestedVec(v) => {
            let encoded = v.encode_to_vec();
            let decoded = Vec::<Vec<u8>>::decode(&encoded).unwrap();
            assert_eq!(v, decoded);
        }
        RlpTestInput::TupleU8U8(a, b) => {
            let v = (a, b);
            let encoded = v.encode_to_vec();
            let decoded = <(u8, u8)>::decode(&encoded).unwrap();
            assert_eq!(v, decoded);
        }
        RlpTestInput::TupleU64U64(a, b) => {
            let v = (a, b);
            let encoded = v.encode_to_vec();
            let decoded = <(u64, u64)>::decode(&encoded).unwrap();
            assert_eq!(v, decoded);
        }
        RlpTestInput::TupleU64String(a, b) => {
            let v = (a, b);
            let encoded = v.encode_to_vec();
            let decoded = <(u64, String)>::decode(&encoded).unwrap();
            assert_eq!(v, decoded);
        }
    }
});
