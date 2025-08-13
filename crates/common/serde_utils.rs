use std::time::Duration;

use serde::{Deserialize, Deserializer, Serializer, de::Error, ser::SerializeSeq};

pub mod u256 {
    use super::*;
    use ethereum_types::U256;
    use serde_json::Number;

    pub mod dec_str {
        use super::*;
        pub fn deserialize<'de, D>(d: D) -> Result<U256, D::Error>
        where
            D: Deserializer<'de>,
        {
            let value = String::deserialize(d)?;
            U256::from_dec_str(&value).map_err(|e| D::Error::custom(e.to_string()))
        }

        pub fn serialize<S>(value: &U256, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serializer.serialize_str(&value.to_string())
        }
    }

    pub fn deser_number<'de, D>(d: D) -> Result<U256, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Number::deserialize(d)?.to_string();
        U256::from_dec_str(&value).map_err(|e| D::Error::custom(e.to_string()))
    }

    pub fn deser_number_opt<'de, D>(d: D) -> Result<Option<U256>, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Handle the null case explicitly
        let opt = Option::<Number>::deserialize(d)?;
        match opt {
            Some(number) => {
                // Convert number to string and parse to U256
                let value = number.to_string();
                U256::from_dec_str(&value)
                    .map(Some)
                    .map_err(|e| D::Error::custom(e.to_string()))
            }
            None => Ok(None),
        }
    }

    pub fn deser_hex_str<'de, D>(d: D) -> Result<U256, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(d)?;
        U256::from_str_radix(value.trim_start_matches("0x"), 16)
            .map_err(|_| D::Error::custom("Failed to deserialize u256 value"))
    }

    pub fn deser_hex_str_opt<'de, D>(d: D) -> Result<Option<U256>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = Option::<String>::deserialize(d)?;
        match s {
            Some(s) => U256::from_str_radix(s.trim_start_matches("0x"), 16)
                .map_err(|_| D::Error::custom("Failed to deserialize u256 value"))
                .map(Some),
            None => Ok(None),
        }
    }

    pub fn deser_hex_or_dec_str<'de, D>(d: D) -> Result<U256, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(d)?;
        if value.starts_with("0x") {
            U256::from_str_radix(value.trim_start_matches("0x"), 16)
                .map_err(|_| D::Error::custom("Failed to deserialize u256 value"))
        } else {
            U256::from_dec_str(&value).map_err(|e| D::Error::custom(e.to_string()))
        }
    }

    pub fn serialize_number<S>(value: &u64, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&value.to_string())
    }

    pub mod vec {
        use super::*;
        use serde::de::IntoDeserializer;
        use serde::{Deserialize, Deserializer};

        pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<U256>, D::Error>
        where
            D: Deserializer<'de>,
        {
            let raw_vec = Vec::<String>::deserialize(deserializer)?;
            raw_vec
                .into_iter()
                .map(|s| {
                    let deser = s.into_deserializer();
                    super::deser_hex_or_dec_str(deser)
                })
                .collect()
        }
    }

    pub mod hashmap {
        use super::*;
        use serde::de::IntoDeserializer;
        use serde::{Deserialize, Deserializer};
        use std::collections::HashMap;

        pub fn deserialize<'de, D>(deserializer: D) -> Result<HashMap<U256, U256>, D::Error>
        where
            D: Deserializer<'de>,
        {
            let raw_map = HashMap::<String, String>::deserialize(deserializer)?;
            raw_map
                .into_iter()
                .map(|(k, v)| {
                    let key_deser = k.into_deserializer();
                    let val_deser = v.into_deserializer();

                    let key = super::deser_hex_or_dec_str(key_deser)?;
                    let value = super::deser_hex_or_dec_str(val_deser)?;
                    Ok((key, value))
                })
                .collect()
        }
    }
}

pub mod u64 {
    use serde::de::IntoDeserializer;

    use super::*;

    pub mod hex_str {
        use super::*;

        pub fn deserialize<'de, D>(d: D) -> Result<u64, D::Error>
        where
            D: Deserializer<'de>,
        {
            let value = String::deserialize(d)?;
            u64::from_str_radix(value.trim_start_matches("0x"), 16)
                .map_err(|_| D::Error::custom("Failed to deserialize u64 value"))
        }

        pub fn deser_vec<'de, D>(deserializer: D) -> Result<Vec<u64>, D::Error>
        where
            D: Deserializer<'de>,
        {
            let raw_vec = Vec::<String>::deserialize(deserializer)?;
            raw_vec
                .into_iter()
                .map(|s| {
                    let deser = s.into_deserializer();
                    deserialize(deser)
                })
                .collect()
        }

        pub fn serialize<S>(value: &u64, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serializer.serialize_str(&format!("{value:#x}"))
        }
    }
    pub mod hex_str_padding {
        use super::*;

        pub fn deserialize<'de, D>(d: D) -> Result<u64, D::Error>
        where
            D: Deserializer<'de>,
        {
            super::hex_str::deserialize(d)
        }

        pub fn serialize<S>(value: &u64, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serializer.serialize_str(&format!("{value:#018x}"))
        }
    }

    pub mod hex_str_opt {
        use serde::Serialize;

        use super::*;

        pub fn serialize<S>(value: &Option<u64>, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            Option::<String>::serialize(&value.map(|v| format!("{v:#x}")), serializer)
        }

        pub fn deserialize<'de, D>(d: D) -> Result<Option<u64>, D::Error>
        where
            D: Deserializer<'de>,
        {
            let value = Option::<String>::deserialize(d)?;
            match value {
                Some(s) if !s.is_empty() => u64::from_str_radix(s.trim_start_matches("0x"), 16)
                    .map_err(|_| D::Error::custom("Failed to deserialize u64 value"))
                    .map(Some),
                _ => Ok(None),
            }
        }
    }

    pub mod hex_str_opt_padded {
        use serde::Serialize;

        use super::*;
        pub fn serialize<S>(value: &Option<u64>, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            Option::<String>::serialize(&value.map(|v| format!("{v:#018x}")), serializer)
        }

        pub fn deserialize<'de, D>(d: D) -> Result<Option<u64>, D::Error>
        where
            D: Deserializer<'de>,
        {
            super::hex_str_opt::deserialize(d)
        }
    }

    pub fn deser_dec_str<'de, D>(d: D) -> Result<u64, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(d)?;
        value
            .parse()
            .map_err(|_| D::Error::custom("Failed to deserialize u64 value"))
    }

    pub fn deser_hex_or_dec_str<'de, D>(d: D) -> Result<u64, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(d)?;
        if value.starts_with("0x") {
            u64::from_str_radix(value.trim_start_matches("0x"), 16)
                .map_err(|_| D::Error::custom("Failed to deserialize u64 value"))
        } else {
            value
                .parse()
                .map_err(|_| D::Error::custom("Failed to deserialize u64 value"))
        }
    }
}

pub mod u128 {
    use super::*;

    pub mod hex_str {
        use super::*;

        pub fn deserialize<'de, D>(d: D) -> Result<u128, D::Error>
        where
            D: Deserializer<'de>,
        {
            let value = String::deserialize(d)?;
            u128::from_str_radix(value.trim_start_matches("0x"), 16)
                .map_err(|_| D::Error::custom("Failed to deserialize u128 value"))
        }

        pub fn serialize<S>(value: &u128, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serializer.serialize_str(&format!("{value:#x}"))
        }
    }
}

pub mod vec_u8 {
    use ::bytes::Bytes;

    use super::*;

    pub fn deserialize<'de, D>(d: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(d)?;
        let bytes = hex::decode(value.trim_start_matches("0x"))
            .map_err(|e| D::Error::custom(e.to_string()))?;
        Ok(bytes)
    }

    pub fn serialize<S>(value: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("0x{:x}", Bytes::copy_from_slice(value)))
    }
}

/// Serializes to and deserializes from 0x prefixed hex string
pub mod bytes {
    use ::bytes::Bytes;

    use super::*;

    pub fn deserialize<'de, D>(d: D) -> Result<Bytes, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(d)?;
        let bytes = hex::decode(value.trim_start_matches("0x"))
            .map_err(|e| D::Error::custom(e.to_string()))?;
        Ok(Bytes::from(bytes))
    }

    pub fn serialize<S>(value: &Bytes, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("0x{value:x}"))
    }

    pub mod vec {
        use super::*;

        pub fn deserialize<'de, D>(d: D) -> Result<Vec<Bytes>, D::Error>
        where
            D: Deserializer<'de>,
        {
            let value = Vec::<String>::deserialize(d)?;
            let mut output = Vec::new();
            for str in value {
                let bytes = hex::decode(str.trim_start_matches("0x"))
                    .map_err(|e| D::Error::custom(e.to_string()))?
                    .into();
                output.push(bytes);
            }
            Ok(output)
        }

        pub fn serialize<S>(value: &Vec<Bytes>, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serialize_vec_of_hex_encodables(value, serializer)
        }
    }
}

/// Serializes to and deserializes from 0x prefixed hex string
pub mod bool {
    use super::*;

    pub fn deserialize<'de, D>(d: D) -> Result<bool, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(d)?;
        Ok(u8::from_str_radix(value.trim_start_matches("0x"), 16)
            .map_err(|_| D::Error::custom("Failed to deserialize hex string to boolean value"))?
            != 0)
    }

    pub fn serialize<S>(value: &bool, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("{:#x}", *value as u8))
    }
}

pub mod bytes48 {
    use super::*;

    pub fn serialize<S>(value: &[u8; 48], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("0x{}", hex::encode(value)))
    }

    pub mod vec {
        use super::*;

        pub fn serialize<S>(value: &Vec<[u8; 48]>, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serialize_vec_of_hex_encodables(value, serializer)
        }

        pub fn deserialize<'de, D>(d: D) -> Result<Vec<[u8; 48]>, D::Error>
        where
            D: Deserializer<'de>,
        {
            let value = Vec::<String>::deserialize(d)?;
            let mut output = Vec::new();
            for str in value {
                let bytes = hex::decode(str.trim_start_matches("0x"))
                    .map_err(|e| D::Error::custom(e.to_string()))?;
                if bytes.len() != 48 {
                    return Err(D::Error::custom(format!(
                        "Expected 48 bytes, got {}",
                        bytes.len()
                    )));
                }
                let mut blob = [0u8; 48];
                blob.copy_from_slice(&bytes);
                output.push(blob);
            }
            Ok(output)
        }
    }
}

pub mod blob {
    use super::*;
    use crate::types::BYTES_PER_BLOB;

    pub fn serialize<S>(value: &[u8; BYTES_PER_BLOB], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("0x{}", hex::encode(value)))
    }

    pub mod vec {
        use super::*;

        pub fn serialize<S>(
            value: &Vec<[u8; BYTES_PER_BLOB]>,
            serializer: S,
        ) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serialize_vec_of_hex_encodables(value, serializer)
        }

        pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<[u8; BYTES_PER_BLOB]>, D::Error>
        where
            D: Deserializer<'de>,
        {
            let value = Vec::<String>::deserialize(deserializer)?;
            let mut output = Vec::new();
            for str in value {
                let bytes = hex::decode(str.trim_start_matches("0x"))
                    .map_err(|e| D::Error::custom(e.to_string()))?;
                if bytes.len() != BYTES_PER_BLOB {
                    return Err(D::Error::custom(format!(
                        "Expected {} bytes, got {}",
                        BYTES_PER_BLOB,
                        bytes.len()
                    )));
                }
                let mut blob = [0u8; BYTES_PER_BLOB];
                blob.copy_from_slice(&bytes);
                output.push(blob);
            }
            Ok(output)
        }
    }
}

// Const generics are not supported on `Serialize` impls so we need separate impls for different array sizes
fn serialize_vec_of_hex_encodables<S: Serializer, T: std::convert::AsRef<[u8]>>(
    value: &Vec<T>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let mut seq_serializer = serializer.serialize_seq(Some(value.len()))?;
    for encoded in value {
        seq_serializer.serialize_element(&format!("0x{}", hex::encode(encoded)))?;
    }
    seq_serializer.end()
}

pub mod duration {
    use std::time::Duration;

    use super::*;
    pub fn deserialize<'de, D>(d: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(d)?;
        parse_duration(value.clone())
            .ok_or_else(|| D::Error::custom(format!("Failed to parse Duration: {value}")))
    }

    pub mod opt {
        use super::*;

        pub fn deserialize<'de, D>(d: D) -> Result<Option<Duration>, D::Error>
        where
            D: Deserializer<'de>,
        {
            if let Some(value) = Option::<String>::deserialize(d)? {
                Ok(Some(parse_duration(value.clone()).ok_or_else(|| {
                    D::Error::custom(format!("Failed to parse Duration: {value}"))
                })?))
            } else {
                Ok(None)
            }
        }
    }
}

/// Parses a Duration in string format
/// The acceptable format is a concatentation of positive numeric values (with decimals allowed) followed by a time unit of measurement.
/// The units accepted are: Hours(h), Minutes(m), Senconds(s), Milliseconds(ms), Microseconds (us|µs) and Nanoseconds(ns)
/// For example, a duration such as "1h30m" or "1.6m" will be accepted but "-1s" or "30mh" will not
/// Some imprecision can be expected when using milliseconds/microseconds/nanoseconds with significant decimal components
/// If the format is incorrect this function will return None
fn parse_duration(input: String) -> Option<Duration> {
    let mut res = Duration::ZERO;
    let mut integer_buffer = String::new();
    let mut chars = input.chars().peekable();
    while let Some(char) = chars.next() {
        match char {
            // Numeric Value
            char @ '0'..='9' | char @ '.' => integer_buffer.push(char),
            // Unit of Measurement
            char => {
                // Parse the numeric value we collected
                let integer: f64 = integer_buffer.parse().ok()?;
                // Obtain the duration component based off of the unit of measurement
                let duration_component = match char {
                    // Hour
                    'h' => Duration::from_secs_f64(60_f64 * 60_f64 * integer),
                    'm' => {
                        if chars.peek().is_some_and(|c| *c == 's') {
                            chars.next();
                            // Millisecond
                            Duration::from_micros((integer * 1000_f64).round() as u64)
                        } else {
                            // Minute
                            Duration::from_secs_f64(60_f64 * integer)
                        }
                    }
                    // Second
                    's' => Duration::from_secs_f64(integer),
                    // Microsecond
                    'u' | 'µ' => {
                        if chars.next().is_some_and(|c| c == 's') {
                            Duration::from_nanos((integer * 1000_f64).round() as u64)
                        } else {
                            return None;
                        }
                    }
                    // Nanosecond
                    'n' => {
                        if chars.next().is_some_and(|c| c == 's') {
                            Duration::from_nanos(integer.round() as u64)
                        } else {
                            return None;
                        }
                    }
                    _ => return None,
                };
                // Add duration component to result
                res += duration_component;
                // Clear state so we can parse the next value
                integer_buffer.clear();
            }
        }
    }
    Some(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_duration_simple_integers() {
        assert_eq!(
            parse_duration("24h".to_string()),
            Some(Duration::from_secs(60 * 60 * 24))
        );
        assert_eq!(
            parse_duration("20m".to_string()),
            Some(Duration::from_secs(60 * 20))
        );
        assert_eq!(
            parse_duration("13s".to_string()),
            Some(Duration::from_secs(13))
        );
        assert_eq!(
            parse_duration("500ms".to_string()),
            Some(Duration::from_millis(500))
        );
        assert_eq!(
            parse_duration("900µs".to_string()),
            Some(Duration::from_micros(900))
        );
        assert_eq!(
            parse_duration("900us".to_string()),
            Some(Duration::from_micros(900))
        );
        assert_eq!(
            parse_duration("40ns".to_string()),
            Some(Duration::from_nanos(40))
        );
    }

    #[test]
    fn parse_duration_mixed_integers() {
        assert_eq!(
            parse_duration("24h30m".to_string()),
            Some(Duration::from_secs(60 * 60 * 24 + 30 * 60))
        );
        assert_eq!(
            parse_duration("20m15s".to_string()),
            Some(Duration::from_secs(60 * 20 + 15))
        );
        assert_eq!(
            parse_duration("13s4ms".to_string()),
            Some(Duration::from_secs(13) + Duration::from_millis(4))
        );
        assert_eq!(
            parse_duration("500ms60µs".to_string()),
            Some(Duration::from_millis(500) + Duration::from_micros(60))
        );
        assert_eq!(
            parse_duration("900us21ns".to_string()),
            Some(Duration::from_micros(900) + Duration::from_nanos(21))
        );
    }

    #[test]
    fn parse_duration_simple_with_decimals() {
        assert_eq!(
            parse_duration("1.5h".to_string()),
            Some(Duration::from_secs(60 * 90))
        );
        assert_eq!(
            parse_duration("0.5m".to_string()),
            Some(Duration::from_secs(30))
        );
        assert_eq!(
            parse_duration("4.5s".to_string()),
            Some(Duration::from_secs_f32(4.5))
        );
        assert_eq!(
            parse_duration("0.8ms".to_string()),
            Some(Duration::from_micros(800))
        );
        assert_eq!(
            parse_duration("0.95us".to_string()),
            Some(Duration::from_nanos(950))
        );
        // Rounded Up
        assert_eq!(
            parse_duration("0.75ns".to_string()),
            Some(Duration::from_nanos(1))
        );
    }

    #[test]
    fn parse_duration_mixed_decimals() {
        assert_eq!(
            parse_duration("1.5h0.5m10s".to_string()),
            Some(Duration::from_secs(60 * 90 + 30 + 10))
        );
        assert_eq!(
            parse_duration("0.5m15s".to_string()),
            Some(Duration::from_secs(30 + 15))
        );
    }
}
