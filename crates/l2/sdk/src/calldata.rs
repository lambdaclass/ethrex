use ethrex_common::Bytes;
use ethrex_common::{Address, H32, U256};
use ethrex_rpc::clients::eth::errors::CalldataEncodeError;
use keccak_hash::keccak;

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
#[derive(Debug, PartialEq, Clone)]
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
    let params: Vec<String> = params
        .trim_end_matches(')')
        .split(',')
        .map(|x| x.trim().split_once(' ').unzip().0.unwrap_or(x).to_string())
        .collect();
    Ok((name.to_string(), params))
}

fn compute_function_selector(name: &str, params: &[String]) -> Result<H32, CalldataEncodeError> {
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
    if params
        .first()
        .ok_or(CalldataEncodeError::InternalError)?
        .is_empty()
    {
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
fn encode_tuple(values: &[Value]) -> Result<Vec<u8>, CalldataEncodeError> {
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
                    ret.extend_from_slice(&tuple_encoding);
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
                    ret.extend_from_slice(&fixed_array_encoding);
                } else {
                    write_u256(&mut ret, U256::from(current_dynamic_offset), current_offset)?;

                    let tuple_encoding = encode_tuple(fixed_array_values)?;
                    ret.extend_from_slice(&tuple_encoding);
                    current_dynamic_offset += tuple_encoding.len();
                }
            }
            Value::FixedBytes(bytes) => {
                let mut to_copy = [0; 32];
                to_copy.copy_from_slice(bytes);
                copy_into(&mut ret, &to_copy, current_offset, 32)?;
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
    let to_copy = U256::from(values.len()).to_big_endian();

    ret.extend_from_slice(&to_copy);
    ret.extend_from_slice(values);

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
