//! Discriminating codec for multiplexing discv4 and discv5 on a shared UDP socket.
//!
//! This codec simply passes through raw bytes - the actual protocol discrimination
//! happens in the multiplexer based on the keccak hash check.

use bytes::BytesMut;
use std::io::{Error, ErrorKind};
use std::net::SocketAddr;
use tokio_util::codec::{Decoder, Encoder};

/// A raw packet received from the UDP socket.
#[derive(Debug, Clone)]
pub struct RawPacket {
    pub data: BytesMut,
    pub from: SocketAddr,
}

/// A codec that passes through raw bytes for the multiplexer to process.
/// The actual protocol discrimination happens in the multiplexer.
#[derive(Debug)]
pub struct DiscriminatingCodec;

impl DiscriminatingCodec {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DiscriminatingCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder for DiscriminatingCodec {
    type Item = BytesMut;
    type Error = std::io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.is_empty() {
            Ok(None)
        } else {
            Ok(Some(src.split_to(src.len())))
        }
    }
}

impl Encoder<BytesMut> for DiscriminatingCodec {
    type Error = std::io::Error;

    fn encode(&mut self, _item: BytesMut, _dst: &mut BytesMut) -> Result<(), Self::Error> {
        Err(Error::new(
            ErrorKind::Unsupported,
            "DiscriminatingCodec is receive-only; each protocol handles its own encoding",
        ))
    }
}
