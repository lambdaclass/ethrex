use ethereum_types::U256;
use ethrex_rlp::constants::{RLP_EMPTY_LIST, RLP_NULL};
use ethrex_rlp::decode::RLPDecode;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

#[test]
fn test_decode_bool() {
    let rlp = vec![0x01];
    let decoded = bool::decode(&rlp).unwrap();
    assert!(decoded);

    let rlp = vec![RLP_NULL];
    let decoded = bool::decode(&rlp).unwrap();
    assert!(!decoded);
}

#[test]
fn test_decode_u8() {
    let rlp = vec![0x01];
    let decoded = u8::decode(&rlp).unwrap();
    assert_eq!(decoded, 1);

    let rlp = vec![RLP_NULL];
    let decoded = u8::decode(&rlp).unwrap();
    assert_eq!(decoded, 0);

    let rlp = vec![0x7Fu8];
    let decoded = u8::decode(&rlp).unwrap();
    assert_eq!(decoded, 127);

    let rlp = vec![RLP_NULL + 1, RLP_NULL];
    let decoded = u8::decode(&rlp).unwrap();
    assert_eq!(decoded, 128);

    let rlp = vec![RLP_NULL + 1, 0x90];
    let decoded = u8::decode(&rlp).unwrap();
    assert_eq!(decoded, 144);

    let rlp = vec![RLP_NULL + 1, 0xFF];
    let decoded = u8::decode(&rlp).unwrap();
    assert_eq!(decoded, 255);
}

#[test]
fn test_decode_u16() {
    let rlp = vec![0x01];
    let decoded = u8::decode(&rlp).unwrap();
    assert_eq!(decoded, 1);

    let rlp = vec![RLP_NULL];
    let decoded = u8::decode(&rlp).unwrap();
    assert_eq!(decoded, 0);

    let rlp = vec![0x81, 0xFF];
    let decoded = u8::decode(&rlp).unwrap();
    assert_eq!(decoded, 255);
}

#[test]
fn test_decode_u32() {
    let rlp = vec![0x83, 0x01, 0x00, 0x00];
    let decoded = u32::decode(&rlp).unwrap();
    assert_eq!(decoded, 65536);
}

#[test]
fn test_decode_fixed_length_array() {
    let rlp = vec![0x0f];
    let decoded = <[u8; 1]>::decode(&rlp).unwrap();
    assert_eq!(decoded, [0x0f]);

    let rlp = vec![RLP_NULL + 3, 0x02, 0x03, 0x04];
    let decoded = <[u8; 3]>::decode(&rlp).unwrap();
    assert_eq!(decoded, [0x02, 0x03, 0x04]);
}

#[test]
fn test_decode_ip_addresses() {
    // IPv4
    let rlp = vec![RLP_NULL + 4, 192, 168, 0, 1];
    let decoded = Ipv4Addr::decode(&rlp).unwrap();
    let expected = Ipv4Addr::from_str("192.168.0.1").unwrap();
    assert_eq!(decoded, expected);

    // IPv6
    let rlp = vec![
        0x90, 0x20, 0x01, 0x00, 0x00, 0x13, 0x0f, 0x00, 0x00, 0x00, 0x00, 0x09, 0xc0, 0x87, 0x6a,
        0x13, 0x0b,
    ];
    let decoded = Ipv6Addr::decode(&rlp).unwrap();
    let expected = Ipv6Addr::from_str("2001:0000:130F:0000:0000:09C0:876A:130B").unwrap();
    assert_eq!(decoded, expected);
}

#[test]
fn test_decode_u256() {
    let rlp = vec![RLP_NULL + 1, 0x01];
    let decoded = U256::decode(&rlp).unwrap();
    let expected = U256::from(1);
    assert_eq!(decoded, expected);

    let mut rlp = vec![RLP_NULL + 32];
    let number_bytes = [0x01; 32];
    rlp.extend(number_bytes);
    let decoded = U256::decode(&rlp).unwrap();
    let expected = U256::from_big_endian(&number_bytes);
    assert_eq!(decoded, expected);
}

#[test]
fn test_decode_string() {
    let rlp = vec![RLP_NULL + 3, b'd', b'o', b'g'];
    let decoded = String::decode(&rlp).unwrap();
    let expected = String::from("dog");
    assert_eq!(decoded, expected);

    let rlp = vec![RLP_NULL];
    let decoded = String::decode(&rlp).unwrap();
    let expected = String::from("");
    assert_eq!(decoded, expected);
}

#[test]
fn test_decode_lists() {
    // empty list
    let rlp = vec![RLP_EMPTY_LIST];
    let decoded: Vec<String> = Vec::decode(&rlp).unwrap();
    let expected: Vec<String> = vec![];
    assert_eq!(decoded, expected);

    //  list with a single number
    let rlp = vec![RLP_EMPTY_LIST + 1, 0x01];
    let decoded: Vec<u8> = Vec::decode(&rlp).unwrap();
    let expected = vec![1];
    assert_eq!(decoded, expected);

    // list with 3 numbers
    let rlp = vec![RLP_EMPTY_LIST + 3, 0x01, 0x02, 0x03];
    let decoded: Vec<u8> = Vec::decode(&rlp).unwrap();
    let expected = vec![1, 2, 3];
    assert_eq!(decoded, expected);

    // list of strings
    let rlp = vec![0xc8, 0x83, b'c', b'a', b't', 0x83, b'd', b'o', b'g'];
    let decoded: Vec<String> = Vec::decode(&rlp).unwrap();
    let expected = vec!["cat".to_string(), "dog".to_string()];
    assert_eq!(decoded, expected);
}

#[test]
fn test_decode_list_of_lists() {
    // list of lists of numbers
    let rlp = vec![
        RLP_EMPTY_LIST + 6,
        RLP_EMPTY_LIST + 2,
        0x01,
        0x02,
        RLP_EMPTY_LIST + 2,
        0x03,
        0x04,
    ];
    let decoded: Vec<Vec<u8>> = Vec::decode(&rlp).unwrap();
    let expected = vec![vec![1, 2], vec![3, 4]];
    assert_eq!(decoded, expected);

    // list of list of strings
    let rlp = vec![
        0xd2, 0xc8, 0x83, b'c', b'a', b't', 0x83, b'd', b'o', b'g', 0xc8, 0x83, b'f', b'o', b'o',
        0x83, b'b', b'a', b'r',
    ];
    let decoded: Vec<Vec<String>> = Vec::decode(&rlp).unwrap();
    let expected = vec![
        vec!["cat".to_string(), "dog".to_string()],
        vec!["foo".to_string(), "bar".to_string()],
    ];
    assert_eq!(decoded, expected);
}

#[test]
fn test_decode_tuples() {
    // tuple with numbers
    let rlp = vec![RLP_EMPTY_LIST + 2, 0x01, 0x02];
    let decoded: (u8, u8) = <(u8, u8)>::decode(&rlp).unwrap();
    let expected = (1, 2);
    assert_eq!(decoded, expected);

    // tuple with string and number
    let rlp = vec![RLP_EMPTY_LIST + 5, 0x01, 0x83, b'c', b'a', b't'];
    let decoded: (u8, String) = <(u8, String)>::decode(&rlp).unwrap();
    let expected = (1, "cat".to_string());
    assert_eq!(decoded, expected);

    // tuple with bool and string
    let rlp = vec![RLP_EMPTY_LIST + 6, 0x01, 0x84, b't', b'r', b'u', b'e'];
    let decoded: (bool, String) = <(bool, String)>::decode(&rlp).unwrap();
    let expected = (true, "true".to_string());
    assert_eq!(decoded, expected);

    // tuple with list and number
    let rlp = vec![RLP_EMPTY_LIST + 2, RLP_EMPTY_LIST, 0x03];
    let decoded = <(Vec<u8>, u8)>::decode(&rlp).unwrap();
    let expected = (vec![], 3);
    assert_eq!(decoded, expected);

    // tuple with number and list
    let rlp = vec![RLP_EMPTY_LIST + 2, 0x03, RLP_EMPTY_LIST];
    let decoded = <(u8, Vec<u8>)>::decode(&rlp).unwrap();
    let expected = (3, vec![]);
    assert_eq!(decoded, expected);

    // tuple with tuples
    let rlp = vec![
        RLP_EMPTY_LIST + 6,
        RLP_EMPTY_LIST + 2,
        0x01,
        0x02,
        RLP_EMPTY_LIST + 2,
        0x03,
        0x04,
    ];
    let decoded = <((u8, u8), (u8, u8))>::decode(&rlp).unwrap();
    let expected = ((1, 2), (3, 4));
    assert_eq!(decoded, expected);
}

#[test]
fn test_decode_tuples_3_elements() {
    // tuple with numbers
    let rlp = vec![RLP_EMPTY_LIST + 3, 0x01, 0x02, 0x03];
    let decoded: (u8, u8, u8) = <(u8, u8, u8)>::decode(&rlp).unwrap();
    let expected = (1, 2, 3);
    assert_eq!(decoded, expected);

    // tuple with string and number
    let rlp = vec![RLP_EMPTY_LIST + 6, 0x01, 0x02, 0x83, b'c', b'a', b't'];
    let decoded: (u8, u8, String) = <(u8, u8, String)>::decode(&rlp).unwrap();
    let expected = (1, 2, "cat".to_string());
    assert_eq!(decoded, expected);

    // tuple with bool and string
    let rlp = vec![RLP_EMPTY_LIST + 7, 0x01, 0x02, 0x84, b't', b'r', b'u', b'e'];
    let decoded: (u8, u8, String) = <(u8, u8, String)>::decode(&rlp).unwrap();
    let expected = (1, 2, "true".to_string());
    assert_eq!(decoded, expected);

    // tuple with tuples
    let rlp = vec![
        RLP_EMPTY_LIST + 9,
        RLP_EMPTY_LIST + 2,
        0x01,
        0x02,
        RLP_EMPTY_LIST + 2,
        0x03,
        0x04,
        RLP_EMPTY_LIST + 2,
        0x05,
        0x06,
    ];
    let decoded = <((u8, u8), (u8, u8), (u8, u8))>::decode(&rlp).unwrap();
    let expected = ((1, 2), (3, 4), (5, 6));
    assert_eq!(decoded, expected);
}

#[test]
fn test_decode_list_as_string() {
    // [1, 2, 3, 4] != 0x01020304
    let rlp = vec![RLP_EMPTY_LIST + 4, 0x01, 0x02, 0x03, 0x04];
    let decoded: Result<[u8; 4], _> = RLPDecode::decode(&rlp);
    // It should fail because a list is not a string
    assert!(decoded.is_err());

    // [1, 2] != 0x0102
    let rlp = vec![RLP_EMPTY_LIST + 2, 0x01, 0x02];
    let decoded: Result<u16, _> = RLPDecode::decode(&rlp);
    // It should fail because a list is not a string
    assert!(decoded.is_err());
}
