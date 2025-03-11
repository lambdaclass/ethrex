use bytes::BufMut;
use ethrex_common::H512;
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::{RLPDecodeError, RLPEncodeError},
    structs::{Decoder, Encoder},
};
use k256::PublicKey;

use crate::rlpx::utils::{id2pubkey, snappy_decompress};

use super::{
    message::RLPxMessage,
    utils::{pubkey2id, snappy_compress},
};

#[derive(Debug, Clone, PartialEq)]
pub enum Capability {
    P2p,
    Eth,
    Snap,
    UnsupportedCapability(String),
}

impl RLPEncode for Capability {
    fn encode(&self, buf: &mut dyn BufMut) {
        match self {
            Self::P2p => "p2p".encode(buf),
            Self::Eth => "eth".encode(buf),
            Self::Snap => "snap".encode(buf),
            Self::UnsupportedCapability(name) => name.encode(buf),
        }
    }
}

impl RLPDecode for Capability {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (cap_string, rest) = String::decode_unfinished(rlp)?;
        match cap_string.as_str() {
            "p2p" => Ok((Capability::P2p, rest)),
            "eth" => Ok((Capability::Eth, rest)),
            "snap" => Ok((Capability::Snap, rest)),
            other => Ok((Capability::UnsupportedCapability(other.to_string()), rest)),
        }
    }
}

#[derive(Debug)]
pub(crate) struct HelloMessage {
    pub(crate) capabilities: Vec<(Capability, u8)>,
    pub(crate) node_id: PublicKey,
}

impl HelloMessage {
    pub fn new(capabilities: Vec<(Capability, u8)>, node_id: PublicKey) -> Self {
        Self {
            capabilities,
            node_id,
        }
    }
}

impl RLPxMessage for HelloMessage {
    fn encode(&self, mut buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        Encoder::new(&mut buf)
            .encode_field(&5_u8) // protocolVersion
            .encode_field(&"Ethrex/0.1.0") // clientId
            .encode_field(&self.capabilities) // capabilities
            .encode_field(&0u8) // listenPort (ignored)
            .encode_field(&pubkey2id(&self.node_id)) // nodeKey
            .finish();
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        // decode hello message: [protocolVersion: P, clientId: B, capabilities, listenPort: P, nodeId: B_64, ...]
        let decoder = Decoder::new(msg_data)?;
        let (protocol_version, decoder): (u64, _) = decoder.decode_field("protocolVersion")?;

        assert_eq!(protocol_version, 5, "only protocol version 5 is supported");

        let (_client_id, decoder): (String, _) = decoder.decode_field("clientId")?;
        // TODO: store client id for debugging purposes

        // [[cap1, capVersion1], [cap2, capVersion2], ...]
        let (capabilities, decoder): (Vec<(Capability, u8)>, _) =
            decoder.decode_field("capabilities")?;

        // This field should be ignored
        let (_listen_port, decoder): (u16, _) = decoder.decode_field("listenPort")?;

        let (node_id, decoder): (H512, _) = decoder.decode_field("nodeId")?;

        // Implementations must ignore any additional list elements
        let _padding = decoder.finish_unchecked();

        Ok(Self::new(
            capabilities,
            id2pubkey(node_id).ok_or(RLPDecodeError::MalformedData)?,
        ))
    }
}

#[derive(Debug)]
pub(crate) struct DisconnectMessage {
    pub(crate) reason: Option<u8>,
}

impl DisconnectMessage {
    pub fn new(reason: Option<u8>) -> Self {
        Self { reason }
    }

    /// Returns the meaning of the disconnect reason's error code
    /// The meaning of each error code is defined by the spec: https://github.com/ethereum/devp2p/blob/master/rlpx.md#disconnect-0x01
    pub fn reason(&self) -> &str {
        if let Some(reason) = self.reason {
            match reason {
                0x00 => "Disconnect requested",
                0x01 => "TCP sub-system error",
                0x02 => "Breach of protocol, e.g. a malformed message, bad RLP, ...",
                0x03 => "Useless peer",
                0x04 => "Too many peers",
                0x05 => "Already connected",
                0x06 => "Incompatible P2P protocol version",
                0x07 => "Null node identity received - this is automatically invalid",
                0x08 => "Client quitting",
                0x09 => "Unexpected identity in handshake",
                0x0a => "Identity is the same as this node (i.e. connected to itself)",
                0x0b => "Ping timeout",
                0x10 => "Some other reason specific to a subprotocol",
                _ => "Unknown Reason",
            }
        } else {
            "Reason Not Provided"
        }
    }
}

impl RLPxMessage for DisconnectMessage {
    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        // Disconnect msg_data is reason or none
        match self.reason {
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

        Ok(Self::new(reason))
    }
}

#[derive(Debug)]
pub(crate) struct PingMessage {}

impl PingMessage {
    pub fn new() -> Self {
        Self {}
    }
}

impl RLPxMessage for PingMessage {
    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        // Ping msg_data is only []
        Vec::<u8>::new().encode(&mut encoded_data);
        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        // decode ping message: data is empty list [] but it is snappy compressed
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let result = decoder.finish_unchecked();
        let empty: &[u8] = &[];
        assert_eq!(result, empty, "Ping msg_data should be &[]");
        Ok(Self::new())
    }
}

#[derive(Debug)]
pub(crate) struct PongMessage {}

impl PongMessage {
    pub fn new() -> Self {
        Self {}
    }
}

impl RLPxMessage for PongMessage {
    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        // Pong msg_data is only []
        Vec::<u8>::new().encode(&mut encoded_data);
        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        // decode pong message: data is empty list [] but it is snappy compressed
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let result = decoder.finish_unchecked();
        let empty: &[u8] = &[];
        assert_eq!(result, empty, "Pong msg_data should be &[]");
        Ok(Self::new())
    }
}
