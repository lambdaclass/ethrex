use serde::{de::Error, ser::SerializeSeq, Deserialize, Deserializer, Serializer};

pub mod u256 {
    use super::*;
    use ethereum_types::U256;
    use serde_json::Number;

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

    pub fn deser_dec_str<'de, D>(d: D) -> Result<U256, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(d)?;
        U256::from_dec_str(&value).map_err(|e| D::Error::custom(e.to_string()))
    }

    pub fn deser_hex_str<'de, D>(d: D) -> Result<U256, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(d)?;
        U256::from_str_radix(value.trim_start_matches("0x"), 16)
            .map_err(|_| D::Error::custom("Failed to deserialize u256 value"))
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
}

pub mod u64 {
    use super::*;

    pub mod hex_str {
        use super::*;

        pub fn deserialize<'de, D>(d: D) -> Result<u64, D::Error>
        where
            D: Deserializer<'de>,
        {
            let value = String::deserialize(d)?;
            let res = u64::from_str_radix(value.trim_start_matches("0x"), 16)
                .map_err(|_| D::Error::custom("Failed to deserialize u64 value"));
            res
        }

        pub fn serialize<S>(value: &u64, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serializer.serialize_str(&format!("{:#x}", value))
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
            serializer.serialize_str(&format!("{:#018x}", value))
        }
    }

    pub mod hex_str_opt {
        use serde::Serialize;

        use super::*;

        pub fn serialize<S>(value: &Option<u64>, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            Option::<String>::serialize(&value.map(|v| format!("{:#x}", v)), serializer)
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
            Option::<String>::serialize(&value.map(|v| format!("{:#018x}", v)), serializer)
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
            let res = u128::from_str_radix(value.trim_start_matches("0x"), 32)
                .map_err(|_| D::Error::custom("Failed to deserialize u128 value"));
            res
        }

        pub fn serialize<S>(value: &u128, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serializer.serialize_str(&format!("{:#x}", value))
        }
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
        serializer.serialize_str(&format!("0x{:x}", value))
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

    pub mod vec {
        use crate::types::BYTES_PER_BLOB;

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
