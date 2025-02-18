use ethrex_core::H512;
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    structs::{Decoder, Encoder},
};

use crate::peer::packet::Packet;

use super::Capability;

pub struct Hello {
    protocol_version: u8,
    client_id: String,
    capabilities: Vec<Capability>,
    listen_port: u16,
    node_id: H512,
}

impl RLPEncode for Hello {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.protocol_version)
            .encode_field(&self.client_id)
            .encode_field(&self.capabilities)
            .encode_field(&self.listen_port)
            .encode_field(&self.node_id)
            .finish();
    }
}

impl Packet for Hello {
    fn try_rlp_decode(rlp: &[u8]) -> Result<(Self, &[u8]), ethrex_rlp::error::RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;

        let (protocol_version, decoder) = decoder.decode_field("signature")?;
        let (client_id, decoder) = decoder.decode_field("initiator_pubkey")?;
        let (capabilities, decoder) = decoder.decode_field("nonce")?;
        let (listen_port, decoder) = decoder.decode_field("version")?;
        let (node_id, decoder) = decoder.decode_field("version")?;
        Ok((
            Hello {
                protocol_version,
                client_id,
                capabilities,
                listen_port,
                node_id,
            },
            decoder.finish_unchecked(),
        ))
    }
}

#[derive(Clone, Copy)]
pub enum DisconnectReason {
    DisconnectRequested = 0x00,
    TCPError,
    BadProtocol,
    UselessPeer,
    TooManyPeers,
    AlreadyConnected,
    IncompatibleP2PProtocol,
    NullIdentity,
    ClientQuit,
    UnexpectedIdentity,
    SameIdentity,
    PingTimeout = 0x0b,
    Other = 0x10,
}

impl From<DisconnectReason> for u8 {
    fn from(value: DisconnectReason) -> Self {
        match value {
            DisconnectReason::DisconnectRequested => 0x00,
            DisconnectReason::TCPError => 0x01,
            DisconnectReason::BadProtocol => 0x02,
            DisconnectReason::UselessPeer => 0x03,
            DisconnectReason::TooManyPeers => 0x04,
            DisconnectReason::AlreadyConnected => 0x05,
            DisconnectReason::IncompatibleP2PProtocol => 0x06,
            DisconnectReason::NullIdentity => 0x07,
            DisconnectReason::ClientQuit => 0x08,
            DisconnectReason::UnexpectedIdentity => 0x09,
            DisconnectReason::SameIdentity => 0x0a,
            DisconnectReason::PingTimeout => 0x0b,
            DisconnectReason::Other => 0x10,
        }
    }
}

impl RLPEncode for DisconnectReason {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf).encode_field(&u8::from(*self)).finish();
    }
}

impl RLPDecode for DisconnectReason {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), ethrex_rlp::error::RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;

        let (value, decoder) = decoder.decode_field("value")?;
        Ok((DisconnectReason::from(value), decoder.finish_unchecked()))
    }
}

pub struct Disconnect {
    reason: DisconnectReason,
}

impl RLPEncode for Disconnect {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf).encode_field(&self.reason).finish();
    }
}

impl Packet for Disconnect {
    fn try_rlp_decode(rlp: &[u8]) -> Result<(Self, &[u8]), ethrex_rlp::error::RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;

        let (reason, decoder) = decoder.decode_field("reason")?;
        Ok((Disconnect { reason }, decoder.finish_unchecked()))
    }
}

pub struct Ping;

impl RLPEncode for Ping {
    fn encode(&self, _buf: &mut dyn bytes::BufMut) {}
}

impl Packet for Ping {
    fn try_rlp_decode(rlp: &[u8]) -> Result<(Self, &[u8]), ethrex_rlp::error::RLPDecodeError> {
        Ok((Ping, rlp))
    }
}

pub struct Pong;

impl RLPEncode for Pong {
    fn encode(&self, _buf: &mut dyn bytes::BufMut) {}
}

impl Packet for Pong {
    fn try_rlp_decode(rlp: &[u8]) -> Result<(Self, &[u8]), ethrex_rlp::error::RLPDecodeError> {
        Ok((Pong, rlp))
    }
}
