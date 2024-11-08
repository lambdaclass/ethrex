use bytes::BufMut;
use ethereum_rust_rlp::error::{RLPDecodeError, RLPEncodeError};
use std::fmt::Display;

use super::eth::blocks::{BlockBodies, BlockHeaders, GetBlockBodies, GetBlockHeaders};
use super::eth::status::StatusMessage;
use super::p2p::{DisconnectMessage, HelloMessage, PingMessage, PongMessage};
use super::snap::{
    AccountRange, ByteCodes, GetAccountRange, GetByteCodes, GetStorageRanges, GetTrieNodes,
    StorageRanges, TrieNodes,
};

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
    // https://github.com/ethereum/devp2p/blob/5713591d0366da78a913a811c7502d9ca91d29a8/caps/eth.md#getblockheaders-0x03
    GetBlockHeaders(GetBlockHeaders),
    BlockHeaders(BlockHeaders),
    GetBlockBodies(GetBlockBodies),
    BlockBodies(BlockBodies),
    // snap capability
    GetAccountRange(GetAccountRange),
    AccountRange(AccountRange),
    GetStorageRanges(GetStorageRanges),
    StorageRanges(StorageRanges),
    GetByteCodes(GetByteCodes),
    ByteCodes(ByteCodes),
    GetTrieNodes(GetTrieNodes),
    TrieNodes(TrieNodes),
}

impl Message {
    pub fn decode(msg_id: u8, msg_data: &[u8]) -> Result<Message, RLPDecodeError> {
        match msg_id {
            0x00 => Ok(Message::Hello(HelloMessage::decode(msg_data)?)),
            0x01 => Ok(Message::Disconnect(DisconnectMessage::decode(msg_data)?)),
            0x02 => Ok(Message::Ping(PingMessage::decode(msg_data)?)),
            0x03 => Ok(Message::Pong(PongMessage::decode(msg_data)?)),
            // Subprotocols like 'eth' use offsets to identify
            // themselves, the eth capability starts
            // at 0x10 (16), the status message
            // has offset 0, so a message with id 0x10
            // identifies an eth status message.
            // Another example is the eth getBlockHeaders message,
            // which has 3 as its offset, so it is identified as 0x13 (19).
            // References:
            // - https://ethereum.stackexchange.com/questions/37051/ethereum-network-messaging
            // - https://github.com/ethereum/devp2p/blob/master/caps/eth.md#status-0x00
            0x10 => Ok(Message::Status(StatusMessage::decode(msg_data)?)),
            0x13 => Ok(Message::GetBlockHeaders(GetBlockHeaders::decode(msg_data)?)),
            0x14 => Ok(Message::BlockHeaders(BlockHeaders::decode(msg_data)?)),
            0x15 => Ok(Message::GetBlockBodies(GetBlockBodies::decode(msg_data)?)),
            0x21 => Ok(Message::GetAccountRange(GetAccountRange::decode(msg_data)?)),
            0x22 => Ok(Message::AccountRange(AccountRange::decode(msg_data)?)),
            0x23 => Ok(Message::GetStorageRanges(GetStorageRanges::decode(
                msg_data,
            )?)),
            0x24 => Ok(Message::StorageRanges(StorageRanges::decode(msg_data)?)),
            0x25 => Ok(Message::GetByteCodes(GetByteCodes::decode(msg_data)?)),
            0x26 => Ok(Message::ByteCodes(ByteCodes::decode(msg_data)?)),
            0x27 => Ok(Message::GetTrieNodes(GetTrieNodes::decode(msg_data)?)),
            0x28 => Ok(Message::TrieNodes(TrieNodes::decode(msg_data)?)),
            _ => Err(RLPDecodeError::MalformedData),
        }
    }

    pub fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        match self {
            Message::Hello(msg) => {
                0x00_u8.encode(buf);
                msg.encode(buf)
            }
            Message::Disconnect(msg) => {
                0x01_u8.encode(buf);
                msg.encode(buf)
            }
            Message::Ping(msg) => {
                0x02_u8.encode(buf);
                msg.encode(buf)
            }
            Message::Pong(msg) => {
                0x03_u8.encode(buf);
                msg.encode(buf)
            }
            Message::Status(msg) => {
                0x10_u8.encode(buf);
                msg.encode(buf)
            }
            Message::GetBlockHeaders(msg) => {
                0x13_u8.encode(buf);
                msg.encode(buf)
            }
            Message::BlockHeaders(msg) => {
                0x14_u8.encode(buf);
                msg.encode(buf)
            }
            Message::GetBlockBodies(msg) => {
                0x15_u8.encode(buf);
                msg.encode(buf)
            }
            Message::BlockBodies(msg) => {
                0x16_u8.encode(buf);
                msg.encode(buf)
            }
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
            Message::GetByteCodes(msg) => {
                0x25_u8.encode(buf);
                msg.encode(buf)
            }
            Message::ByteCodes(msg) => {
                0x26_u8.encode(buf);
                msg.encode(buf)
            }
            Message::GetTrieNodes(msg) => {
                0x27_u8.encode(buf);
                msg.encode(buf)
            }
            Message::TrieNodes(msg) => {
                0x28_u8.encode(buf);
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
            Message::GetBlockHeaders(_) => "eth:getBlockHeaders".fmt(f),
            Message::BlockHeaders(_) => "eth:BlockHeaders".fmt(f),
            Message::BlockBodies(_) => "eth:BlockBodies".fmt(f),
            Message::GetBlockBodies(_) => "eth:GetBlockBodies".fmt(f),
            Message::GetAccountRange(_) => "snap:GetAccountRange".fmt(f),
            Message::AccountRange(_) => "snap:AccountRange".fmt(f),
            Message::GetStorageRanges(_) => "snap:GetStorageRanges".fmt(f),
            Message::StorageRanges(_) => "snap:StorageRanges".fmt(f),
            Message::GetByteCodes(_) => "snap:GetByteCodes".fmt(f),
            Message::ByteCodes(_) => "snap:ByteCodes".fmt(f),
            Message::GetTrieNodes(_) => "snap:GetTrieNodes".fmt(f),
            Message::TrieNodes(_) => "snap:TrieNodes".fmt(f),
        }
    }
}
