use bytes::BufMut;
use ethereum_rust_rlp::error::{RLPDecodeError, RLPEncodeError};
use std::fmt::Display;

use super::eth::status::StatusMessage;
use super::p2p::{DisconnectMessage, HelloMessage, PingMessage, PongMessage};

pub trait RLPxMessage: Sized {
    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError>;

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError>;
}
#[derive(Debug)]
pub(crate) enum Message {
    Hello(HelloMessage),
    Disconnect(DisconnectMessage),
    Ping(PingMessage),
    Pong(PongMessage),
    Status(StatusMessage),
}

impl Message {
    pub fn decode(msg_id: u8, msg_data: &[u8]) -> Result<Message, RLPDecodeError> {
        match msg_id {
            0x00 => Ok(Message::Hello(HelloMessage::decode(msg_data)?)),
            0x01 => Ok(Message::Disconnect(DisconnectMessage::decode(msg_data)?)),
            0x02 => Ok(Message::Ping(PingMessage::decode(msg_data)?)),
            0x03 => Ok(Message::Pong(PongMessage::decode(msg_data)?)),
            0x10 => Ok(Message::Status(StatusMessage::decode(msg_data)?)),
            _ => Err(RLPDecodeError::MalformedData),
        }
    }

    pub fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        match self {
            Message::Hello(msg) => msg.encode(buf),
            Message::Disconnect(msg) => msg.encode(buf),
            Message::Ping(msg) => msg.encode(buf),
            Message::Pong(msg) => msg.encode(buf),
            Message::Status(msg) => msg.encode(buf),
        }
    }
}

impl Display for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Message::Hello(_) => "p2p:Hello".fmt(f),
            Message::Disconnect(_) => "p2p:Disconnect".fmt(f),
            Message::Ping(_) => "p2p:Ping".fmt(f),
            Message::Pong(_) => "p2p:Pong".fmt(f),
            Message::Status(_) => "eth:Status".fmt(f),
        }
    }
}
