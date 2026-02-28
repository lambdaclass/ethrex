use ethrex_common::Bytes;
use ethrex_common::H256;
use ethrex_common::utils::{decode_hex, keccak};
use ethrex_common::{Address, H32, U256};
use ethrex_l2_common::calldata::Value;
use ethrex_rpc::clients::EthClientError;
use ethrex_rpc::clients::eth::errors::CalldataEncodeError;

use crate::address_to_word;

#[derive(Debug, thiserror::Error)]
pub enum CalldataDecodeError {
    #[error("Failed to parse function signature: {0}")]
    ParseError(String),
    #[error("Invalid calldata. Tried to read more bytes than there are.")]
    OutOfBounds,
    #[error("Internal Calldata decoding error. This is most likely a bug")]
    InternalError,
}

pub fn parse_signature(signature: &str) -> Result<(String, Vec<String>), CalldataEncodeError> {
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

#[doc(hidden)]
/// Computes the 4-byte function selector from a function name and parameters.
///
/// Exposed for testing - not part of the stable public API.
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

pub fn decode_calldata(signature: &str, data: Bytes) -> Result<Vec<Value>, CalldataDecodeError> {
    let (_, params) =
        parse_signature(signature).map_err(|e| CalldataDecodeError::ParseError(e.to_string()))?;
    let mut decoder = DecodeHelper::new(&data);
    let datatype = DataType::Tuple(
        params
            .iter()
            .map(|v| DataType::parse(v))
            .collect::<Result<Vec<_>, _>>()?,
    );
    match datatype.decode(&mut decoder)? {
        Value::Tuple(values) => Ok(values),
        _ => Err(CalldataDecodeError::InternalError),
    }
}

struct DecodeHelper<'a> {
    buf: &'a [u8],
    index: usize,
}

const SELECTOR_SIZE: usize = 4;

impl<'a> DecodeHelper<'a> {
    fn new(buf: &'a [u8]) -> Self {
        DecodeHelper {
            buf,
            index: SELECTOR_SIZE,
        }
    }
    fn consume(&mut self, n: usize) -> Result<&'a [u8], CalldataDecodeError> {
        let data = self
            .buf
            .get(self.index..self.index + n)
            .ok_or(CalldataDecodeError::OutOfBounds)?;
        self.index += n;
        Ok(data)
    }
    fn consume_u256(&mut self) -> Result<U256, CalldataDecodeError> {
        Ok(U256::from_big_endian(self.consume(32)?))
    }
    fn start_reading_at(&self, offset: usize) -> Result<Self, CalldataDecodeError> {
        let data = self
            .buf
            .get(self.index + offset..)
            .ok_or(CalldataDecodeError::OutOfBounds)?;
        Ok(DecodeHelper {
            buf: data,
            index: 0,
        })
    }
}

#[derive(Clone, Debug)]
enum DataType {
    Array(Box<DataType>),
    FixedArray(usize, Box<DataType>),
    Tuple(Vec<DataType>),
    Bytes,
    FixedBytes(usize),
    Address,
    Bool,
    Uint,
    Int,
}

impl DataType {
    fn parse(param: &str) -> Result<Self, CalldataDecodeError> {
        Ok(match param {
            _ if param.ends_with("[]") => {
                let inner = param
                    .strip_suffix("[]")
                    .ok_or(CalldataDecodeError::InternalError)?;
                DataType::Array(Box::new(DataType::parse(inner)?))
            }
            _ if param.ends_with("]") => {
                let mut n = String::new();
                let mut iter = param.chars().rev().skip(1);
                for c in iter.by_ref() {
                    if c.is_ascii_digit() {
                        n.insert(0, c);
                    } else {
                        if c != '[' {
                            return Err(CalldataDecodeError::ParseError(format!(
                                "expected ] but found {c}"
                            )));
                        }
                        break;
                    }
                }
                iter.next();
                let inner: String = iter.collect::<String>().chars().rev().collect();
                let n: usize = n.parse().map_err(|_| CalldataDecodeError::OutOfBounds)?;
                DataType::FixedArray(n, Box::new(DataType::parse(&inner)?))
            }
            _ if param.ends_with(")") => {
                let (_, inner) = parse_signature(param)
                    .map_err(|e| CalldataDecodeError::ParseError(e.to_string()))?;
                DataType::Tuple(
                    inner
                        .iter()
                        .map(|v| DataType::parse(v))
                        .collect::<Result<Vec<_>, _>>()?,
                )
            }
            "address" => DataType::Address,
            "bool" => DataType::Bool,
            "bytes" => DataType::Bytes,
            _ if param.starts_with("bytes") => {
                let n = param
                    .trim_start_matches("bytes")
                    .parse()
                    .map_err(|_| CalldataDecodeError::ParseError("invalid bytesN".to_string()))?;
                DataType::FixedBytes(n)
            }
            _ if param.starts_with("uint") => DataType::Uint,
            _ if param.starts_with("int") => DataType::Int,
            _ => {
                return Err(CalldataDecodeError::ParseError(format!(
                    "unknown type {param}"
                )));
            }
        })
    }
    fn is_dynamic(&self) -> bool {
        match self {
            DataType::Array(_) => true,
            DataType::Bytes => true,
            DataType::FixedArray(_, inner) => inner.is_dynamic(),
            DataType::Tuple(inner) => inner.iter().any(|t| t.is_dynamic()),
            _ => false,
        }
    }
    fn decode(&self, data: &mut DecodeHelper) -> Result<Value, CalldataDecodeError> {
        Ok(match self {
            DataType::Uint => Value::Uint(data.consume_u256()?),
            DataType::Int => Value::Int(data.consume_u256()?),
            DataType::Address => {
                data.consume(32 - 20)?;
                Value::Address(Address::from_slice(data.consume(20)?))
            }
            DataType::Bool => Value::Bool(!data.consume_u256()?.is_zero()),
            DataType::FixedBytes(n) => Value::FixedBytes(
                data.consume(32)?
                    .get(0..*n)
                    .ok_or(CalldataDecodeError::OutOfBounds)?
                    .to_vec()
                    .into(),
            ),
            DataType::Bytes => {
                let n: usize = data
                    .consume_u256()?
                    .try_into()
                    .map_err(|_| CalldataDecodeError::OutOfBounds)?;
                let size = if n.is_multiple_of(32) {
                    n
                } else {
                    n.next_multiple_of(32)
                };
                Value::Bytes(
                    data.consume(size)?
                        .get(0..n)
                        .ok_or(CalldataDecodeError::OutOfBounds)?
                        .to_vec()
                        .into(),
                )
            }
            DataType::FixedArray(n, inner_type) => {
                let inner_type = *inner_type.clone();
                let value = DataType::Tuple(vec![inner_type; *n]).decode(data)?;
                match value {
                    Value::Tuple(inner) => Value::FixedArray(inner),
                    _ => return Err(CalldataDecodeError::InternalError),
                }
            }
            DataType::Tuple(inner_types) => {
                let mut values = Vec::new();
                let start_reader = data.start_reading_at(0)?;
                for inner_type in inner_types {
                    if inner_type.is_dynamic() {
                        let offset: usize = data
                            .consume_u256()?
                            .try_into()
                            .map_err(|_| CalldataDecodeError::OutOfBounds)?;
                        values
                            .push(inner_type.decode(&mut start_reader.start_reading_at(offset)?)?);
                    } else {
                        values.push(inner_type.decode(data)?);
                    }
                }
                Value::Tuple(values)
            }
            DataType::Array(inner_type) => {
                let n: usize = data
                    .consume_u256()?
                    .try_into()
                    .map_err(|_| CalldataDecodeError::OutOfBounds)?;
                let mut values = Vec::new();
                for _ in 0..n {
                    values.push(inner_type.decode(data)?);
                }
                Value::Array(values)
            }
        })
    }
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

#[allow(clippy::indexing_slicing)]
pub fn from_hex_string_to_h256_array(hex_string: &str) -> Result<Vec<H256>, EthClientError> {
    let bytes = decode_hex(hex_string)
        .map_err(|_| EthClientError::Custom("Invalid hex string".to_owned()))?;

    // The ABI encoding for dynamic arrays is:
    // 1. Offset to data (32 bytes)
    // 2. Length of array (32 bytes)
    // 3. Array elements (each 32 bytes)
    if bytes.len() < 64 {
        return Err(EthClientError::Custom("Response too short".to_owned()));
    }

    // Get the offset (should be 0x20 for simple arrays)
    let offset = U256::from_big_endian(&bytes[0..32]).as_usize();

    // Get the length of the array
    let length = U256::from_big_endian(&bytes[offset..offset + 32]).as_usize();

    // Calculate the start of the array data
    let data_start = offset + 32;
    let data_end = data_start + (length * 32);

    if data_end > bytes.len() {
        return Err(EthClientError::Custom("Invalid array length".to_owned()));
    }

    // Convert the slice directly to H256 array
    bytes[data_start..data_end]
        .chunks_exact(32)
        .map(|chunk| Ok(H256::from_slice(chunk)))
        .collect()
}
