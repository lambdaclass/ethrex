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
use super::p2p::{Capability, DisconnectMessage, HelloMessage, PingMessage, PongMessage};
use super::snap::{
    AccountRange, ByteCodes, GetAccountRange, GetByteCodes, GetStorageRanges, GetTrieNodes,
    StorageRanges, TrieNodes,
};
use ethrex_rlp::encode::RLPEncode;

pub trait RLPxMessage: Sized {
    const CODE: u8;

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
}

impl Message {
    pub fn code(&self) -> u8 {
        let eth_offset = Capability::p2p(5).length();
        let snap_offset = eth_offset + Capability::eth(68).length();
        match self {
            Message::Hello(_) => HelloMessage::CODE,
            Message::Disconnect(_) => DisconnectMessage::CODE,
            Message::Ping(_) => PingMessage::CODE,
            Message::Pong(_) => PongMessage::CODE,

            // eth capability
            Message::Status(_) => eth_offset + StatusMessage::CODE,
            Message::Transactions(_) => eth_offset + Transactions::CODE,
            Message::GetBlockHeaders(_) => eth_offset + GetBlockHeaders::CODE,
            Message::BlockHeaders(_) => eth_offset + BlockHeaders::CODE,
            Message::GetBlockBodies(_) => eth_offset + GetBlockBodies::CODE,
            Message::BlockBodies(_) => eth_offset + BlockBodies::CODE,
            Message::NewPooledTransactionHashes(_) => eth_offset + NewPooledTransactionHashes::CODE,
            Message::GetPooledTransactions(_) => eth_offset + GetPooledTransactions::CODE,
            Message::PooledTransactions(_) => eth_offset + PooledTransactions::CODE,
            Message::GetReceipts(_) => eth_offset + GetReceipts::CODE,
            Message::Receipts(_) => eth_offset + Receipts::CODE,
            Message::BlockRangeUpdate(_) => eth_offset + BlockRangeUpdate::CODE,
            // snap capability
            Message::GetAccountRange(_) => snap_offset + GetAccountRange::CODE,
            Message::AccountRange(_) => snap_offset + AccountRange::CODE,
            Message::GetStorageRanges(_) => snap_offset + GetStorageRanges::CODE,
            Message::StorageRanges(_) => snap_offset + StorageRanges::CODE,
            Message::GetByteCodes(_) => snap_offset + GetByteCodes::CODE,
            Message::ByteCodes(_) => snap_offset + ByteCodes::CODE,
            Message::GetTrieNodes(_) => snap_offset + GetTrieNodes::CODE,
            Message::TrieNodes(_) => snap_offset + TrieNodes::CODE,
        }
    }

    pub fn decode(
        msg_id: u8,
        data: &[u8],
        p2p_capability: &Option<Capability>,
        eth_capability: &Option<Capability>,
        snap_capability: &Option<Capability>,
    ) -> Result<Message, RLPDecodeError> {
        let Some(p2p_capability) = p2p_capability else {
            return Err(RLPDecodeError::MalformedData);
        };

        // P2P protocol
        if msg_id < p2p_capability.length() {
            return match msg_id {
                HelloMessage::CODE => Ok(Message::Hello(HelloMessage::decode(data)?)),
                DisconnectMessage::CODE => {
                    Ok(Message::Disconnect(DisconnectMessage::decode(data)?))
                }
                PingMessage::CODE => Ok(Message::Ping(PingMessage::decode(data)?)),
                PongMessage::CODE => Ok(Message::Pong(PongMessage::decode(data)?)),
                _ => Err(RLPDecodeError::MalformedData),
            };
        }

        let eth_msg_id = msg_id - p2p_capability.length();

        // eth (wire) protocol
        if let Some(eth_capability) = eth_capability {
            if eth_msg_id < eth_capability.length() {
                return match eth_msg_id {
                    StatusMessage::CODE => Ok(Message::Status(StatusMessage::decode(data)?)),
                    Transactions::CODE => Ok(Message::Transactions(Transactions::decode(data)?)),
                    GetBlockHeaders::CODE => {
                        Ok(Message::GetBlockHeaders(GetBlockHeaders::decode(data)?))
                    }
                    BlockHeaders::CODE => Ok(Message::BlockHeaders(BlockHeaders::decode(data)?)),
                    GetBlockBodies::CODE => {
                        Ok(Message::GetBlockBodies(GetBlockBodies::decode(data)?))
                    }
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
                };
            } else {
                let snap_msg_id = eth_msg_id - eth_capability.length();
                if let Some(snap_capability) = snap_capability {
                    if snap_msg_id < snap_capability.length() {
                        return match snap_msg_id {
                            GetAccountRange::CODE => {
                                Ok(Message::GetAccountRange(GetAccountRange::decode(data)?))
                            }
                            AccountRange::CODE => {
                                Ok(Message::AccountRange(AccountRange::decode(data)?))
                            }
                            GetStorageRanges::CODE => {
                                Ok(Message::GetStorageRanges(GetStorageRanges::decode(data)?))
                            }
                            StorageRanges::CODE => {
                                Ok(Message::StorageRanges(StorageRanges::decode(data)?))
                            }
                            GetByteCodes::CODE => {
                                Ok(Message::GetByteCodes(GetByteCodes::decode(data)?))
                            }
                            ByteCodes::CODE => Ok(Message::ByteCodes(ByteCodes::decode(data)?)),
                            GetTrieNodes::CODE => {
                                Ok(Message::GetTrieNodes(GetTrieNodes::decode(data)?))
                            }
                            TrieNodes::CODE => Ok(Message::TrieNodes(TrieNodes::decode(data)?)),
                            _ => Err(RLPDecodeError::MalformedData),
                        };
                    }
                }
            }
        }
        return Err(RLPDecodeError::MalformedData);
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
        }
    }
}
