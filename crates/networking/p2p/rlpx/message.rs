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

#[allow(clippy::upper_case_acronyms)]
enum MessageProtocol {
    P2P,
    ETH,
    SNAP,
}

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
    fn protocol(&self) -> MessageProtocol {
        match self {
            Message::Hello(_) => MessageProtocol::P2P,
            Message::Disconnect(_) => MessageProtocol::P2P,
            Message::Ping(_) => MessageProtocol::P2P,
            Message::Pong(_) => MessageProtocol::P2P,

            // eth capability
            Message::Status(_) => MessageProtocol::ETH,
            Message::Transactions(_) => MessageProtocol::ETH,
            Message::GetBlockHeaders(_) => MessageProtocol::ETH,
            Message::BlockHeaders(_) => MessageProtocol::ETH,
            Message::GetBlockBodies(_) => MessageProtocol::ETH,
            Message::BlockBodies(_) => MessageProtocol::ETH,
            Message::NewPooledTransactionHashes(_) => MessageProtocol::ETH,
            Message::GetPooledTransactions(_) => MessageProtocol::ETH,
            Message::PooledTransactions(_) => MessageProtocol::ETH,
            Message::GetReceipts(_) => MessageProtocol::ETH,
            Message::Receipts(_) => MessageProtocol::ETH,
            Message::BlockRangeUpdate(_) => MessageProtocol::ETH,
            // snap capability
            Message::GetAccountRange(_) => MessageProtocol::SNAP,
            Message::AccountRange(_) => MessageProtocol::SNAP,
            Message::GetStorageRanges(_) => MessageProtocol::SNAP,
            Message::StorageRanges(_) => MessageProtocol::SNAP,
            Message::GetByteCodes(_) => MessageProtocol::SNAP,
            Message::ByteCodes(_) => MessageProtocol::SNAP,
            Message::GetTrieNodes(_) => MessageProtocol::SNAP,
            Message::TrieNodes(_) => MessageProtocol::SNAP,
        }
    }

    pub fn code(&self) -> u8 {
        match self {
            Message::Hello(_) => HelloMessage::CODE,
            Message::Disconnect(_) => DisconnectMessage::CODE,
            Message::Ping(_) => PingMessage::CODE,
            Message::Pong(_) => PongMessage::CODE,

            // eth capability
            Message::Status(_) => StatusMessage::CODE,
            Message::Transactions(_) => Transactions::CODE,
            Message::GetBlockHeaders(_) => GetBlockHeaders::CODE,
            Message::BlockHeaders(_) => BlockHeaders::CODE,
            Message::GetBlockBodies(_) => GetBlockBodies::CODE,
            Message::BlockBodies(_) => BlockBodies::CODE,
            Message::NewPooledTransactionHashes(_) => NewPooledTransactionHashes::CODE,
            Message::GetPooledTransactions(_) => GetPooledTransactions::CODE,
            Message::PooledTransactions(_) => PooledTransactions::CODE,
            Message::GetReceipts(_) => GetReceipts::CODE,
            Message::Receipts(_) => Receipts::CODE,
            Message::BlockRangeUpdate(_) => BlockRangeUpdate::CODE,
            // snap capability
            Message::GetAccountRange(_) => GetAccountRange::CODE,
            Message::AccountRange(_) => AccountRange::CODE,
            Message::GetStorageRanges(_) => GetStorageRanges::CODE,
            Message::StorageRanges(_) => StorageRanges::CODE,
            Message::GetByteCodes(_) => GetByteCodes::CODE,
            Message::ByteCodes(_) => ByteCodes::CODE,
            Message::GetTrieNodes(_) => GetTrieNodes::CODE,
            Message::TrieNodes(_) => TrieNodes::CODE,
        }
    }

    pub fn offset(
        &self,
        p2p_capability: &Option<Capability>,
        eth_capability: &Option<Capability>,
        snap_capability: &Option<Capability>,
    ) -> Result<u8, RLPEncodeError> {
        match self.protocol() {
            MessageProtocol::P2P => {
                if let Some(p2p_capability) = p2p_capability {
                    if self.code() < p2p_capability.length() {
                        return Ok(0);
                    }
                }
            }
            MessageProtocol::ETH => {
                if let (Some(p2p_capability), Some(eth_capability)) =
                    (p2p_capability, eth_capability)
                {
                    if self.code() < eth_capability.length() {
                        return Ok(p2p_capability.length());
                    }
                }
            }
            MessageProtocol::SNAP => {
                if let (Some(p2p_capability), Some(eth_capability), Some(snap_capability)) =
                    (p2p_capability, eth_capability, snap_capability)
                {
                    if self.code() < snap_capability.length() {
                        return Ok(p2p_capability.length() + eth_capability.length());
                    }
                }
            }
        }
        Err(RLPEncodeError::Custom("TODO".into()))
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
                    Receipts::CODE => match eth_capability.version {
                        68 => Ok(Message::Receipts(Receipts::decode68(data)?)),
                        69 => Ok(Message::Receipts(Receipts::decode(data)?)),
                        _ => Err(RLPDecodeError::MalformedData),
                    },

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

        Err(RLPDecodeError::MalformedData)
    }

    pub fn encode(
        &self,
        buf: &mut dyn BufMut,
        p2p_capability: &Option<Capability>,
        eth_capability: &Option<Capability>,
        snap_capability: &Option<Capability>,
    ) -> Result<(), RLPEncodeError> {
        (self.code() + self.offset(p2p_capability, eth_capability, snap_capability)?).encode(buf);
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
            Message::Receipts(msg) => {
                if let Some(eth_capability) = eth_capability {
                    match eth_capability.version {
                        68 => msg.encode68(buf),
                        69 => msg.encode(buf),
                        _ => Err(RLPEncodeError::Custom("TODO".into())),
                    }
                } else {
                    Err(RLPEncodeError::Custom("TODO".into()))
                }
            }
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
