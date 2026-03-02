use ethrex_common::{Address, Bytes, H32, U256};
use ethrex_l2_common::calldata::Value;
use ethrex_l2_sdk::calldata::{
    compute_function_selector, decode_calldata, encode_calldata, encode_tuple, parse_signature,
};

#[test]
fn fixed_array_encoding_test() {
    use bytes::{BufMut, BytesMut};
    let raw_function_signature = "test(uint256,bytes,bytes32,bytes,bytes32,bytes,bytes,bytes32,bytes,uint256[8],bytes,bytes)";
    let bytes_calldata: [u8; 32] = [0; 32];

    let mut buf = BytesMut::new();
    buf.put_u8(0x12);
    buf.put_u8(0x34);

    let a = buf.freeze();

    let fixed_array = vec![
        Value::Uint(U256::from(4)),
        Value::Uint(U256::from(3)),
        Value::Uint(U256::from(2)),
        Value::Uint(U256::from(1)),
        Value::Uint(U256::from(8)),
        Value::Uint(U256::from(9)),
        Value::Uint(U256::from(1)),
        Value::Uint(U256::from(0)),
    ];

    let arguments = vec![
        Value::Uint(U256::from(1)),
        Value::Bytes(a.clone()),
        Value::FixedBytes(bytes_calldata.to_vec().into()),
        Value::Bytes(a.clone()),
        Value::FixedBytes(bytes_calldata.to_vec().into()),
        Value::Bytes(Bytes::new()),
        Value::Bytes(a.clone()),
        Value::FixedBytes(bytes_calldata.to_vec().into()),
        Value::Bytes(Bytes::new()),
        Value::FixedArray(fixed_array),
        Value::Bytes(Bytes::new()),
        Value::Bytes(a),
    ];

    let calldata = encode_calldata(raw_function_signature, &arguments).unwrap();
    let decoded = decode_calldata(raw_function_signature, calldata.clone().into()).unwrap();
    assert_eq!(arguments, decoded);
    let expected_calldata = hex::decode("ac0f26b000000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000260000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002a0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002e0000000000000000000000000000000000000000000000000000000000000030000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000340000000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000000000000000000000000000030000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000009000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000360000000000000000000000000000000000000000000000000000000000000038000000000000000000000000000000000000000000000000000000000000000021234000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000212340000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000212340000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000021234000000000000000000000000000000000000000000000000000000000000").unwrap();

    assert_eq!(calldata, expected_calldata);
}

#[test]
fn calldata_test() {
    let raw_function_signature = "blockWithdrawalsLogs(uint256,bytes)";
    let mut bytes_calldata = vec![];

    bytes_calldata.extend_from_slice(&U256::zero().to_big_endian());
    bytes_calldata.extend_from_slice(&U256::one().to_big_endian());

    let arguments = vec![
        Value::Uint(U256::from(902)),
        Value::Bytes(bytes_calldata.into()),
    ];

    let calldata = encode_calldata(raw_function_signature, &arguments).unwrap();

    let decoded = decode_calldata(raw_function_signature, calldata.clone().into()).unwrap();
    assert_eq!(arguments, decoded);
    assert_eq!(
        calldata,
        vec![
            20, 108, 34, 199, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 3, 134, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 64, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 64, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
        ]
    );
}

#[test]
fn raw_function_selector() {
    let raw_function_signature = "deposit((address,address,uint256,bytes))";

    let (name, params) = parse_signature(raw_function_signature).unwrap();
    let selector = compute_function_selector(&name, &params).unwrap();

    assert_eq!(selector, H32::from(&[0x02, 0xe8, 0x6b, 0xbe]));
}

#[test]
fn encode_tuple_dynamic_offset() {
    let raw_function_signature = "deposit((address,address,uint256,bytes))";
    let address = Address::from_low_u64_be(424242_u64);

    let tuple = Value::Tuple(vec![
        Value::Address(address),
        Value::Address(address),
        Value::Uint(U256::from(21000 * 5)),
        Value::Bytes(Bytes::from_static(b"")),
    ]);
    let values = vec![tuple];

    let calldata = encode_calldata(raw_function_signature, &values).unwrap();
    let decoded = decode_calldata(raw_function_signature, calldata.clone().into()).unwrap();
    assert_eq!(values, decoded);

    assert_eq!(calldata, hex::decode("02e86bbe0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000006793200000000000000000000000000000000000000000000000000000000000679320000000000000000000000000000000000000000000000000000000000019a2800000000000000000000000000000000000000000000000000000000000000800000000000000000000000000000000000000000000000000000000000000000").unwrap());

    let mut encoding = vec![0x02, 0xe8, 0x6b, 0xbe]; // function selector
    encoding.extend_from_slice(&encode_tuple(&values).unwrap());

    assert_eq!(calldata, encoding);
}

#[test]
fn correct_tuple_parsing() {
    // the arguments are:
    // - uint256
    // - (uint256, address)
    // - ((address, address), (uint256, bytes))
    // - ((address, address), uint256)
    // - (uint256, (address, address))
    // - address
    let raw_function_signature = "my_function(uint256,(uin256,address),((address,address),(uint256,bytes)),((address,address),uint256),(uint256,(address,address)),address)";

    let exepected_arguments: Vec<String> = vec![
        "uint256".to_string(),
        "(uin256,address)".to_string(),
        "((address,address),(uint256,bytes))".to_string(),
        "((address,address),uint256)".to_string(),
        "(uint256,(address,address))".to_string(),
        "address".to_string(),
    ];
    let (name, params) = parse_signature(raw_function_signature).unwrap();
    assert_eq!(name, "my_function");
    assert_eq!(params, exepected_arguments);
}

#[test]
fn empty_calldata() {
    let calldata = encode_calldata("number()", &[]).unwrap();
    assert_eq!(calldata, hex::decode("8381f58a").unwrap());
    let decoded = decode_calldata("number()", calldata.into()).unwrap();
    assert!(decoded.is_empty());
}

#[test]
fn bytes_has_padding() {
    let raw_function_signature = "my_function(bytes)";
    let bytes = Bytes::from_static(b"hello world");
    let values = vec![Value::Bytes(bytes)];

    let calldata = encode_calldata(raw_function_signature, &values).unwrap();

    assert_eq!(calldata, hex::decode("f570899b0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000b68656c6c6f20776f726c64000000000000000000000000000000000000000000").unwrap());
    let decoded = decode_calldata(raw_function_signature, calldata.into()).unwrap();
    assert_eq!(values, decoded);
}
