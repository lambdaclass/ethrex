use ethrex_common::Bytes;
use ethrex_common::{Address, H32, U256};
use ethrex_rpc::clients::eth::errors::CalldataEncodeError;
use keccak_hash::keccak;
use serde::{Deserialize, Serialize};

/// Struct representing the possible solidity types for function arguments
/// - `Uint` -> `uint256`
/// - `Address` -> `address`
/// - `Bool` -> `bool`
/// - `Bytes` -> `bytes`
/// - `String` -> `string`
/// - `Array` -> `T[]`
/// - `Tuple` -> `(X_1, ..., X_k)`
/// - `FixedArray` -> `T[k]`
/// - `FixedBytes` -> `bytesN`
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub enum Value {
    Address(Address),
    Uint(U256),
    Int(U256),
    Bool(bool),
    Bytes(Bytes),
    String(String),
    Array(Vec<Value>),
    Tuple(Vec<Value>),
    FixedArray(Vec<Value>),
    FixedBytes(Bytes),
}

fn parse_signature(signature: &str) -> Result<(String, Vec<String>), CalldataEncodeError> {
    let sig = signature.trim().trim_start_matches("function ");
    let (name, params) = sig
        .split_once('(')
        .ok_or(CalldataEncodeError::ParseError(signature.to_owned()))?;
    let params = params.rsplit_once(')').map_or(params, |(left, _)| left);

    // We use this to only keep track of top level tuples
    // "address,(uint256,uint256)" -> "address" and "(uint256,uint256)"
    // "address,(unit256,(uint256,uint256))" -> "address" and "(unit256,(uint256,uint256))"
    let mut splitted_params = Vec::new();
    let mut current_param = String::new();
    let mut parenthesis_depth = 0;

    for ch in params.chars() {
        match ch {
            '(' => {
                parenthesis_depth += 1;
                current_param.push(ch);
            }
            ')' => {
                parenthesis_depth -= 1;
                current_param.push(ch);
            }
            ',' if parenthesis_depth == 0 => {
                if !current_param.is_empty() {
                    splitted_params.push(current_param.trim().to_string());
                    current_param = String::new();
                }
            }
            _ => current_param.push(ch),
        }
    }

    // push the last param if it exists
    if !current_param.is_empty() {
        splitted_params.push(current_param.trim().to_string());
    }

    Ok((name.to_string(), splitted_params))
}

pub fn compute_function_selector(
    name: &str,
    params: &[String],
) -> Result<H32, CalldataEncodeError> {
    let normalized_signature = format!("{name}({})", params.join(","));
    let hash = keccak(normalized_signature.as_bytes());

    Ok(H32::from(&hash[..4].try_into().map_err(|_| {
        CalldataEncodeError::ParseError(name.to_owned())
    })?))
}

pub fn encode_calldata(signature: &str, values: &[Value]) -> Result<Vec<u8>, CalldataEncodeError> {
    let (name, params) = parse_signature(signature)?;

    // Checks if params = [""]
    // that case happen when we have a function selector as follows: function name()
    let mut params = params;
    if params.is_empty() {
        params = vec![];
    }

    if params.len() != values.len() {
        return Err(CalldataEncodeError::WrongArgumentLength(
            signature.to_owned(),
        ));
    }

    let function_selector = compute_function_selector(&name, &params)?;
    let calldata = encode_tuple(values)?;
    let mut with_selector = function_selector.as_bytes().to_vec();

    with_selector.extend_from_slice(&calldata);

    Ok(with_selector)
}

// This is the main entrypoint for ABI encoding solidity function arguments, as the list of arguments themselves are
// considered a tuple. Before going through this function, read the solidity ABI spec first
// https://docs.soliditylang.org/en/develop/abi-spec.html.
// The encoding of a tuple consists of two parts: a static and a dynamic one (what the spec calls the head and tail of the encoding).
// The dynamic part always follows at the end of the static one.
// Arguments are encoded in order. If the argument is static, it is encoded in place, i.e, there's no dynamic part.
// If the argument is dynamic, only its offset to the dynamic part is recorded on the static sector.
pub fn encode_tuple(values: &[Value]) -> Result<Vec<u8>, CalldataEncodeError> {
    let mut current_offset = 0;
    let mut current_dynamic_offset = 0;
    for value in values {
        current_dynamic_offset += static_offset_value(value);
    }

    let mut ret = vec![0; current_dynamic_offset];

    for value in values {
        match value {
            Value::Address(h160) => {
                write_u256(&mut ret, address_to_word(*h160), current_offset)?;
            }
            Value::Uint(u256) => {
                write_u256(&mut ret, *u256, current_offset)?;
            }
            Value::Int(u256) => {
                write_u256(&mut ret, *u256, current_offset)?;
            }
            Value::Bool(boolean) => {
                write_u256(&mut ret, U256::from(u8::from(*boolean)), current_offset)?;
            }
            Value::Bytes(bytes) => {
                write_u256(&mut ret, U256::from(current_dynamic_offset), current_offset)?;

                let bytes_encoding = encode_bytes(bytes);
                ret.extend_from_slice(&bytes_encoding);
                current_dynamic_offset += bytes_encoding.len();
            }
            Value::String(string_value) => {
                write_u256(&mut ret, U256::from(current_dynamic_offset), current_offset)?;

                let utf8_encoded = Bytes::copy_from_slice(string_value.as_bytes());
                let bytes_encoding = encode_bytes(&utf8_encoded);
                ret.extend_from_slice(&bytes_encoding);
                current_dynamic_offset += bytes_encoding.len();
            }
            Value::Array(array_values) => {
                write_u256(&mut ret, U256::from(current_dynamic_offset), current_offset)?;

                let array_encoding = encode_array(array_values)?;
                ret.extend_from_slice(&array_encoding);
                current_dynamic_offset += array_encoding.len();
            }
            Value::Tuple(tuple_values) => {
                if !is_dynamic(value) {
                    let tuple_encoding = encode_tuple(tuple_values)?;
                    copy_into(
                        &mut ret,
                        &tuple_encoding,
                        current_offset,
                        tuple_encoding.len(),
                    )?;
                } else {
                    write_u256(&mut ret, U256::from(current_dynamic_offset), current_offset)?;

                    let tuple_encoding = encode_tuple(tuple_values)?;
                    ret.extend_from_slice(&tuple_encoding);
                    current_dynamic_offset += tuple_encoding.len();
                }
            }
            Value::FixedArray(fixed_array_values) => {
                if !is_dynamic(value) {
                    let fixed_array_encoding = encode_tuple(fixed_array_values)?;
                    copy_into(
                        &mut ret,
                        &fixed_array_encoding,
                        current_offset,
                        fixed_array_encoding.len(),
                    )?;
                } else {
                    write_u256(&mut ret, U256::from(current_dynamic_offset), current_offset)?;

                    let tuple_encoding = encode_tuple(fixed_array_values)?;
                    ret.extend_from_slice(&tuple_encoding);
                    current_dynamic_offset += tuple_encoding.len();
                }
            }
            Value::FixedBytes(bytes) => {
                let mut bytes = bytes.to_vec();
                bytes.resize(32, 0);
                copy_into(&mut ret, &bytes, current_offset, 32)?;
            }
        }

        current_offset += static_offset_value(value);
    }

    Ok(ret)
}

fn write_u256(values: &mut [u8], number: U256, offset: usize) -> Result<(), CalldataEncodeError> {
    let to_copy = number.to_big_endian();
    copy_into(values, &to_copy, offset, 32)?;

    Ok(())
}

// Returns the size that the value occupies in the static sector of the abi encoding.
// For dynamic types, this is always 32 (the offset to the dynamic sector).
// For static types, it's 32 unless the value is a static tuple or a fixed array, in which case
// it's the sum of the sizes of their elements.
fn static_offset_value(value: &Value) -> usize {
    let mut ret = 0;

    match value {
        Value::Address(_)
        | Value::Uint(_)
        | Value::Int(_)
        | Value::Bool(_)
        | Value::Bytes(_)
        | Value::String(_)
        | Value::Array(_)
        | Value::FixedBytes(_) => ret += 32,
        Value::Tuple(vec) => {
            if is_dynamic(value) {
                ret += 32;
            } else {
                for element in vec {
                    // Here every element is guaranteed to be static, otherwise we would not be
                    // in the `else` branch of the `if` statement.
                    ret += static_offset_value(element);
                }
            }
        }
        Value::FixedArray(vec) => {
            if is_dynamic(value) {
                ret += 32;
            } else {
                for element in vec {
                    // Here every element is guaranteed to be static (and of the same type), otherwise we would not be
                    // in the `else` branch of the `if` statement.
                    ret += static_offset_value(element);
                }
            }
        }
    }

    ret
}

fn is_dynamic(value: &Value) -> bool {
    match value {
        Value::Bytes(_) | Value::String(_) | Value::Array(_) => true,
        Value::Tuple(vec) => vec.iter().any(is_dynamic),
        Value::FixedArray(vec) => {
            if let Some(first_elem) = vec.first() {
                is_dynamic(first_elem)
            } else {
                false
            }
        }
        _ => false,
    }
}

fn encode_array(values: &[Value]) -> Result<Vec<u8>, CalldataEncodeError> {
    let mut ret = vec![];
    let to_copy = U256::from(values.len()).to_big_endian();
    ret.extend_from_slice(&to_copy);

    let tuple_encoding = encode_tuple(values)?;
    ret.extend_from_slice(&tuple_encoding);

    Ok(ret)
}

fn encode_bytes(values: &Bytes) -> Vec<u8> {
    let mut ret = vec![];

    // the bytes has to be padded to 32 bytes
    let padding = 32 - (values.len() % 32);
    let mut padded_bytes = values.to_vec();
    if padding != 32 {
        padded_bytes.extend_from_slice(&vec![0; padding]);
    }

    let to_copy = U256::from(values.len()).to_big_endian(); // we write the length without padding

    ret.extend_from_slice(&to_copy);
    ret.extend_from_slice(&padded_bytes);

    ret
}

fn copy_into(
    values: &mut [u8],
    to_copy: &[u8],
    offset: usize,
    size: usize,
) -> Result<(), CalldataEncodeError> {
    let to_copy_slice = to_copy
        .get(..size)
        .ok_or(CalldataEncodeError::InternalError)?;

    values
        .get_mut(offset..(size + offset))
        .ok_or(CalldataEncodeError::InternalError)?
        .copy_from_slice(to_copy_slice);

    Ok(())
}

fn address_to_word(address: Address) -> U256 {
    let mut word = [0u8; 32];
    for (word_byte, address_byte) in word.iter_mut().skip(12).zip(address.as_bytes().iter()) {
        *word_byte = *address_byte;
    }
    U256::from_big_endian(&word)
}

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
}

#[test]
fn bytes_has_padding() {
    let raw_function_signature = "my_function(bytes)";
    let bytes = Bytes::from_static(b"hello world");
    let values = vec![Value::Bytes(bytes)];

    let calldata = encode_calldata(raw_function_signature, &values).unwrap();

    assert_eq!(calldata, hex::decode("f570899b0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000b68656c6c6f20776f726c64000000000000000000000000000000000000000000").unwrap());
}
