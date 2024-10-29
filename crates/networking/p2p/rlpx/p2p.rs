use bytes::BufMut;
use ethereum_rust_core::H512;
use ethereum_rust_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::{RLPDecodeError, RLPEncodeError},
    structs::{Decoder, Encoder},
};
use k256::PublicKey;
use snap::raw::Decoder as SnappyDecoder;

use crate::rlpx::utils::id2pubkey;

use super::{
    message::RLPxMessage,
    utils::{pubkey2id, snappy_encode},
};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Capability {
    P2p,
    Eth,
    Snap,
}

impl RLPEncode for Capability {
    fn encode(&self, buf: &mut dyn BufMut) {
        match self {
            Self::P2p => "p2p".encode(buf),
            Self::Eth => "eth".encode(buf),
            Self::Snap => "snap".encode(buf),
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
            _ => Err(RLPDecodeError::UnexpectedString),
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
        0_u8.encode(buf); //msg_id
        Encoder::new(&mut buf)
            .encode_field(&5_u8) // protocolVersion
            .encode_field(&"Ethereum(++)/1.0.0") // clientId
            .encode_field(&self.capabilities) // capabilities
            .encode_field(&0u8) // listenPort (ignored)
            .encode_field(&pubkey2id(&self.node_id)) // nodeKey
            .finish();
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        // decode hello message: [protocolVersion: P, clientId: B, capabilities, listenPort: P, nodeId: B_64, ...]
        let decoder = Decoder::new(msg_data).unwrap();
        let (protocol_version, decoder): (u64, _) =
            decoder.decode_field("protocolVersion").unwrap();

        assert_eq!(protocol_version, 5, "only protocol version 5 is supported");

        let (_client_id, decoder): (String, _) = decoder.decode_field("clientId").unwrap();
        // TODO: store client id for debugging purposes

        // [[cap1, capVersion1], [cap2, capVersion2], ...]
        let (capabilities, decoder): (Vec<(Capability, u8)>, _) =
            decoder.decode_field("capabilities").unwrap();

        // This field should be ignored
        let (_listen_port, decoder): (u16, _) = decoder.decode_field("listenPort").unwrap();

        let (node_id, decoder): (H512, _) = decoder.decode_field("nodeId").unwrap();

        // Implementations must ignore any additional list elements
        let _padding = decoder.finish_unchecked();

        Ok(Self::new(capabilities, id2pubkey(node_id).unwrap()))
    }
}

#[derive(Debug)]
pub(crate) struct DisconnectMessage {
    reason: Option<u8>,
}

impl DisconnectMessage {
    pub fn new(reason: Option<u8>) -> Self {
        Self { reason }
    }
}

impl RLPxMessage for DisconnectMessage {
    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        1_u8.encode(buf); //msg_id

        let mut encoded_data = vec![];
        // Disconnect msg_data is reason or none
        match self.reason {
            Some(value) => Encoder::new(&mut encoded_data)
                .encode_field(&value)
                .finish(),
            None => Vec::<u8>::new().encode(&mut encoded_data),
        }
        let msg_data = snappy_encode(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        // decode disconnect message: [reason (optional)]
        let mut snappy_decoder = SnappyDecoder::new();
        let decompressed_data = snappy_decoder.decompress_vec(msg_data).unwrap();
        // It seems that disconnect reason can be encoded in different ways:
        // TODO: it may be not compressed at all. We should check that case
        let reason = match decompressed_data.len() {
            0 => None,
            // As a single u8
            1 => Some(decompressed_data[0]),
            // As an RLP encoded Vec<u8>
            _ => {
                let decoder = Decoder::new(&decompressed_data)?;
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
        2_u8.encode(buf); // msg_id

        let mut encoded_data = vec![];
        // Ping msg_data is only []
        Vec::<u8>::new().encode(&mut encoded_data);
        let msg_data = snappy_encode(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        // decode ping message: data is empty list [] but it is snappy compressed
        let mut snappy_decoder = SnappyDecoder::new();
        let decompressed_data = snappy_decoder.decompress_vec(msg_data).unwrap();
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
        2_u8.encode(buf); // msg_id

        let mut encoded_data = vec![];
        // Pong msg_data is only []
        Vec::<u8>::new().encode(&mut encoded_data);
        let msg_data = snappy_encode(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        // decode pong message: data is empty list [] but it is snappy compressed
        let mut snappy_decoder = SnappyDecoder::new();
        let decompressed_data = snappy_decoder.decompress_vec(msg_data).unwrap();
        let decoder = Decoder::new(&decompressed_data)?;
        let result = decoder.finish_unchecked();
        let empty: &[u8] = &[];
        assert_eq!(result, empty, "Pong msg_data should be &[]");
        Ok(Self::new())
    }
}
