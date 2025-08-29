use crate::{
    discv4::messages::Packet,
    rlpx::{error::RLPxError, message as rlpx},
};

use bytes::BytesMut;
use tokio_util::codec::{Decoder, Encoder};

pub struct Discv4Codec {}

impl Discv4Codec {
    pub(crate) fn new() -> Self {
        Self {}
    }
}

impl Decoder for Discv4Codec {
    type Item = Packet;
    type Error = std::io::Error;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if !buf.is_empty() {
            let len = buf.len();
            Ok(Some(Packet::decode(&buf.split_to(len)).map_err(|err| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string())
            })?))
        } else {
            Ok(None)
        }
    }
}

impl Encoder<rlpx::Message> for Discv4Codec {
    type Error = RLPxError;

    fn encode(&mut self, _message: rlpx::Message, _buffer: &mut BytesMut) -> Result<(), Self::Error> {
        Ok(())
    }
}
