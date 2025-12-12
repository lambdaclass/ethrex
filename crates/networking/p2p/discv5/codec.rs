use crate::discv5::messages::{Packet, PacketDecodeErr};

use bytes::BytesMut;
use ethrex_common::H256;
use tokio_util::codec::{Decoder, Encoder};

#[derive(Debug)]
pub struct Discv5Codec {
    dest_id: H256,
    nonce: u128,
}

impl Discv5Codec {
    pub fn new(dest_id: H256) -> Self {
        Self {
            dest_id,
            nonce: rand::random(),
        }
    }

    fn new_nonce(&mut self) -> Vec<u8> {
        self.nonce = self.nonce.wrapping_add(1);
        self.nonce.to_be_bytes()[4..].to_vec()
    }
}

impl Decoder for Discv5Codec {
    type Item = Packet;
    type Error = PacketDecodeErr;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if !buf.is_empty() {
            Ok(Some(Packet::decode(
                &self.dest_id,
                &buf.split_to(buf.len()),
            )?))
        } else {
            Ok(None)
        }
    }
}

impl Encoder<Packet> for Discv5Codec {
    type Error = PacketDecodeErr;

    fn encode(&mut self, package: Packet, buf: &mut BytesMut) -> Result<(), Self::Error> {
        let masking_iv: u128 = rand::random();
        let nonce = self.new_nonce();
        package.encode(buf, masking_iv, nonce, &self.dest_id)
    }
}
