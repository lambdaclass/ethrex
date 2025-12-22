use crate::discv5::messages::{Packet, PacketCodecError};
use crate::discv5::session::Session;

use bytes::BytesMut;
use ethrex_common::H256;
use tokio_util::codec::{Decoder, Encoder};

#[derive(Debug)]
pub struct Discv5Codec {
    local_node_id: H256,
    /// Outgoing message count, used for nonce generation as per the spec.
    session: Option<Session>,
}

impl Discv5Codec {
    pub fn new(local_node_id: H256) -> Self {
        Self {
            local_node_id,
            session: None,
        }
    }
}

impl Decoder for Discv5Codec {
    type Item = Packet;
    type Error = PacketCodecError;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        Ok(Some(Packet::decode(
            &self.local_node_id,
            &buf.split_to(buf.len()),
        )?))
    }
}

impl Encoder<Packet> for Discv5Codec {
    type Error = PacketCodecError;

    fn encode(&mut self, _packet: Packet, _buf: &mut BytesMut) -> Result<(), Self::Error> {
        // We are not going to use Discv5Coded to send messages, only to receive them
        unimplemented!();
    }
}
