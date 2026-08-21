use super::{
    decode::{RLPDecode, decode_rlp_item, get_item_with_prefix},
    encode::{RLPEncode, backpatch_list_prefix},
    error::RLPDecodeError,
};
use alloc::format;
use alloc::vec::Vec;
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
/// # use ethrex_rlp::structs::Decoder;
/// # use ethrex_rlp::error::RLPDecodeError;
/// # use ethrex_rlp::decode::RLPDecode;
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
        self.get_encoded_item_ref()
            .map(|(field, updated_self)| (field.to_vec(), updated_self))
    }

    /// Returns the next field without decoding it, i.e. the payload bytes including its prefix.
    pub fn get_encoded_item_ref(self) -> Result<(&'a [u8], Self), RLPDecodeError> {
        get_item_with_prefix(self.payload).map(|(field, rest)| {
            let updated_self = Self {
                payload: rest,
                ..self
            };
            (field, updated_self)
        })
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
    pub const fn finish(self) -> Result<&'a [u8], RLPDecodeError> {
        if self.payload.is_empty() {
            Ok(self.remaining)
        } else {
            Err(RLPDecodeError::MalformedData)
        }
    }

    /// Returns true if the decoder has finished decoding the given input
    pub const fn is_done(&self) -> bool {
        self.payload.is_empty()
    }

    /// Same as [`finish`](Self::finish), but discards the item's remaining payload
    /// instead of failing.
    pub const fn finish_unchecked(self) -> &'a [u8] {
        self.remaining
    }

    pub const fn get_payload_len(&self) -> usize {
        self.payload.len()
    }
}

fn field_decode_error<T>(field_name: &str, err: RLPDecodeError) -> RLPDecodeError {
    let typ = core::any::type_name::<T>();
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
/// # use ethrex_rlp::structs::Encoder;
/// # use ethrex_rlp::encode::RLPEncode;
/// #[derive(Debug, PartialEq, Eq)]
/// struct Simple {
///     pub a: u8,
///     pub b: u16,
/// }
///
/// impl RLPEncode for Simple {
///     fn encode(&self, buf: &mut Vec<u8>) {
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
#[derive(Debug)]
#[must_use = "`Encoder` must be consumed with `finish` to write the list prefix"]
pub struct Encoder<'a> {
    buf: &'a mut Vec<u8>,
    /// Where this list's payload starts in `buf`; the prefix is inserted here
    /// by `finish`.
    start: usize,
}

/// Catches an `Encoder` that is dropped without `finish`.
///
/// Fields are written straight into the output buffer, so a missed `finish`
/// leaves a payload with no list prefix in front of it. If that happens inside
/// an outer list, the outer `finish` prefixes the lot and produces a
/// well-formed but structurally wrong encoding, which decodes cleanly and is
/// therefore very hard to trace back. `#[must_use]` catches the common shape
/// (an unused `Encoder` as a statement) but not an encoder bound to a variable
/// and then abandoned on an early return, so debug builds assert too.
#[cfg(debug_assertions)]
impl Drop for Encoder<'_> {
    fn drop(&mut self) {
        panic!(
            "`Encoder` dropped without `finish()`: {} bytes of RLP payload are in \
             the output buffer with no list prefix in front of them",
            self.buf.len() - self.start
        );
    }
}

impl<'a> Encoder<'a> {
    /// Creates a new encoder appending to the given buffer.
    ///
    /// Records the buffer's current length as the start of this list's payload.
    /// Fields are written through to `buf` immediately, so the caller must
    /// reach [`Encoder::finish`] for the result to be valid RLP.
    pub fn new(buf: &'a mut Vec<u8>) -> Self {
        let start = buf.len();
        Self { buf, start }
    }

    /// Encodes a field straight into the output buffer.
    pub fn encode_field<T: RLPEncode>(self, value: &T) -> Self {
        <T as RLPEncode>::encode(value, self.buf);
        self
    }

    /// If `Some`, encodes a field, else does nothing.
    pub fn encode_optional_field<T: RLPEncode>(self, opt_value: &Option<T>) -> Self {
        if let Some(value) = opt_value {
            <T as RLPEncode>::encode(value, self.buf);
        }
        self
    }

    /// Encodes a (key, value) list where the values are already encoded (i.e. value = RLP prefix || payload)
    /// but the keys are not encoded
    pub fn encode_key_value_list<T: RLPEncode>(self, list: &Vec<(Bytes, Bytes)>) -> Self {
        for (key, value) in list {
            <Bytes>::encode(key, self.buf);
            // value is already encoded
            self.buf.extend_from_slice(value);
        }
        self
    }

    /// Finishes encoding the struct by inserting the list prefix in front of
    /// the payload that has been accumulating in the output buffer.
    pub fn finish(self) {
        // The list is complete, so defuse the unfinished-encoder assertion.
        // `ManuallyDrop` rather than `mem::forget` because the `Drop` impl only
        // exists in debug builds and forgetting a non-`Drop` type is a clippy
        // error; this reborrows either way instead of moving out of `self`.
        let mut this = core::mem::ManuallyDrop::new(self);
        let start = this.start;
        backpatch_list_prefix(this.buf, start);
    }

    /// Adds a raw value to the buffer without rlp-encoding it
    pub fn encode_raw(self, value: &[u8]) -> Self {
        self.buf.extend_from_slice(value);
        self
    }

    /// Encodes a field as bytes
    /// This method is used to bypass the conflicting implementations between Vec<T> and Vec<u8>
    pub fn encode_bytes(self, value: &[u8]) -> Self {
        <[u8] as RLPEncode>::encode(value, self.buf);
        self
    }
}
