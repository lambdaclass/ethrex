use super::{
    message::RLPxMessage,
    utils::{decompress_pubkey, snappy_compress},
};
use crate::rlpx::utils::{compress_pubkey, snappy_decompress};
use bytes::BufMut;
use ethrex_common::H512;
use ethrex_rlp::structs::{Decoder, Encoder};
use ethrex_rlp::{
    decode::{decode_rlp_item, RLPDecode},
    encode::RLPEncode,
    error::{RLPDecodeError, RLPEncodeError},
};
use k256::PublicKey;
use serde::Serialize;

pub const SUPPORTED_ETH_CAPABILITIES: [Capability; 1] = [Capability::eth(68)];
pub const SUPPORTED_SNAP_CAPABILITIES: [Capability; 1] = [Capability::snap(1)];
pub const SUPPORTED_P2P_CAPABILITIES: [Capability; 1] = [Capability::p2p(5)];

#[derive(Debug, Clone, PartialEq)]
pub struct Capability {
    pub protocol: &'static str,
    pub version: u8,
}

impl Capability {
    pub const fn eth(version: u8) -> Self {
        Capability {
            protocol: "eth",
            version,
        }
    }

    pub const fn p2p(version: u8) -> Self {
        Capability {
            protocol: "p2p",
            version,
        }
    }

    pub const fn snap(version: u8) -> Self {
        Capability {
            protocol: "snap",
            version,
        }
    }
}

impl RLPEncode for Capability {
    fn encode(&self, buf: &mut dyn BufMut) {
        Encoder::new(buf)
            .encode_field(&self.protocol)
            .encode_field(&self.version)
            .finish();
    }
}

impl RLPDecode for Capability {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (protocol, rest) = String::decode_unfinished(&rlp[1..])?;
        let (version, rest) = u8::decode_unfinished(rest)?;
        match protocol.as_str() {
            "eth" => Ok((Capability::eth(version), rest)),
            "p2p" => Ok((Capability::p2p(version), rest)),
            "snap" => Ok((Capability::snap(version), rest)),
            _ => Err(RLPDecodeError::MalformedData),
        }
    }
}

impl Serialize for Capability {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&format!("{}/{}", self.protocol, self.version))
    }
}

#[derive(Debug)]
pub(crate) struct HelloMessage {
    pub(crate) capabilities: Vec<Capability>,
    pub(crate) node_id: PublicKey,
    pub(crate) client_id: String,
}

impl HelloMessage {
    pub fn new(capabilities: Vec<Capability>, node_id: PublicKey, client_id: String) -> Self {
        Self {
            capabilities,
            node_id,
            client_id,
        }
    }
}

impl RLPxMessage for HelloMessage {
    const CODE: u8 = 0x00;
    fn encode(&self, mut buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        Encoder::new(&mut buf)
            .encode_field(&5_u8) // protocolVersion
            .encode_field(&self.client_id) // clientId
            .encode_field(&self.capabilities) // capabilities
            .encode_field(&0u8) // listenPort (ignored)
            .encode_field(&decompress_pubkey(&self.node_id)) // nodeKey
            .finish();
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        // decode hello message: [protocolVersion: P, clientId: B, capabilities, listenPort: P, nodeId: B_64, ...]
        let decoder = Decoder::new(msg_data)?;
        let (protocol_version, decoder): (u64, _) = decoder.decode_field("protocolVersion")?;

        assert_eq!(protocol_version, 5, "only protocol version 5 is supported");

        let (client_id, decoder): (String, _) = decoder.decode_field("clientId")?;

        // [[cap1, capVersion1], [cap2, capVersion2], ...]
        let (capabilities, decoder): (Vec<Capability>, _) = decoder.decode_field("capabilities")?;

        // This field should be ignored
        let (_listen_port, decoder): (u16, _) = decoder.decode_field("listenPort")?;

        let (node_id, decoder): (H512, _) = decoder.decode_field("nodeId")?;

        // Implementations must ignore any additional list elements
        let _padding = decoder.finish_unchecked();

        Ok(Self::new(
            capabilities,
            compress_pubkey(node_id).ok_or(RLPDecodeError::MalformedData)?,
            client_id,
        ))
    }
}

// Create disconnectreason enum
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DisconnectReason {
    DisconnectRequested = 0x00,
    NetworkError = 0x01,
    ProtocolError = 0x02,
    UselessPeer = 0x03,
    TooManyPeers = 0x04,
    AlreadyConnected = 0x05,
    IncompatibleVersion = 0x06,
    InvalidIdentity = 0x07,
    ClientQuitting = 0x08,
    UnexpectedIdentity = 0x09,
    SelfIdentity = 0x0a,
    PingTimeout = 0x0b,
    SubprotocolError = 0x10,
    InvalidReason = 0xff,
}

// impl display for disconnectreason
impl std::fmt::Display for DisconnectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DisconnectReason::DisconnectRequested => write!(f, "Disconnect Requested"),
            DisconnectReason::NetworkError => write!(f, "TCP Subsystem Error"),
            DisconnectReason::ProtocolError => write!(f, "Breach of Protocol"),
            DisconnectReason::UselessPeer => write!(f, "Useless Peer"),
            DisconnectReason::TooManyPeers => write!(f, "Too Many Peers"),
            DisconnectReason::AlreadyConnected => write!(f, "Already Connected"),
            DisconnectReason::IncompatibleVersion => {
                write!(f, "Incompatible P2P Protocol Version")
            }
            DisconnectReason::InvalidIdentity => write!(f, "Null Node Identity Received"),
            DisconnectReason::ClientQuitting => write!(f, "Client Quitting"),
            DisconnectReason::UnexpectedIdentity => {
                write!(f, "Unexpected Identity in Handshake")
            }
            DisconnectReason::SelfIdentity => {
                write!(f, "Identity is the Same as This Node")
            }
            DisconnectReason::PingTimeout => write!(f, "Ping Timeout"),
            DisconnectReason::SubprotocolError => {
                write!(f, "Some Other Reason Specific to a Subprotocol")
            }
            DisconnectReason::InvalidReason => write!(f, "Invalid Disconnect Reason"),
        }
    }
}

impl From<u8> for DisconnectReason {
    fn from(value: u8) -> Self {
        match value {
            0x00 => DisconnectReason::DisconnectRequested,
            0x01 => DisconnectReason::NetworkError,
            0x02 => DisconnectReason::ProtocolError,
            0x03 => DisconnectReason::UselessPeer,
            0x04 => DisconnectReason::TooManyPeers,
            0x05 => DisconnectReason::AlreadyConnected,
            0x06 => DisconnectReason::IncompatibleVersion,
            0x07 => DisconnectReason::InvalidIdentity,
            0x08 => DisconnectReason::ClientQuitting,
            0x09 => DisconnectReason::UnexpectedIdentity,
            0x0a => DisconnectReason::SelfIdentity,
            0x0b => DisconnectReason::PingTimeout,
            0x10 => DisconnectReason::SubprotocolError,
            _ => DisconnectReason::InvalidReason,
        }
    }
}

impl From<DisconnectReason> for u8 {
    fn from(val: DisconnectReason) -> Self {
        val as u8
    }
}
#[derive(Debug)]
pub(crate) struct DisconnectMessage {
    pub(crate) reason: Option<DisconnectReason>,
}

impl DisconnectMessage {
    pub fn new(reason: Option<DisconnectReason>) -> Self {
        Self { reason }
    }

    /// Returns the meaning of the disconnect reason's error code
    /// The meaning of each error code is defined by the spec: https://github.com/ethereum/devp2p/blob/master/rlpx.md#disconnect-0x01
    pub fn reason(&self) -> DisconnectReason {
        self.reason.unwrap_or(DisconnectReason::InvalidReason)
    }
}

impl RLPxMessage for DisconnectMessage {
    const CODE: u8 = 0x01;
    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        // Disconnect msg_data is reason or none
        match self.reason.map(Into::<u8>::into) {
            Some(value) => Encoder::new(&mut encoded_data)
                .encode_field(&value)
                .finish(),
            None => Vec::<u8>::new().encode(&mut encoded_data),
        }
        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        // decode disconnect message: [reason (optional)]
        // The msg data may be compressed or not
        let msg_data = if let Ok(decompressed) = snappy_decompress(msg_data) {
            decompressed
        } else {
            msg_data.to_vec()
        };
        // It seems that disconnect reason can be encoded in different ways:
        let reason = match msg_data.len() {
            0 => None,
            // As a single u8
            1 => Some(msg_data[0]),
            // As an RLP encoded Vec<u8>
            _ => {
                let decoder = Decoder::new(&msg_data)?;
                let (reason, _): (Option<u8>, _) = decoder.decode_optional_field();
                reason
            }
        };

        Ok(Self::new(reason.map(|r| r.into())))
    }
}

#[derive(Debug)]
pub(crate) struct PingMessage {}

impl RLPxMessage for PingMessage {
    const CODE: u8 = 0x02;
    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        // Ping msg_data is only []
        Vec::<u8>::new().encode(&mut encoded_data);
        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        // decode ping message: data is empty list [] or string but it is snappy compressed
        let decompressed_data = snappy_decompress(msg_data)?;
        let (_, payload, remaining) = decode_rlp_item(&decompressed_data)?;

        let empty: &[u8] = &[];
        assert_eq!(payload, empty, "Ping payload should be &[]");
        assert_eq!(remaining, empty, "Ping remaining should be &[]");
        Ok(Self {})
    }
}

#[derive(Debug)]
pub(crate) struct PongMessage {}

impl RLPxMessage for PongMessage {
    const CODE: u8 = 0x03;
    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        // Pong msg_data is only []
        Vec::<u8>::new().encode(&mut encoded_data);
        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        // decode pong message: data is empty list [] or string but it is snappy compressed
        let decompressed_data = snappy_decompress(msg_data)?;
        let (_, payload, remaining) = decode_rlp_item(&decompressed_data)?;

        let empty: &[u8] = &[];
        assert_eq!(payload, empty, "Pong payload should be &[]");
        assert_eq!(remaining, empty, "Pong remaining should be &[]");
        Ok(Self {})
    }
}
