use bytes::BufMut;
use ethereum_rust_rlp::error::{RLPDecodeError, RLPEncodeError};
use std::fmt::Display;

use super::eth::status::StatusMessage;
use super::p2p::{DisconnectMessage, HelloMessage, PingMessage, PongMessage};
use super::snap::{AccountRange, GetAccountRange, GetStorageRanges, StorageRanges};

use ethereum_rust_rlp::encode::RLPEncode;

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
    // snap capability
    GetAccountRange(GetAccountRange),
    AccountRange(AccountRange),
    GetStorageRanges(GetStorageRanges),
    StorageRanges(StorageRanges),
}

impl Message {
    pub fn decode(msg_id: u8, msg_data: &[u8]) -> Result<Message, RLPDecodeError> {
        match msg_id {
            0x00 => Ok(Message::Hello(HelloMessage::decode(msg_data)?)),
            0x01 => Ok(Message::Disconnect(DisconnectMessage::decode(msg_data)?)),
            0x02 => Ok(Message::Ping(PingMessage::decode(msg_data)?)),
            0x03 => Ok(Message::Pong(PongMessage::decode(msg_data)?)),
            0x10 => Ok(Message::Status(StatusMessage::decode(msg_data)?)),
            0x21 => Ok(Message::GetAccountRange(GetAccountRange::decode(msg_data)?)),
            0x22 => Ok(Message::AccountRange(AccountRange::decode(msg_data)?)),
            0x23 => Ok(Message::GetStorageRanges(GetStorageRanges::decode(
                msg_data,
            )?)),
            0x24 => Ok(Message::StorageRanges(StorageRanges::decode(msg_data)?)),
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
            Message::GetAccountRange(msg) => {
                0x21_u8.encode(buf);
                msg.encode(buf)
            }
            Message::AccountRange(msg) => {
                0x22_u8.encode(buf);
                msg.encode(buf)
            }
            Message::GetStorageRanges(msg) => {
                0x23_u8.encode(buf);
                msg.encode(buf)
            }
            Message::StorageRanges(msg) => {
                0x24_u8.encode(buf);
                msg.encode(buf)
            }
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
            Message::GetAccountRange(_) => "snap:GetAccountRange".fmt(f),
            Message::AccountRange(_) => "snap:AccountRange".fmt(f),
            Message::GetStorageRanges(_) => "snap:GetStorageRanges".fmt(f),
            Message::StorageRanges(_) => "snap:StorageRanges".fmt(f),
        }
    }
}
