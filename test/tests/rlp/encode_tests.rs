use std::net::IpAddr;

use ethereum_types::{Address, U256};
use ethrex_common::constants::{RLP_EMPTY_LIST, RLP_NULL};
use librlp::RlpEncode;
use hex_literal::hex;

#[test]
fn can_encode_booleans() {
    let mut encoded = Vec::new();
    true.encode_to_vec(&mut encoded);
    assert_eq!(encoded, vec![0x01]);

    let mut encoded = Vec::new();
    false.encode_to_vec(&mut encoded);
    assert_eq!(encoded, vec![RLP_NULL]);
}

#[test]
fn can_encode_u32() {
    let mut encoded = Vec::new();
    0u32.encode_to_vec(&mut encoded);
    assert_eq!(encoded, vec![RLP_NULL]);
    assert_eq!(encoded.len(), 0u32.encoded_length());

    let mut encoded = Vec::new();
    1u32.encode_to_vec(&mut encoded);
    assert_eq!(encoded, vec![0x01]);
    assert_eq!(encoded.len(), 1u32.encoded_length());

    let mut encoded = Vec::new();
    0x7Fu32.encode_to_vec(&mut encoded);
    assert_eq!(encoded, vec![0x7f]);
    assert_eq!(encoded.len(), 0x7Fu32.encoded_length());

    let mut encoded = Vec::new();
    0x80u32.encode_to_vec(&mut encoded);
    assert_eq!(encoded, vec![RLP_NULL + 1, 0x80]);
    assert_eq!(encoded.len(), 0x80u32.encoded_length());

    let mut encoded = Vec::new();
    0x90u32.encode_to_vec(&mut encoded);
    assert_eq!(encoded, vec![RLP_NULL + 1, 0x90]);
    assert_eq!(encoded.len(), 0x90u32.encoded_length());
}

#[test]
fn can_encode_u16() {
    let mut encoded = Vec::new();
    0u16.encode_to_vec(&mut encoded);
    assert_eq!(encoded, vec![RLP_NULL]);
    assert_eq!(encoded.len(), 0u16.encoded_length());

    let mut encoded = Vec::new();
    1u16.encode_to_vec(&mut encoded);
    assert_eq!(encoded, vec![0x01]);
    assert_eq!(encoded.len(), 1u16.encoded_length());

    let mut encoded = Vec::new();
    0x7Fu16.encode_to_vec(&mut encoded);
    assert_eq!(encoded, vec![0x7f]);
    assert_eq!(encoded.len(), 0x7Fu16.encoded_length());

    let mut encoded = Vec::new();
    0x80u16.encode_to_vec(&mut encoded);
    assert_eq!(encoded, vec![RLP_NULL + 1, 0x80]);
    assert_eq!(encoded.len(), 0x80u16.encoded_length());

    let mut encoded = Vec::new();
    0x90u16.encode_to_vec(&mut encoded);
    assert_eq!(encoded, vec![RLP_NULL + 1, 0x90]);
    assert_eq!(encoded.len(), 0x90u16.encoded_length());
}

#[test]
fn u16_length_matches() {
    let mut encoded = Vec::new();
    0x0100u16.encode_to_vec(&mut encoded);
    assert_eq!(encoded.len(), 0x0100u16.encoded_length(),);
}

#[test]
fn u256_length_matches() {
    let value = U256::from(0x0100u64);
    let mut encoded = Vec::new();
    value.encode_to_vec(&mut encoded);
    assert_eq!(encoded.len(), value.encoded_length(),);
}

#[test]
fn u64_lengths_match() {
    for n in 0u64..=10_000 {
        let mut encoded = Vec::new();
        n.encode_to_vec(&mut encoded);
        assert_eq!(
            encoded.len(),
            n.encoded_length(),
            "u64 length mismatch at value {n}"
        );
    }
}

#[test]
fn can_encode_u8() {
    let mut encoded = Vec::new();
    0u8.encode_to_vec(&mut encoded);
    assert_eq!(encoded, vec![RLP_NULL]);
    assert_eq!(encoded.len(), 0u8.encoded_length());

    let mut encoded = Vec::new();
    1u8.encode_to_vec(&mut encoded);
    assert_eq!(encoded, vec![0x01]);
    assert_eq!(encoded.len(), 1u8.encoded_length());

    let mut encoded = Vec::new();
    0x7Fu8.encode_to_vec(&mut encoded);
    assert_eq!(encoded, vec![0x7f]);
    assert_eq!(encoded.len(), 0x7Fu8.encoded_length());

    let mut encoded = Vec::new();
    0x80u8.encode_to_vec(&mut encoded);
    assert_eq!(encoded, vec![RLP_NULL + 1, 0x80]);
    assert_eq!(encoded.len(), 0x80u8.encoded_length());

    let mut encoded = Vec::new();
    0x90u8.encode_to_vec(&mut encoded);
    assert_eq!(encoded, vec![RLP_NULL + 1, 0x90]);
    assert_eq!(encoded.len(), 0x90u8.encoded_length());
}

#[test]
fn can_encode_u64() {
    let mut encoded = Vec::new();
    0u64.encode_to_vec(&mut encoded);
    assert_eq!(encoded, vec![RLP_NULL]);
    assert_eq!(encoded.len(), 0u64.encoded_length());

    let mut encoded = Vec::new();
    1u64.encode_to_vec(&mut encoded);
    assert_eq!(encoded, vec![0x01]);
    assert_eq!(encoded.len(), 1u64.encoded_length());

    let mut encoded = Vec::new();
    0x7Fu64.encode_to_vec(&mut encoded);
    assert_eq!(encoded, vec![0x7f]);
    assert_eq!(encoded.len(), 0x7Fu64.encoded_length());

    let mut encoded = Vec::new();
    0x80u64.encode_to_vec(&mut encoded);
    assert_eq!(encoded, vec![RLP_NULL + 1, 0x80]);
    assert_eq!(encoded.len(), 0x80u64.encoded_length());

    let mut encoded = Vec::new();
    0x90u64.encode_to_vec(&mut encoded);
    assert_eq!(encoded, vec![RLP_NULL + 1, 0x90]);
    assert_eq!(encoded.len(), 0x90u64.encoded_length());
}

#[test]
fn can_encode_usize() {
    let mut encoded = Vec::new();
    0usize.encode_to_vec(&mut encoded);
    assert_eq!(encoded, vec![0x80]);
    assert_eq!(encoded.len(), 0usize.encoded_length());

    let mut encoded = Vec::new();
    1usize.encode_to_vec(&mut encoded);
    assert_eq!(encoded, vec![0x01]);
    assert_eq!(encoded.len(), 1usize.encoded_length());

    let mut encoded = Vec::new();
    0x7Fusize.encode_to_vec(&mut encoded);
    assert_eq!(encoded, vec![0x7f]);
    assert_eq!(encoded.len(), 0x7Fusize.encoded_length());

    let mut encoded = Vec::new();
    0x80usize.encode_to_vec(&mut encoded);
    assert_eq!(encoded, vec![0x80 + 1, 0x80]);
    assert_eq!(encoded.len(), 0x80usize.encoded_length());

    let mut encoded = Vec::new();
    0x90usize.encode_to_vec(&mut encoded);
    assert_eq!(encoded, vec![0x80 + 1, 0x90]);
    assert_eq!(encoded.len(), 0x90usize.encoded_length());
}

#[test]
fn can_encode_bytes() {
    // encode byte 0x00
    let message: [u8; 1] = [0x00];
    let encoded = {
        let mut buf = vec![];
        message.encode_to_vec(&mut buf);
        buf
    };
    assert_eq!(encoded, vec![0x00]);
    assert_eq!(encoded.len(), message.encoded_length());

    // encode byte 0x0f
    let message: [u8; 1] = [0x0f];
    let encoded = {
        let mut buf = vec![];
        message.encode_to_vec(&mut buf);
        buf
    };
    assert_eq!(encoded, vec![0x0f]);
    assert_eq!(encoded.len(), message.encoded_length());

    // encode bytes '\x04\x00'
    let message: [u8; 2] = [0x04, 0x00];
    let encoded = {
        let mut buf = vec![];
        message.encode_to_vec(&mut buf);
        buf
    };
    assert_eq!(encoded, vec![RLP_NULL + 2, 0x04, 0x00]);
    assert_eq!(encoded.len(), message.encoded_length());
}

#[test]
fn can_encode_strings() {
    // encode dog
    let message = "dog";
    let encoded = {
        let mut buf = vec![];
        message.encode_to_vec(&mut buf);
        buf
    };
    let expected: [u8; 4] = [RLP_NULL + 3, b'd', b'o', b'g'];
    assert_eq!(encoded, expected);
    assert_eq!(encoded.len(), message.encoded_length());

    // encode empty string
    let message = "";
    let encoded = {
        let mut buf = vec![];
        message.encode_to_vec(&mut buf);
        buf
    };
    let expected: [u8; 1] = [RLP_NULL];
    assert_eq!(encoded, expected);
    assert_eq!(encoded.len(), message.encoded_length());
}

#[test]
fn can_encode_lists_of_str() {
    // encode ["cat", "dog"]
    let message = vec!["cat", "dog"];
    let encoded = librlp::encode_list_to_rlp(&message);
    let expected: [u8; 9] = [0xc8, 0x83, b'c', b'a', b't', 0x83, b'd', b'o', b'g'];
    assert_eq!(encoded, expected);

    // encode empty list
    let message: Vec<&str> = vec![];
    let encoded = librlp::encode_list_to_rlp(&message);
    let expected: [u8; 1] = [RLP_EMPTY_LIST];
    assert_eq!(encoded, expected);
}

#[test]
fn can_encode_ip() {
    // encode an IPv4 address
    let message = "192.168.0.1";
    let ip: IpAddr = message.parse().unwrap();
    let encoded = {
        let mut buf = vec![];
        ip.encode_to_vec(&mut buf);
        buf
    };
    let expected: [u8; 5] = [RLP_NULL + 4, 192, 168, 0, 1];
    assert_eq!(encoded, expected);
    assert_eq!(encoded.len(), ip.encoded_length());

    // encode an IPv6 address
    let message = "2001:0000:130F:0000:0000:09C0:876A:130B";
    let ip: IpAddr = message.parse().unwrap();
    let encoded = {
        let mut buf = vec![];
        ip.encode_to_vec(&mut buf);
        buf
    };
    let expected: [u8; 17] = [
        0x90, 0x20, 0x01, 0x00, 0x00, 0x13, 0x0f, 0x00, 0x00, 0x00, 0x00, 0x09, 0xc0, 0x87, 0x6a,
        0x13, 0x0b,
    ];
    assert_eq!(encoded, expected);
    assert_eq!(encoded.len(), ip.encoded_length());
}

#[test]
fn can_encode_addresses() {
    let address = Address::from(hex!("ef2d6d194084c2de36e0dabfce45d046b37d1106"));
    let encoded = {
        let mut buf = vec![];
        address.encode_to_vec(&mut buf);
        buf
    };
    let expected = hex!("94ef2d6d194084c2de36e0dabfce45d046b37d1106");
    assert_eq!(encoded, expected);
    assert_eq!(encoded.len(), address.encoded_length());
}

#[test]
fn can_encode_u256() {
    let mut encoded = Vec::new();
    U256::from(1).encode_to_vec(&mut encoded);
    assert_eq!(encoded, vec![1]);
    assert_eq!(encoded.len(), U256::from(1).encoded_length());

    let mut encoded = Vec::new();
    U256::from(128).encode_to_vec(&mut encoded);
    assert_eq!(encoded, vec![0x80 + 1, 128]);
    assert_eq!(encoded.len(), U256::from(128).encoded_length());

    let mut encoded = Vec::new();
    U256::max_value().encode_to_vec(&mut encoded);
    let bytes = [0xff; 32];
    let mut expected: Vec<u8> = bytes.into();
    expected.insert(0, 0x80 + 32);
    assert_eq!(encoded, expected);
    assert_eq!(encoded.len(), U256::max_value().encoded_length());
}

#[test]
fn can_encode_tuple() {
    // TODO: check if works for tuples with total length greater than 55 bytes
    let tuple: (u8, u8) = (0x01, 0x02);
    let mut encoded = Vec::new();
    tuple.encode_to_vec(&mut encoded);
    let expected = vec![0xc0 + 2, 0x01, 0x02];
    assert_eq!(encoded, expected);
    assert_eq!(encoded.len(), tuple.encoded_length());
}
