use crate::{
    discv4::messages::Packet,
    rlpx::{error::RLPxError, message as rlpx},
};

use bytes::BytesMut;
use std::io::{Error, ErrorKind};
use tokio_util::codec::{Decoder, Encoder};

pub struct Discv4Codec;

impl Decoder for Discv4Codec {
    type Item = Packet;
    type Error = Error;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if buf.is_empty() {
            return Ok(None);
        }
        Ok(Some(Packet::decode(&buf).map_err(|err| {
            Error::new(ErrorKind::InvalidData, err.to_string())
        })?))
    }
}

impl Encoder<rlpx::Message> for Discv4Codec {
    type Error = RLPxError;

    fn encode(
        &mut self,
        _message: rlpx::Message,
        _buffer: &mut BytesMut,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}
