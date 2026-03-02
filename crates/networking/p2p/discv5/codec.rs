use crate::discv5::messages::{Packet, PacketCodecError};

use bytes::BytesMut;
use ethrex_common::H256;
use std::io::{Error, ErrorKind};
use tokio_util::codec::{Decoder, Encoder};

#[derive(Debug)]
pub struct Discv5Codec {
    local_node_id: H256,
}

impl Discv5Codec {
    pub fn new(local_node_id: H256) -> Self {
        Self { local_node_id }
    }
}

impl Decoder for Discv5Codec {
    type Item = Packet;
    type Error = PacketCodecError;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if !buf.is_empty() {
            Ok(Some(Packet::decode(
                &self.local_node_id,
                &buf.split_to(buf.len()),
            )?))
        } else {
            Ok(None)
        }
    }
}

impl Encoder<Packet> for Discv5Codec {
    type Error = PacketCodecError;

    fn encode(&mut self, _packet: Packet, _buf: &mut BytesMut) -> Result<(), Self::Error> {
        Err(Error::new(
            ErrorKind::Unsupported,
            "Discv5Codec is receive-only; the server handles its own encoding",
        )
        .into())
    }
}
