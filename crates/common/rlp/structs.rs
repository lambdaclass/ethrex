use super::{
    decode::{decode_rlp_item, get_item_with_prefix, RLPDecode},
    encode::{encode_length, RLPEncode},
    error::RLPDecodeError,
};
use bytes::BufMut;
use bytes::Bytes;

/// # Struct decoding helper
///
/// Used to decode a struct from RLP format.
/// The struct's fields must implement [`RLPDecode`].
/// The struct is expected as a list, with its values being the fields
/// in the order they are passed to [`Decoder::decode_field`].
///
/// # Examples
///
/// ```
/// # use ethereum_rust_rlp::structs::Decoder;
/// # use ethereum_rust_rlp::error::RLPDecodeError;
/// # use ethereum_rust_rlp::decode::RLPDecode;
/// #[derive(Debug, PartialEq, Eq)]
/// struct Simple {
///     pub a: u8,
///     pub b: u16,
/// }
///
/// impl RLPDecode for Simple {
///     fn decode_unfinished(buf: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
///         let decoder = Decoder::new(&buf).unwrap();
///         // The fields are expected in the same order as given here
///         let (a, decoder) = decoder.decode_field("a").unwrap();
///         let (b, decoder) = decoder.decode_field("b").unwrap();
///         let rest = decoder.finish().unwrap();
///         Ok((Simple { a, b }, rest))
///     }
/// }
///
/// let bytes = [0xc2, 61, 75];
/// let decoded = Simple::decode(&bytes).unwrap();
///
/// assert_eq!(decoded, Simple { a: 61, b: 75 });
/// ```
#[derive(Debug)]
#[must_use = "`Decoder` must be consumed with `finish` to perform decoding checks"]
pub struct Decoder<'a> {
    payload: &'a [u8],
    remaining: &'a [u8],
}

impl<'a> Decoder<'a> {
    pub fn new(buf: &'a [u8]) -> Result<Self, RLPDecodeError> {
        match decode_rlp_item(buf)? {
            (true, payload, remaining) => Ok(Self { payload, remaining }),
            (false, _, _) => Err(RLPDecodeError::UnexpectedString),
        }
    }

    pub fn decode_field<T: RLPDecode>(self, name: &str) -> Result<(T, Self), RLPDecodeError> {
        let (field, rest) = <T as RLPDecode>::decode_unfinished(self.payload)
            .map_err(|err| field_decode_error::<T>(name, err))?;
        let updated_self = Self {
            payload: rest,
            ..self
        };
        Ok((field, updated_self))
    }
    /// Returns the next field without decoding it, i.e. the payload bytes including its prefix.
    pub fn get_encoded_item(self) -> Result<(Vec<u8>, Self), RLPDecodeError> {
        match get_item_with_prefix(self.payload) {
            Ok((field, rest)) => {
                let updated_self = Self {
                    payload: rest,
                    ..self
                };
                Ok((field.to_vec(), updated_self))
            }
            Err(err) => Err(err),
        }
    }

    /// Returns Some(field) if there's some field to decode, otherwise returns None
    pub fn decode_optional_field<T: RLPDecode>(self) -> (Option<T>, Self) {
        match <T as RLPDecode>::decode_unfinished(self.payload) {
            Ok((field, rest)) => {
                let updated_self = Self {
                    payload: rest,
                    ..self
                };
                (Some(field), updated_self)
            }
            Err(_) => (None, self),
        }
    }

    /// Finishes encoding the struct and returns the remaining bytes after the item.
    /// If the item's payload is not empty, returns an error.
    pub fn finish(self) -> Result<&'a [u8], RLPDecodeError> {
        if self.payload.is_empty() {
            Ok(self.remaining)
        } else {
            Err(RLPDecodeError::MalformedData)
        }
    }

    /// Same as [`finish`](Self::finish), but discards the item's remaining payload
    /// instead of failing.
    pub fn finish_unchecked(self) -> &'a [u8] {
        self.remaining
    }
}

fn field_decode_error<T>(field_name: &str, err: RLPDecodeError) -> RLPDecodeError {
    let typ = std::any::type_name::<T>();
    let err_msg = format!("Error decoding field '{field_name}' of type {typ}: {err}");
    RLPDecodeError::Custom(err_msg)
}

/// # Struct encoding helper
///
/// Used to encode a struct into RLP format.
/// The struct's fields must implement [`RLPEncode`].
/// The struct is encoded as a list, with its values being the fields
/// in the order they are passed to [`Encoder::encode_field`].
///
/// # Examples
///
/// ```
/// # use ethereum_rust_rlp::structs::Encoder;
/// # use ethereum_rust_rlp::encode::RLPEncode;
/// # use bytes::BufMut;
/// #[derive(Debug, PartialEq, Eq)]
/// struct Simple {
///     pub a: u8,
///     pub b: u16,
/// }
///
/// impl RLPEncode for Simple {
///     fn encode(&self, buf: &mut dyn BufMut) {
///         // The fields are encoded in the order given here
///         Encoder::new(buf)
///             .encode_field(&self.a)
///             .encode_field(&self.b)
///             .finish();
///     }
/// }
///
/// let mut buf = vec![];
/// Simple { a: 61, b: 75 }.encode(&mut buf);
///
/// assert_eq!(&buf, &[0xc2, 61, 75]);
/// ```
#[must_use = "`Encoder` must be consumed with `finish` to perform the encoding"]
pub struct Encoder<'a> {
    buf: &'a mut dyn BufMut,
    temp_buf: Vec<u8>,
}

// NOTE: BufMut doesn't implement Debug, so we can't derive Debug for Encoder.
impl core::fmt::Debug for Encoder<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Encoder")
            .field("buf", &"...")
            .field("temp_buf", &self.temp_buf)
            .finish()
    }
}

impl<'a> Encoder<'a> {
    /// Creates a new encoder that writes to the given buffer.
    pub fn new(buf: &'a mut dyn BufMut) -> Self {
        // PERF: we could pre-allocate the buffer or switch to `ArrayVec`` if we could
        // bound the size of the encoded data.
        Self {
            buf,
            temp_buf: Default::default(),
        }
    }

    /// Stores a field to be encoded.
    pub fn encode_field<T: RLPEncode>(mut self, value: &T) -> Self {
        <T as RLPEncode>::encode(value, &mut self.temp_buf);
        self
    }

    /// If `Some`, stores a field to be encoded, else does nothing.
    pub fn encode_optional_field<T: RLPEncode>(mut self, opt_value: &Option<T>) -> Self {
        if let Some(value) = opt_value {
            <T as RLPEncode>::encode(value, &mut self.temp_buf);
        }
        self
    }

    /// Stores a (key, value) list where the values are already encoded (i.e. value = RLP prefix || payload)
    /// but the keys are not encoded
    pub fn encode_key_value_list<T: RLPEncode>(mut self, list: &Vec<(Bytes, Bytes)>) -> Self {
        for (key, value) in list {
            <Bytes>::encode(key, &mut self.temp_buf);
            // value is already encoded
            self.temp_buf.put_slice(value);
        }
        self
    }

    /// Finishes encoding the struct and writes the result to the buffer.
    pub fn finish(self) {
        encode_length(self.temp_buf.len(), self.buf);
        self.buf.put_slice(&self.temp_buf);
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        decode::RLPDecode,
        encode::RLPEncode,
        structs::{Decoder, Encoder},
    };

    #[derive(Debug, PartialEq, Eq)]
    struct Simple {
        pub a: u8,
        pub b: u16,
    }

    #[test]
    fn test_decoder_simple_struct() {
        let expected = Simple { a: 61, b: 75 };
        let mut buf = Vec::new();
        (expected.a, expected.b).encode(&mut buf);

        let decoder = Decoder::new(&buf).unwrap();
        let (a, decoder) = decoder.decode_field("a").unwrap();
        let (b, decoder) = decoder.decode_field("b").unwrap();
        let rest = decoder.finish().unwrap();

        assert!(rest.is_empty());
        let got = Simple { a, b };
        assert_eq!(got, expected);

        // Decoding the struct as a tuple should give the same result
        let tuple_decode = <(u8, u16) as RLPDecode>::decode(&buf).unwrap();
        assert_eq!(tuple_decode, (a, b));
    }

    #[test]
    fn test_encoder_simple_struct() {
        let input = Simple { a: 61, b: 75 };
        let mut buf = Vec::new();

        Encoder::new(&mut buf)
            .encode_field(&input.a)
            .encode_field(&input.b)
            .finish();

        assert_eq!(buf, vec![0xc2, 61, 75]);

        // Encoding the struct from a tuple should give the same result
        let mut tuple_encoded = Vec::new();
        (input.a, input.b).encode(&mut tuple_encoded);
        assert_eq!(buf, tuple_encoded);
    }
}
