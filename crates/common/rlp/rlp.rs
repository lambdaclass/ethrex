//! # ethrex-rlp
//!
//! Recursive Length Prefix (RLP) encoding and decoding for the ethrex Ethereum client.
//!
//! RLP is the primary serialization format used in Ethereum for encoding structured data
//! including transactions, blocks, account state, and network protocol messages.
//!
//! ## Quick Start
//!
//! ```rust
//! use ethrex_rlp::encode::RLPEncode;
//! use ethrex_rlp::decode::RLPDecode;
//!
//! // Encoding
//! let value: u64 = 42;
//! let encoded = value.encode_to_vec();
//!
//! // Decoding
//! let decoded = u64::decode(&encoded).unwrap();
//! assert_eq!(value, decoded);
//! ```
//!
//! ## Core Traits
//!
//! - [`encode::RLPEncode`]: Trait for types that can be RLP-encoded
//! - [`decode::RLPDecode`]: Trait for types that can be RLP-decoded
//!
//! ## Builder Structs
//!
//! For complex types, use the builder pattern:
//!
//! - [`structs::Encoder`]: Fluent API for encoding structs field by field
//! - [`structs::Decoder`]: Fluent API for decoding structs with error context
//!
//! ## Modules
//!
//! - [`encode`]: Encoding trait, implementations, and helper functions
//! - [`decode`]: Decoding trait, implementations, and helper functions
//! - [`structs`]: `Encoder` and `Decoder` builder types for complex structures
//! - [`error`]: Error types for encoding and decoding failures
//! - [`constants`]: RLP protocol constants (`RLP_NULL`, `RLP_EMPTY_LIST`)
//!
//! ## Supported Types
//!
//! The crate provides built-in RLP implementations for:
//!
//! - **Primitives**: `bool`, `u8`, `u16`, `u32`, `u64`, `u128`, `usize`
//! - **Bytes**: `[u8]`, `[u8; N]`, `Vec<u8>`, `Bytes`, `str`, `String`
//! - **Ethereum types**: `Address`, `H256`, `U256`, `Signature`, `Bloom`
//! - **Collections**: `Vec<T>`, tuples up to 5 elements
//! - **Network**: `IpAddr`, `Ipv4Addr`, `Ipv6Addr`
//!
//! ## Custom Type Example
//!
//! ```rust
//! use ethrex_rlp::{
//!     encode::RLPEncode,
//!     decode::RLPDecode,
//!     structs::{Encoder, Decoder},
//!     error::RLPDecodeError,
//! };
//! use bytes::BufMut;
//!
//! struct MyStruct {
//!     nonce: u64,
//!     value: u64,
//! }
//!
//! impl RLPEncode for MyStruct {
//!     fn encode(&self, buf: &mut dyn BufMut) {
//!         Encoder::new(buf)
//!             .encode_field(&self.nonce)
//!             .encode_field(&self.value)
//!             .finish();
//!     }
//! }
//!
//! impl RLPDecode for MyStruct {
//!     fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
//!         let decoder = Decoder::new(rlp)?;
//!         let (nonce, decoder) = decoder.decode_field("nonce")?;
//!         let (value, decoder) = decoder.decode_field("value")?;
//!         let remaining = decoder.finish()?;
//!         Ok((Self { nonce, value }, remaining))
//!     }
//! }
//! ```
//!
//! ## Security
//!
//! - Maximum payload size of 1GB prevents memory exhaustion attacks
//! - Strict validation rejects malformed data with trailing bytes
//! - All decoding validates format before returning data

pub mod constants;
pub mod decode;
pub mod encode;
pub mod error;
pub mod structs;
