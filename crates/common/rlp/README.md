# ethrex-rlp

Recursive Length Prefix (RLP) encoding and decoding library for the ethrex Ethereum client.

## Overview

This crate provides a complete implementation of [RLP encoding](https://ethereum.org/en/developers/docs/data-structures-and-encoding/rlp/), the primary serialization format used in Ethereum for encoding structured data. RLP is used throughout the Ethereum protocol for serializing transactions, blocks, account state, and network messages.

## Features

- **Trait-based API**: `RLPEncode` and `RLPDecode` traits for implementing custom types
- **Builder pattern**: `Encoder` and `Decoder` structs for encoding/decoding complex structures
- **Comprehensive type support**: Built-in implementations for primitives, Ethereum types, and collections
- **Zero-copy decoding**: Returns borrowed slices where possible for performance
- **Security hardening**: 1GB payload limit to prevent memory exhaustion attacks
- **Snap compression**: Integrated support for Snap-compressed RLP data

## Usage

### Encoding

```rust
use ethrex_rlp::encode::RLPEncode;

// Encode primitives
let encoded = 42u64.encode_to_vec();

// Encode using the standalone function
use ethrex_rlp::encode::encode;
let encoded = encode(&"hello");
```

### Decoding

```rust
use ethrex_rlp::decode::RLPDecode;

// Decode primitives
let value = u64::decode(&encoded)?;

// Decode with remaining bytes (for streaming)
let (value, remaining) = u64::decode_unfinished(&data)?;
```

### Custom Structs

Use the `Encoder` and `Decoder` helpers for implementing RLP on custom types:

```rust
use ethrex_rlp::{
    encode::RLPEncode,
    decode::RLPDecode,
    structs::{Encoder, Decoder},
    error::RLPDecodeError,
};

struct Transaction {
    nonce: u64,
    gas_price: u64,
    to: Option<Address>,
}

impl RLPEncode for Transaction {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.nonce)
            .encode_field(&self.gas_price)
            .encode_optional_field(&self.to)
            .finish();
    }
}

impl RLPDecode for Transaction {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (nonce, decoder) = decoder.decode_field("nonce")?;
        let (gas_price, decoder) = decoder.decode_field("gas_price")?;
        let (to, decoder) = decoder.decode_optional_field()?;
        let remaining = decoder.finish()?;

        Ok((Self { nonce, gas_price, to }, remaining))
    }
}
```

## Supported Types

### Primitives
- Integers: `u8`, `u16`, `u32`, `u64`, `u128`, `usize`
- `bool`, `()`

### Byte Types
- `[u8]`, `[u8; N]`, `Vec<u8>`, `Bytes`
- `str`, `&str`, `String`

### Ethereum Types (from `ethereum-types`)
- Hashes: `H32`, `H64`, `H128`, `H256`, `H264`, `H512`
- `Address`, `U256`, `Signature`, `Bloom`

### Collections
- `Vec<T>` where `T: RLPEncode/RLPDecode`
- Tuples up to 5 elements: `(A, B)`, `(A, B, C)`, etc.
- `Option<T>` (via `encode_optional_field`/`decode_optional_field`)

### Network Types
- `IpAddr`, `Ipv4Addr`, `Ipv6Addr`

## Module Structure

| Module | Description |
|--------|-------------|
| `encode` | `RLPEncode` trait and encoding implementations |
| `decode` | `RLPDecode` trait and decoding implementations |
| `structs` | `Encoder` and `Decoder` builder structs |
| `error` | `RLPDecodeError` and `RLPEncodeError` types |
| `constants` | RLP protocol constants (`RLP_NULL`, `RLP_EMPTY_LIST`) |

## RLP Encoding Rules

The encoding follows the [Ethereum RLP specification](https://ethereum.org/en/developers/docs/data-structures-and-encoding/rlp/):

1. **Single byte `[0x00, 0x7f]`**: Encoded as itself
2. **Empty string**: Encoded as `0x80`
3. **Strings 1-55 bytes**: `[0x80 + length] ++ data`
4. **Strings > 55 bytes**: `[0xb7 + len_of_len] ++ be_length ++ data`
5. **Empty list**: Encoded as `0xc0`
6. **Lists with payload 0-55 bytes**: `[0xc0 + length] ++ payload`
7. **Lists with payload > 55 bytes**: `[0xf7 + len_of_len] ++ be_length ++ payload`

## Error Handling

The crate provides detailed error types:

- `InvalidLength`: Data too short or exceeds 1GB limit
- `MalformedData`: Invalid RLP format or trailing bytes
- `MalformedBoolean`: Boolean not encoded as `0x00` or `0x01`
- `UnexpectedList`: Expected bytes, found list
- `UnexpectedString`: Expected list, found bytes
- `InvalidCompression`: Snap decompression failed
- `Custom`: Application-specific errors with context

The `Decoder` automatically wraps errors with field name and type information for debugging.

## Performance

- **Length pre-computation**: Uses `ByteCounter` to calculate encoded length without allocation
- **Inline hints**: Critical paths annotated with `#[inline(always)]`
- **Leading zero stripping**: Integers encoded without unnecessary leading zeros
- **Progressive parsing**: `decode_unfinished` enables streaming without knowing structure boundaries

## Security

- Maximum payload size of 1GB prevents memory exhaustion attacks
- Strict validation ensures no trailing bytes in decoded data
- All decoding operations validate format before returning data
