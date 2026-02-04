use bytes::BufMut;
use ethrex_rlp::error::{RLPDecodeError, RLPEncodeError};
use std::fmt::Display;

use crate::rlpx::snap::{
    AccountRange, ByteCodes, GetAccountRange, GetByteCodes, GetStorageRanges, GetTrieNodes,
    StorageRanges, TrieNodes,
};

use super::eth::blocks::{BlockBodies, BlockHeaders, GetBlockBodies, GetBlockHeaders};
use super::eth::receipts::{GetReceipts, Receipts68, Receipts69};
use super::eth::status::{StatusMessage68, StatusMessage69};
use super::eth::transactions::{
    GetPooledTransactions, NewPooledTransactionHashes, PooledTransactions, Transactions,
};
use super::eth::update::BlockRangeUpdate;
#[cfg(feature = "l2")]
use super::l2::messages::{BatchSealed, L2Message, NewBlock};
#[cfg(feature = "l2")]
use super::l2::{self, messages};
use super::p2p::{DisconnectMessage, HelloMessage, PingMessage, PongMessage};

use ethrex_rlp::encode::RLPEncode;

const ETH_CAPABILITY_OFFSET: u8 = 0x10;
const SNAP_CAPABILITY_OFFSET_ETH_68: u8 = 0x21;
const SNAP_CAPABILITY_OFFSET_ETH_69: u8 = 0x22;
const BASED_CAPABILITY_OFFSET_ETH_68: u8 = 0x30;
const BASED_CAPABILITY_OFFSET_ETH_69: u8 = 0x31;

#[derive(Debug, Clone, Copy, Default)]
pub enum EthCapVersion {
    #[default]
    V68,
    V69,
}

impl EthCapVersion {
    pub const fn eth_capability_offset(&self) -> u8 {
        ETH_CAPABILITY_OFFSET
    }

    pub const fn snap_capability_offset(&self) -> u8 {
        match self {
            EthCapVersion::V68 => SNAP_CAPABILITY_OFFSET_ETH_68,
            EthCapVersion::V69 => SNAP_CAPABILITY_OFFSET_ETH_69,
        }
    }

    pub const fn based_capability_offset(&self) -> u8 {
        match self {
            EthCapVersion::V68 => BASED_CAPABILITY_OFFSET_ETH_68,
            EthCapVersion::V69 => BASED_CAPABILITY_OFFSET_ETH_69,
        }
    }
}

pub trait RLPxMessage: Sized {
    const CODE: u8;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError>;

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError>;
}
#[derive(Debug, Clone)]
pub enum Message {
    Hello(HelloMessage),
    Disconnect(DisconnectMessage),
    Ping(PingMessage),
    Pong(PongMessage),
    Status68(StatusMessage68),
    Status69(StatusMessage69),
    // eth capability
    // https://github.com/ethereum/devp2p/blob/master/caps/eth.md
    GetBlockHeaders(GetBlockHeaders),
    BlockHeaders(BlockHeaders),
    Transactions(Transactions),
    GetBlockBodies(GetBlockBodies),
    BlockBodies(BlockBodies),
    NewPooledTransactionHashes(NewPooledTransactionHashes),
    GetPooledTransactions(GetPooledTransactions),
    PooledTransactions(PooledTransactions),
    GetReceipts(GetReceipts),
    Receipts68(Receipts68),
    Receipts69(Receipts69),
    BlockRangeUpdate(BlockRangeUpdate),
    // snap capability
    // https://github.com/ethereum/devp2p/blob/master/caps/snap.md
    GetAccountRange(GetAccountRange),
    AccountRange(AccountRange),
    GetStorageRanges(GetStorageRanges),
    StorageRanges(StorageRanges),
    GetByteCodes(GetByteCodes),
    ByteCodes(ByteCodes),
    GetTrieNodes(GetTrieNodes),
    TrieNodes(TrieNodes),
    // based capability
    #[cfg(feature = "l2")]
    L2(messages::L2Message),
}

impl Message {
    pub const fn code(&self, eth_version: EthCapVersion) -> u8 {
        match self {
            Message::Hello(_) => HelloMessage::CODE,
            Message::Disconnect(_) => DisconnectMessage::CODE,
            Message::Ping(_) => PingMessage::CODE,
            Message::Pong(_) => PongMessage::CODE,

            // eth capability
            Message::Status68(_) => eth_version.eth_capability_offset() + StatusMessage68::CODE,
            Message::Status69(_) => eth_version.eth_capability_offset() + StatusMessage69::CODE,
            Message::Transactions(_) => eth_version.eth_capability_offset() + Transactions::CODE,
            Message::GetBlockHeaders(_) => {
                eth_version.eth_capability_offset() + GetBlockHeaders::CODE
            }
            Message::BlockHeaders(_) => eth_version.eth_capability_offset() + BlockHeaders::CODE,
            Message::GetBlockBodies(_) => {
                eth_version.eth_capability_offset() + GetBlockBodies::CODE
            }
            Message::BlockBodies(_) => eth_version.eth_capability_offset() + BlockBodies::CODE,
            Message::NewPooledTransactionHashes(_) => {
                eth_version.eth_capability_offset() + NewPooledTransactionHashes::CODE
            }
            Message::GetPooledTransactions(_) => {
                eth_version.eth_capability_offset() + GetPooledTransactions::CODE
            }
            Message::PooledTransactions(_) => {
                eth_version.eth_capability_offset() + PooledTransactions::CODE
            }
            Message::GetReceipts(_) => eth_version.eth_capability_offset() + GetReceipts::CODE,
            Message::Receipts68(_) => eth_version.eth_capability_offset() + Receipts68::CODE,
            Message::Receipts69(_) => eth_version.eth_capability_offset() + Receipts68::CODE,
            Message::BlockRangeUpdate(_) => {
                eth_version.eth_capability_offset() + BlockRangeUpdate::CODE
            }
            // snap capability
            Message::GetAccountRange(_) => {
                eth_version.snap_capability_offset() + GetAccountRange::CODE
            }
            Message::AccountRange(_) => eth_version.snap_capability_offset() + AccountRange::CODE,
            Message::GetStorageRanges(_) => {
                eth_version.snap_capability_offset() + GetStorageRanges::CODE
            }
            Message::StorageRanges(_) => eth_version.snap_capability_offset() + StorageRanges::CODE,
            Message::GetByteCodes(_) => eth_version.snap_capability_offset() + GetByteCodes::CODE,
            Message::ByteCodes(_) => eth_version.snap_capability_offset() + ByteCodes::CODE,
            Message::GetTrieNodes(_) => eth_version.snap_capability_offset() + GetTrieNodes::CODE,
            Message::TrieNodes(_) => eth_version.snap_capability_offset() + TrieNodes::CODE,

            #[cfg(feature = "l2")]
            // based capability
            Message::L2(l2_msg) => {
                eth_version.based_capability_offset() + {
                    match l2_msg {
                        L2Message::NewBlock(_) => NewBlock::CODE,
                        L2Message::BatchSealed(_) => BatchSealed::CODE,
                    }
                }
            }
        }
    }
    pub fn decode(
        msg_id: u8,
        data: &[u8],
        eth_version: EthCapVersion,
    ) -> Result<Message, RLPDecodeError> {
        if msg_id < eth_version.eth_capability_offset() {
            match msg_id {
                HelloMessage::CODE => Ok(Message::Hello(HelloMessage::decode(data)?)),
                DisconnectMessage::CODE => {
                    Ok(Message::Disconnect(DisconnectMessage::decode(data)?))
                }
                PingMessage::CODE => Ok(Message::Ping(PingMessage::decode(data)?)),
                PongMessage::CODE => Ok(Message::Pong(PongMessage::decode(data)?)),
                _ => Err(RLPDecodeError::malformed_data()),
            }
        } else if msg_id < eth_version.snap_capability_offset() {
            // eth capability
            match msg_id - eth_version.eth_capability_offset() {
                StatusMessage68::CODE if matches!(eth_version, EthCapVersion::V68) => {
                    Ok(Message::Status68(StatusMessage68::decode(data)?))
                }
                StatusMessage69::CODE if matches!(eth_version, EthCapVersion::V69) => {
                    Ok(Message::Status69(StatusMessage69::decode(data)?))
                }
                Transactions::CODE => Ok(Message::Transactions(Transactions::decode(data)?)),
                GetBlockHeaders::CODE => {
                    Ok(Message::GetBlockHeaders(GetBlockHeaders::decode(data)?))
                }
                BlockHeaders::CODE => Ok(Message::BlockHeaders(BlockHeaders::decode(data)?)),
                GetBlockBodies::CODE => Ok(Message::GetBlockBodies(GetBlockBodies::decode(data)?)),
                BlockBodies::CODE => Ok(Message::BlockBodies(BlockBodies::decode(data)?)),
                NewPooledTransactionHashes::CODE => Ok(Message::NewPooledTransactionHashes(
                    NewPooledTransactionHashes::decode(data)?,
                )),
                GetPooledTransactions::CODE => Ok(Message::GetPooledTransactions(
                    GetPooledTransactions::decode(data)?,
                )),
                PooledTransactions::CODE => Ok(Message::PooledTransactions(
                    PooledTransactions::decode(data)?,
                )),
                GetReceipts::CODE => Ok(Message::GetReceipts(GetReceipts::decode(data)?)),
                Receipts68::CODE if matches!(eth_version, EthCapVersion::V68) => {
                    Ok(Message::Receipts68(Receipts68::decode(data)?))
                }
                Receipts69::CODE if matches!(eth_version, EthCapVersion::V69) => {
                    Ok(Message::Receipts69(Receipts69::decode(data)?))
                }
                BlockRangeUpdate::CODE => {
                    Ok(Message::BlockRangeUpdate(BlockRangeUpdate::decode(data)?))
                }
                _ => Err(RLPDecodeError::malformed_data()),
            }
        } else if msg_id < eth_version.based_capability_offset() {
            // snap capability
            match msg_id - eth_version.snap_capability_offset() {
                GetAccountRange::CODE => {
                    Ok(Message::GetAccountRange(GetAccountRange::decode(data)?))
                }
                AccountRange::CODE => Ok(Message::AccountRange(AccountRange::decode(data)?)),
                GetStorageRanges::CODE => {
                    Ok(Message::GetStorageRanges(GetStorageRanges::decode(data)?))
                }
                StorageRanges::CODE => Ok(Message::StorageRanges(StorageRanges::decode(data)?)),
                GetByteCodes::CODE => Ok(Message::GetByteCodes(GetByteCodes::decode(data)?)),
                ByteCodes::CODE => Ok(Message::ByteCodes(ByteCodes::decode(data)?)),
                GetTrieNodes::CODE => Ok(Message::GetTrieNodes(GetTrieNodes::decode(data)?)),
                TrieNodes::CODE => Ok(Message::TrieNodes(TrieNodes::decode(data)?)),
                _ => Err(RLPDecodeError::malformed_data()),
            }
        } else {
            // based capability
            #[cfg(feature = "l2")]
            return Ok(Message::L2(
                match msg_id - eth_version.based_capability_offset() {
                    messages::NewBlock::CODE => {
                        let decoded = l2::messages::NewBlock::decode(data)?;
                        L2Message::NewBlock(decoded)
                    }
                    BatchSealed::CODE => {
                        let decoded = l2::messages::BatchSealed::decode(data)?;
                        L2Message::BatchSealed(decoded)
                    }
                    _ => return Err(RLPDecodeError::malformed_data()),
                },
            ));

            #[cfg(not(feature = "l2"))]
            Err(RLPDecodeError::malformed_data())
        }
    }

    pub fn encode(
        &self,
        buf: &mut dyn BufMut,
        eth_version: EthCapVersion,
    ) -> Result<(), RLPEncodeError> {
        self.code(eth_version).encode(buf);
        match self {
            Message::Hello(msg) => msg.encode(buf),
            Message::Disconnect(msg) => msg.encode(buf),
            Message::Ping(msg) => msg.encode(buf),
            Message::Pong(msg) => msg.encode(buf),
            Message::Status68(msg) => msg.encode(buf),
            Message::Status69(msg) => msg.encode(buf),
            Message::Transactions(msg) => msg.encode(buf),
            Message::GetBlockHeaders(msg) => msg.encode(buf),
            Message::BlockHeaders(msg) => msg.encode(buf),
            Message::GetBlockBodies(msg) => msg.encode(buf),
            Message::BlockBodies(msg) => msg.encode(buf),
            Message::NewPooledTransactionHashes(msg) => msg.encode(buf),
            Message::GetPooledTransactions(msg) => msg.encode(buf),
            Message::PooledTransactions(msg) => msg.encode(buf),
            Message::GetReceipts(msg) => msg.encode(buf),
            Message::Receipts68(msg) => msg.encode(buf),
            Message::Receipts69(msg) => msg.encode(buf),
            Message::BlockRangeUpdate(msg) => msg.encode(buf),
            Message::GetAccountRange(msg) => msg.encode(buf),
            Message::AccountRange(msg) => msg.encode(buf),
            Message::GetStorageRanges(msg) => msg.encode(buf),
            Message::StorageRanges(msg) => msg.encode(buf),
            Message::GetByteCodes(msg) => msg.encode(buf),
            Message::ByteCodes(msg) => msg.encode(buf),
            Message::GetTrieNodes(msg) => msg.encode(buf),
            Message::TrieNodes(msg) => msg.encode(buf),
            #[cfg(feature = "l2")]
            Message::L2(l2_msg) => match l2_msg {
                L2Message::BatchSealed(msg) => msg.encode(buf),
                L2Message::NewBlock(msg) => msg.encode(buf),
            },
        }
    }

    pub fn request_id(&self) -> Option<u64> {
        match self {
            Message::GetBlockHeaders(message) => Some(message.id),
            Message::GetBlockBodies(message) => Some(message.id),
            Message::GetPooledTransactions(message) => Some(message.id),
            Message::GetReceipts(message) => Some(message.id),
            Message::GetAccountRange(message) => Some(message.id),
            Message::GetStorageRanges(message) => Some(message.id),
            Message::GetByteCodes(message) => Some(message.id),
            Message::GetTrieNodes(message) => Some(message.id),
            Message::BlockHeaders(message) => Some(message.id),
            Message::BlockBodies(message) => Some(message.id),
            Message::PooledTransactions(message) => Some(message.id),
            Message::Receipts68(message) => Some(message.id),
            Message::Receipts69(message) => Some(message.id),
            Message::AccountRange(message) => Some(message.id),
            Message::StorageRanges(message) => Some(message.id),
            Message::ByteCodes(message) => Some(message.id),
            Message::TrieNodes(message) => Some(message.id),
            // The rest of the message types does not have a request id.
            Message::Hello(_)
            | Message::Disconnect(_)
            | Message::Ping(_)
            | Message::Pong(_)
            | Message::Status68(_)
            | Message::Status69(_)
            | Message::Transactions(_)
            | Message::NewPooledTransactionHashes(_)
            | Message::BlockRangeUpdate(_) => None,
            #[cfg(feature = "l2")]
            Message::L2(_) => None,
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
            Message::Status68(_) => "eth:Status(68)".fmt(f),
            Message::Status69(_) => "eth:Status(69)".fmt(f),
            Message::GetBlockHeaders(_) => "eth:getBlockHeaders".fmt(f),
            Message::BlockHeaders(_) => "eth:BlockHeaders".fmt(f),
            Message::BlockBodies(_) => "eth:BlockBodies".fmt(f),
            Message::NewPooledTransactionHashes(_) => "eth:NewPooledTransactionHashes".fmt(f),
            Message::GetPooledTransactions(_) => "eth::GetPooledTransactions".fmt(f),
            Message::PooledTransactions(_) => "eth::PooledTransactions".fmt(f),
            Message::Transactions(_) => "eth:TransactionsMessage".fmt(f),
            Message::GetBlockBodies(_) => "eth:GetBlockBodies".fmt(f),
            Message::GetReceipts(_) => "eth:GetReceipts".fmt(f),
            Message::Receipts68(_) => "eth:Receipts(68)".fmt(f),
            Message::Receipts69(_) => "eth:Receipts(69)".fmt(f),
            Message::BlockRangeUpdate(_) => "eth:BlockRangeUpdate".fmt(f),
            Message::GetAccountRange(_) => "snap:GetAccountRange".fmt(f),
            Message::AccountRange(_) => "snap:AccountRange".fmt(f),
            Message::GetStorageRanges(_) => "snap:GetStorageRanges".fmt(f),
            Message::StorageRanges(_) => "snap:StorageRanges".fmt(f),
            Message::GetByteCodes(_) => "snap:GetByteCodes".fmt(f),
            Message::ByteCodes(_) => "snap:ByteCodes".fmt(f),
            Message::GetTrieNodes(_) => "snap:GetTrieNodes".fmt(f),
            Message::TrieNodes(_) => "snap:TrieNodes".fmt(f),
            #[cfg(feature = "l2")]
            Message::L2(l2_msg) => match l2_msg {
                L2Message::BatchSealed(_) => "based:BatchSealed".fmt(f),
                L2Message::NewBlock(_) => "based:NewBlock".fmt(f),
            },
        }
    }
}
