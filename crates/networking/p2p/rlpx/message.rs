use bytes::BufMut;
use ethrex_rlp::error::{RLPDecodeError, RLPEncodeError};
use std::fmt::Display;

use super::eth::blocks::{BlockBodies, BlockHeaders, GetBlockBodies, GetBlockHeaders};
use super::eth::receipts::{GetReceipts, Receipts};
use super::eth::status::StatusMessage;
use super::eth::transactions::{
    GetPooledTransactions, NewPooledTransactionHashes, PooledTransactions, Transactions,
};
use super::eth::update::BlockRangeUpdate;
use super::l2::messages::{BatchSealed, L2Message, NewBlock};
use super::l2::{self, messages};
use super::p2p::{DisconnectMessage, HelloMessage, PingMessage, PongMessage};
use super::snap::{
    AccountRange, ByteCodes, GetAccountRange, GetByteCodes, GetStorageRanges, GetTrieNodes,
    StorageRanges, TrieNodes,
};

use ethrex_rlp::encode::RLPEncode;

const ETH_CAPABILITY_OFFSET: u8 = 0x10;
const SNAP_CAPABILITY_OFFSET: u8 = 0x21;
const BASED_CAPABILITY_OFFSET: u8 = 0x30;

pub trait RLPxMessage: Sized {
    const CODE: u8;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError>;

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError>;
}
#[derive(Debug, Clone)]
pub(crate) enum Message {
    Hello(HelloMessage),
    Disconnect(DisconnectMessage),
    Ping(PingMessage),
    Pong(PongMessage),
    Status(StatusMessage),
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
    Receipts(Receipts),
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
    L2(messages::L2Message),
}

impl Message {
    pub const fn code(&self) -> u8 {
        match self {
            Message::Hello(_) => HelloMessage::CODE,
            Message::Disconnect(_) => DisconnectMessage::CODE,
            Message::Ping(_) => PingMessage::CODE,
            Message::Pong(_) => PongMessage::CODE,

            // eth capability
            Message::Status(_) => ETH_CAPABILITY_OFFSET + StatusMessage::CODE,
            Message::Transactions(_) => ETH_CAPABILITY_OFFSET + Transactions::CODE,
            Message::GetBlockHeaders(_) => ETH_CAPABILITY_OFFSET + GetBlockHeaders::CODE,
            Message::BlockHeaders(_) => ETH_CAPABILITY_OFFSET + BlockHeaders::CODE,
            Message::GetBlockBodies(_) => ETH_CAPABILITY_OFFSET + GetBlockBodies::CODE,
            Message::BlockBodies(_) => ETH_CAPABILITY_OFFSET + BlockBodies::CODE,
            Message::NewPooledTransactionHashes(_) => {
                ETH_CAPABILITY_OFFSET + NewPooledTransactionHashes::CODE
            }
            Message::GetPooledTransactions(_) => {
                ETH_CAPABILITY_OFFSET + GetPooledTransactions::CODE
            }
            Message::PooledTransactions(_) => ETH_CAPABILITY_OFFSET + PooledTransactions::CODE,
            Message::GetReceipts(_) => ETH_CAPABILITY_OFFSET + GetReceipts::CODE,
            Message::Receipts(_) => ETH_CAPABILITY_OFFSET + Receipts::CODE,
            Message::BlockRangeUpdate(_) => ETH_CAPABILITY_OFFSET + BlockRangeUpdate::CODE,
            // snap capability
            Message::GetAccountRange(_) => SNAP_CAPABILITY_OFFSET + GetAccountRange::CODE,
            Message::AccountRange(_) => SNAP_CAPABILITY_OFFSET + AccountRange::CODE,
            Message::GetStorageRanges(_) => SNAP_CAPABILITY_OFFSET + GetStorageRanges::CODE,
            Message::StorageRanges(_) => SNAP_CAPABILITY_OFFSET + StorageRanges::CODE,
            Message::GetByteCodes(_) => SNAP_CAPABILITY_OFFSET + GetByteCodes::CODE,
            Message::ByteCodes(_) => SNAP_CAPABILITY_OFFSET + ByteCodes::CODE,
            Message::GetTrieNodes(_) => SNAP_CAPABILITY_OFFSET + GetTrieNodes::CODE,
            Message::TrieNodes(_) => SNAP_CAPABILITY_OFFSET + TrieNodes::CODE,

            // based capability
            Message::L2(l2_msg) => {
                BASED_CAPABILITY_OFFSET + {
                    match l2_msg {
                        L2Message::NewBlock(_) => NewBlock::CODE,
                        L2Message::BatchSealed(_) => BatchSealed::CODE,
                    }
                }
            }
        }
    }
    pub fn decode(msg_id: u8, data: &[u8]) -> Result<Message, RLPDecodeError> {
        if msg_id < ETH_CAPABILITY_OFFSET {
            match msg_id {
                HelloMessage::CODE => Ok(Message::Hello(HelloMessage::decode(data)?)),
                DisconnectMessage::CODE => {
                    Ok(Message::Disconnect(DisconnectMessage::decode(data)?))
                }
                PingMessage::CODE => Ok(Message::Ping(PingMessage::decode(data)?)),
                PongMessage::CODE => Ok(Message::Pong(PongMessage::decode(data)?)),
                _ => Err(RLPDecodeError::MalformedData),
            }
        } else if msg_id < SNAP_CAPABILITY_OFFSET {
            // eth capability
            match msg_id - ETH_CAPABILITY_OFFSET {
                StatusMessage::CODE => Ok(Message::Status(StatusMessage::decode(data)?)),
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
                Receipts::CODE => Ok(Message::Receipts(Receipts::decode(data)?)),
                BlockRangeUpdate::CODE => {
                    Ok(Message::BlockRangeUpdate(BlockRangeUpdate::decode(data)?))
                }
                _ => Err(RLPDecodeError::MalformedData),
            }
        } else if msg_id < BASED_CAPABILITY_OFFSET {
            // snap capability
            match msg_id - SNAP_CAPABILITY_OFFSET {
                GetAccountRange::CODE => {
                    return Ok(Message::GetAccountRange(GetAccountRange::decode(data)?));
                }
                AccountRange::CODE => Ok(Message::AccountRange(AccountRange::decode(data)?)),
                GetStorageRanges::CODE => {
                    return Ok(Message::GetStorageRanges(GetStorageRanges::decode(data)?));
                }
                StorageRanges::CODE => Ok(Message::StorageRanges(StorageRanges::decode(data)?)),
                GetByteCodes::CODE => Ok(Message::GetByteCodes(GetByteCodes::decode(data)?)),
                ByteCodes::CODE => Ok(Message::ByteCodes(ByteCodes::decode(data)?)),
                GetTrieNodes::CODE => Ok(Message::GetTrieNodes(GetTrieNodes::decode(data)?)),
                TrieNodes::CODE => Ok(Message::TrieNodes(TrieNodes::decode(data)?)),
                _ => Err(RLPDecodeError::MalformedData),
            }
        } else {
            // based capability
            Ok(Message::L2(match msg_id - BASED_CAPABILITY_OFFSET {
                messages::NewBlock::CODE => {
                    let decoded = l2::messages::NewBlock::decode(data)?;
                    L2Message::NewBlock(decoded)
                }
                BatchSealed::CODE => {
                    let decoded = l2::messages::BatchSealed::decode(data)?;
                    L2Message::BatchSealed(decoded)
                }
                _ => return Err(RLPDecodeError::MalformedData),
            }))
        }
    }

    pub fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        self.code().encode(buf);
        match self {
            Message::Hello(msg) => msg.encode(buf),
            Message::Disconnect(msg) => msg.encode(buf),
            Message::Ping(msg) => msg.encode(buf),
            Message::Pong(msg) => msg.encode(buf),
            Message::Status(msg) => msg.encode(buf),
            Message::Transactions(msg) => msg.encode(buf),
            Message::GetBlockHeaders(msg) => msg.encode(buf),
            Message::BlockHeaders(msg) => msg.encode(buf),
            Message::GetBlockBodies(msg) => msg.encode(buf),
            Message::BlockBodies(msg) => msg.encode(buf),
            Message::NewPooledTransactionHashes(msg) => msg.encode(buf),
            Message::GetPooledTransactions(msg) => msg.encode(buf),
            Message::PooledTransactions(msg) => msg.encode(buf),
            Message::GetReceipts(msg) => msg.encode(buf),
            Message::Receipts(msg) => msg.encode(buf),
            Message::BlockRangeUpdate(msg) => msg.encode(buf),
            Message::GetAccountRange(msg) => msg.encode(buf),
            Message::AccountRange(msg) => msg.encode(buf),
            Message::GetStorageRanges(msg) => msg.encode(buf),
            Message::StorageRanges(msg) => msg.encode(buf),
            Message::GetByteCodes(msg) => msg.encode(buf),
            Message::ByteCodes(msg) => msg.encode(buf),
            Message::GetTrieNodes(msg) => msg.encode(buf),
            Message::TrieNodes(msg) => msg.encode(buf),
            Message::L2(l2_msg) => match l2_msg {
                L2Message::BatchSealed(msg) => msg.encode(buf),
                L2Message::NewBlock(msg) => msg.encode(buf),
            },
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
            Message::NewPooledTransactionHashes(_) => "eth:NewPooledTransactionHashes".fmt(f),
            Message::GetPooledTransactions(_) => "eth::GetPooledTransactions".fmt(f),
            Message::PooledTransactions(_) => "eth::PooledTransactions".fmt(f),
            Message::Transactions(_) => "eth:TransactionsMessage".fmt(f),
            Message::GetBlockBodies(_) => "eth:GetBlockBodies".fmt(f),
            Message::GetReceipts(_) => "eth:GetReceipts".fmt(f),
            Message::Receipts(_) => "eth:Receipts".fmt(f),
            Message::BlockRangeUpdate(_) => "eth:BlockRangeUpdate".fmt(f),
            Message::GetAccountRange(_) => "snap:GetAccountRange".fmt(f),
            Message::AccountRange(_) => "snap:AccountRange".fmt(f),
            Message::GetStorageRanges(_) => "snap:GetStorageRanges".fmt(f),
            Message::StorageRanges(_) => "snap:StorageRanges".fmt(f),
            Message::GetByteCodes(_) => "snap:GetByteCodes".fmt(f),
            Message::ByteCodes(_) => "snap:ByteCodes".fmt(f),
            Message::GetTrieNodes(_) => "snap:GetTrieNodes".fmt(f),
            Message::TrieNodes(_) => "snap:TrieNodes".fmt(f),
            Message::L2(l2_msg) => match l2_msg {
                L2Message::BatchSealed(_) => "based:BatchSealed".fmt(f),
                L2Message::NewBlock(_) => "based:NewBlock".fmt(f),
            },
        }
    }
}
