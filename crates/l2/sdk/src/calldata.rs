use ethrex_core::Bytes;
use ethrex_core::{Address, H256, H32, U256};
use eyre::eyre;
use keccak_hash::keccak;
use std::str::FromStr;

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

fn parse_signature(signature: &str) -> (String, Vec<String>) {
    let sig = signature.trim().trim_start_matches("function ");
    let (name, params) = sig.split_once('(').unwrap();
    let params: Vec<String> = params
        .trim_end_matches(')')
        .split(',')
        .map(|x| x.trim().split_once(' ').unzip().0.unwrap_or(x).to_string())
        .collect();
    (name.to_string(), params)
}

fn compute_function_selector(name: &str, params: &[String]) -> H32 {
    let normalized_signature = format!("{name}({})", params.join(","));
    let hash = keccak(normalized_signature.as_bytes());

    H32::from(&hash[..4].try_into().unwrap())
}

pub fn encode_calldata(signature: &str, values: &[Value]) -> Result<Vec<u8>, eyre::Error> {
    let (name, params) = parse_signature(signature);

    if params.len() != values.len() {
        return Err(eyre!(
            "Number of arguments does not match ({} != {})",
            params.len(),
            values.len()
        ));
    }

    let function_selector = compute_function_selector(&name, &params);
    let calldata = encode_tuple(&values);
    let mut with_selector = function_selector.as_bytes().to_vec();

    with_selector.extend_from_slice(&calldata);

    Ok(with_selector)
}

/*
    TODO: Explain this function.
*/
fn encode_tuple(values: &[Value]) -> Vec<u8> {
    let mut current_offset = 0;
    let mut current_dynamic_offset = 0;
    for value in values {
        current_dynamic_offset += static_offset_value(value);
    }

    let mut ret = vec![0; current_dynamic_offset];

    for value in values {
        match value {
            Value::Address(h160) => {
                write_u256(&mut ret, address_to_word(*h160), current_offset);
            }
            Value::Uint(u256) => {
                write_u256(&mut ret, *u256, current_offset);
            }
            Value::Int(u256) => {
                write_u256(&mut ret, *u256, current_offset);
            }
            Value::Bool(boolean) => {
                write_u256(&mut ret, U256::from(u8::from(*boolean)), current_offset);
            }
            Value::Bytes(bytes) => {
                write_u256(&mut ret, U256::from(current_dynamic_offset), current_offset);

                let bytes_encoding = encode_bytes(&bytes);
                ret.extend_from_slice(&bytes_encoding);
                current_dynamic_offset += bytes_encoding.len();
            }
            Value::String(string_value) => {
                write_u256(&mut ret, U256::from(current_dynamic_offset), current_offset);

                let utf8_encoded = Bytes::copy_from_slice(string_value.as_bytes());
                let bytes_encoding = encode_bytes(&utf8_encoded);
                ret.extend_from_slice(&bytes_encoding);
                current_dynamic_offset += bytes_encoding.len();
            }
            Value::Array(array_values) => {
                write_u256(&mut ret, U256::from(current_dynamic_offset), current_offset);

                let array_encoding = encode_array(&array_values);
                ret.extend_from_slice(&array_encoding);
                current_dynamic_offset += array_encoding.len();
            }
            Value::Tuple(tuple_values) => {
                if !is_dynamic(value) {
                    let tuple_encoding = encode_tuple(&tuple_values);
                    ret.extend_from_slice(&tuple_encoding);
                } else {
                    write_u256(&mut ret, U256::from(current_dynamic_offset), current_offset);

                    let tuple_encoding = encode_tuple(&tuple_values);
                    ret.extend_from_slice(&tuple_encoding);
                    current_dynamic_offset += tuple_encoding.len();
                }
            }
            Value::FixedArray(fixed_array_values) => {
                if !is_dynamic(value) {
                    let fixed_array_encoding = encode_tuple(&fixed_array_values);
                    ret.extend_from_slice(&fixed_array_encoding);
                } else {
                    write_u256(&mut ret, U256::from(current_dynamic_offset), current_offset);

                    let tuple_encoding = encode_tuple(&fixed_array_values);
                    ret.extend_from_slice(&tuple_encoding);
                    current_dynamic_offset += tuple_encoding.len();
                }
            }
            Value::FixedBytes(bytes) => {
                let mut to_copy = [0; 32];
                to_copy.copy_from_slice(&bytes);
                copy_into(&mut ret, &to_copy, current_offset, 32);
            }
        }

        current_offset += static_offset_value(value);
    }

    ret
}

fn write_u256(values: &mut [u8], number: U256, offset: usize) {
    let mut to_copy = [0; 32];
    number.to_big_endian(&mut to_copy);
    copy_into(values, &to_copy, offset, 32);
}

fn static_offset_value(value: &Value) -> usize {
    let mut ret = 0;

    match value {
        Value::Address(_) => ret += 32,
        Value::Uint(_) => ret += 32,
        Value::Int(_) => ret += 32,
        Value::Bool(_) => ret += 32,
        Value::Bytes(_) => ret += 32,
        Value::String(_) => ret += 32,
        Value::Array(_) => ret += 32,
        Value::Tuple(vec) => {
            if is_dynamic(value) {
                ret += 32;
            } else {
                for element in vec {
                    // Here every element is guaranteed to be static, otherwise we would not be
                    // in the `else` branch of the `if` statement.
                    ret += static_offset_value(&element);
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
                    ret += static_offset_value(&element);
                }
            }
        }
        Value::FixedBytes(_) => ret += 32,
    }

    ret
}

fn is_dynamic(value: &Value) -> bool {
    let mut result = false;
    match value {
        Value::Bytes(_) => result = true,
        Value::String(_) => result = true,
        Value::Array(_) => result = true,
        Value::Tuple(vec) => {
            for value in vec {
                if is_dynamic(value) {
                    result = true;
                }
            }
        }
        Value::FixedArray(vec) => {
            result = is_dynamic(vec.first().unwrap());
        }
        _ => {}
    }

    result
}

fn encode_array(values: &[Value]) -> Vec<u8> {
    let mut ret = vec![];
    let mut to_copy = [0; 32];
    U256::from(values.len()).to_big_endian(&mut to_copy);
    ret.extend_from_slice(&to_copy);

    let tuple_encoding = encode_tuple(values);
    ret.extend_from_slice(&tuple_encoding);

    ret
}

fn encode_bytes(values: &Bytes) -> Vec<u8> {
    let mut ret = vec![];
    let mut to_copy = [0; 32];
    U256::from(values.len()).to_big_endian(&mut to_copy);

    ret.extend_from_slice(&to_copy);
    ret.extend_from_slice(&values);

    ret
}

fn copy_into(values: &mut [u8], to_copy: &[u8], offset: usize, size: usize) {
    for i in 0..size {
        values[offset + i] = to_copy[i]
    }
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

    let mut bytes: [u8; 32] = [0; 32];
    U256::zero().to_big_endian(&mut bytes);

    bytes_calldata.extend_from_slice(&bytes);
    U256::one().to_big_endian(&mut bytes);
    bytes_calldata.extend_from_slice(&mut bytes);

    let arguments = vec![
        Value::Uint(U256::from(902)),
        Value::Bytes(bytes_calldata.into()).into(),
    ];

    let calldata = encode_calldata(&raw_function_signature, &arguments).unwrap();

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
