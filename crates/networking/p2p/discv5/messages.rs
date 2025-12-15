use std::{array::TryFromSliceError, net::IpAddr};

use aes::cipher::{KeyIvInit, StreamCipher, StreamCipherError};
use aes_gcm::{Aes128Gcm, KeyInit, aead::AeadMutInPlace};
use bytes::{BufMut, Bytes};
use ethrex_common::H256;
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};
use secp256k1::SecretKey;

use crate::types::NodeRecord;

type Aes128Ctr64BE = ctr::Ctr64BE<aes::Aes128>;

// Max and min packet sizes as defined in
// https://github.com/ethereum/devp2p/blob/master/discv5/discv5-wire.md#udp-communication
// Used for package validation
const MIN_PACKET_SIZE: usize = 63;
const MAX_PACKET_SIZE: usize = 1280;
// protocol data
const PROTOCOL_ID: &[u8] = b"discv5";
const PROTOCOL_VERSION: u16 = 0x0001;
// masking-iv size for a u128
const IV_MASKING_SIZE: usize = 16;
// static_header end limit: 23 bytes from static_header + 16 from iv_masking
const STATIC_HEADER_END: usize = IV_MASKING_SIZE + 23;

#[derive(Debug, thiserror::Error)]
pub enum PacketDecodeErr {
    #[error("RLP decoding error")]
    RLPDecodeError(#[from] RLPDecodeError),
    #[error("Invalid packet size")]
    InvalidSize,
    #[error("Invalid protocol: {0}")]
    InvalidProtocol(String),
    #[error("Stream Cipher Error: {0}")]
    ChipherError(String),
    #[error("TryFromSliceError: {0}")]
    TryFromSliceError(#[from] TryFromSliceError),
    #[error("Io Error: {0}")]
    IoError(#[from] std::io::Error),
}

impl From<StreamCipherError> for PacketDecodeErr {
    fn from(error: StreamCipherError) -> Self {
        PacketDecodeErr::ChipherError(error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Packet {
    Ordinary(Ordinary),
    WhoAreYou(WhoAreYou),
    // Handshake(Handshake),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacketHeader {
    pub static_header: Vec<u8>,
    pub flag: u8,
    pub nonce: Vec<u8>,
    pub authdata: Vec<u8>,
    /// Offset in the encoded packet where authdata ends, i.e where the header ends.
    pub header_end_offset: usize,
}

impl Packet {
    pub fn decode(
        dest_id: &H256,
        decrypt_key: &[u8],
        encoded_packet: &[u8],
    ) -> Result<Packet, PacketDecodeErr> {
        if encoded_packet.len() < MIN_PACKET_SIZE || encoded_packet.len() > MAX_PACKET_SIZE {
            return Err(PacketDecodeErr::InvalidSize);
        }

        // the packet structure is
        // masking-iv || masked-header || message
        // 16 bytes for an u128
        let masking_iv = &encoded_packet[..IV_MASKING_SIZE];

        let mut cipher = <Aes128Ctr64BE as KeyIvInit>::new(dest_id[..16].into(), masking_iv.into());

        let packet_header = Packet::decode_header(&mut cipher, encoded_packet)?;

        match packet_header.flag {
            0x00 => Ok(Packet::Ordinary(Ordinary::decode(
                masking_iv,
                packet_header.static_header,
                packet_header.authdata,
                packet_header.nonce,
                decrypt_key,
                &encoded_packet[packet_header.header_end_offset..],
            )?)),
            0x01 => Ok(Packet::WhoAreYou(WhoAreYou::decode(
                &packet_header.authdata,
            )?)),
            _ => Err(RLPDecodeError::MalformedData)?,
        }
    }

    pub fn encode(
        &self,
        buf: &mut dyn BufMut,
        masking_iv: u128,
        nonce: Vec<u8>,
        dest_id: &H256,
    ) -> Result<(), PacketDecodeErr> {
        let masking_as_bytes = masking_iv.to_be_bytes();
        buf.put_slice(&masking_as_bytes);

        let mut cipher =
            <Aes128Ctr64BE as KeyIvInit>::new(dest_id[..16].into(), masking_as_bytes[..].into());

        match self {
            Packet::Ordinary(_ordinary) => todo!(),
            Packet::WhoAreYou(who_are_you) => {
                who_are_you.encode_header(buf, &mut cipher, nonce)?;
            }
        }
        Ok(())
    }

    fn decode_header<T: StreamCipher>(
        cipher: &mut T,
        encoded_packet: &[u8],
    ) -> Result<PacketHeader, PacketDecodeErr> {
        // static header
        let mut static_header = encoded_packet[IV_MASKING_SIZE..STATIC_HEADER_END].to_vec();

        cipher.try_apply_keystream(&mut static_header)?;

        // static-header = protocol-id || version || flag || nonce || authdata-size
        //protocol check
        let protocol_id = &static_header[..6];
        let version = u16::from_be_bytes(static_header[6..8].try_into()?);
        if protocol_id != PROTOCOL_ID || version != PROTOCOL_VERSION {
            return Err(PacketDecodeErr::InvalidProtocol(
                match str::from_utf8(protocol_id) {
                    Ok(result) => format!("{} v{}", result, version),
                    Err(_) => format!("{:?} v{}", protocol_id, version),
                },
            ));
        }

        let flag = static_header[8];
        let nonce = static_header[9..21].to_vec();
        let authdata_size = u16::from_be_bytes(static_header[21..23].try_into()?) as usize;
        let authdata_end = STATIC_HEADER_END + authdata_size;
        let authdata = &mut encoded_packet[STATIC_HEADER_END..authdata_end].to_vec();

        cipher.try_apply_keystream(authdata)?;

        Ok(PacketHeader {
            static_header,
            flag,
            nonce,
            authdata: authdata.to_vec(),
            header_end_offset: authdata_end,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ordinary {
    message: Message,
}

impl Ordinary {
    pub fn decode(
        masking_iv: &[u8],
        static_header: Vec<u8>,
        authdata: Vec<u8>,
        nonce: Vec<u8>,
        decrypt_key: &[u8],
        encrypted_message: &[u8],
    ) -> Result<Ordinary, PacketDecodeErr> {
        // message    = aesgcm_encrypt(initiator-key, nonce, message-pt, message-ad)
        // message-pt = message-type || message-data
        // message-ad = masking-iv || header
        let mut message_ad = masking_iv.to_vec();
        message_ad.extend_from_slice(&static_header);
        message_ad.extend_from_slice(&authdata);

        let mut message = encrypted_message.to_vec();
        Self::decrypt(decrypt_key, nonce, &mut message, message_ad)?;

        let message = Message::decode(&message)?;
        Ok(Ordinary { message })
    }

    fn decrypt(
        key: &[u8],
        nonce: Vec<u8>,
        message: &mut Vec<u8>,
        message_ad: Vec<u8>,
    ) -> Result<(), PacketDecodeErr> {
        let mut cipher = Aes128Gcm::new(key[..16].into());
        cipher
            .decrypt_in_place(nonce.as_slice().into(), &message_ad, message)
            .map_err(|e| PacketDecodeErr::ChipherError(e.to_string()))?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhoAreYou {
    pub id_nonce: u128,
    pub enr_seq: u64,
}

impl WhoAreYou {
    fn encode_header<T: StreamCipher>(
        &self,
        buf: &mut dyn BufMut,
        cipher: &mut T,
        nonce: Vec<u8>,
    ) -> Result<(), PacketDecodeErr> {
        let mut static_header = Vec::new();
        static_header.put_slice(PROTOCOL_ID);
        static_header.put_slice(&PROTOCOL_VERSION.to_be_bytes());
        static_header.put_u8(0x01);
        static_header.put_slice(&nonce);
        static_header.put_slice(&24u16.to_be_bytes());
        cipher.try_apply_keystream(&mut static_header)?;
        buf.put_slice(&static_header);

        let mut authdata = Vec::new();
        self.encode(&mut authdata);
        cipher.try_apply_keystream(&mut authdata)?;
        buf.put_slice(&authdata);

        Ok(())
    }

    fn encode(&self, buf: &mut dyn BufMut) {
        buf.put_slice(&self.id_nonce.to_be_bytes());
        buf.put_slice(&self.enr_seq.to_be_bytes());
    }

    pub fn decode(authdata: &[u8]) -> Result<WhoAreYou, PacketDecodeErr> {
        let id_nonce = u128::from_be_bytes(authdata[..16].try_into()?);
        let enr_seq = u64::from_be_bytes(authdata[16..].try_into()?);

        Ok(WhoAreYou { id_nonce, enr_seq })
    }
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum Message {
    Ping(PingMessage),
    Pong(PongMessage),
    FindNode(FindNodeMessage),
    Nodes(NodesMessage),
    TalkReq(TalkReqMessage),
    // TODO: add the other messages
}

impl Message {
    pub fn decode(encrypted_message: &[u8]) -> Result<Message, RLPDecodeError> {
        let message_type = encrypted_message[0];
        match message_type {
            0x01 => {
                let ping = PingMessage::decode(&encrypted_message[1..])?;
                Ok(Message::Ping(ping))
            }
            0x02 => {
                let pong = PongMessage::decode(&encrypted_message[1..])?;
                Ok(Message::Pong(pong))
            }
            0x03 => {
                let find_node_msg = FindNodeMessage::decode(&encrypted_message[1..])?;
                Ok(Message::FindNode(find_node_msg))
            }
            0x04 => {
                let nodes_msg = NodesMessage::decode(&encrypted_message[1..])?;
                Ok(Message::Nodes(nodes_msg))
            }
            0x05 => {
                let talk_req_msg = TalkReqMessage::decode(&encrypted_message[1..])?;
                Ok(Message::TalkReq(talk_req_msg))
            }
            // 0x06 => {
            //     let (enr_response_msg, _rest) = ENRResponseMessage::decode_unfinished(msg)?;
            //     Ok(Message::ENRResponse(enr_response_msg))
            // }
            _ => Err(RLPDecodeError::MalformedData),
        }
    }

    pub fn encode(&self, _buf: &mut dyn BufMut, _signer: &SecretKey) {
        //TODO
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PingMessage {
    /// The request id of the sender.
    pub req_id: Vec<u8>,
    /// The ENR sequence number of the sender.
    pub enr_seq: u64,
}

impl PingMessage {
    pub fn new(req_id: Vec<u8>, enr_seq: u64) -> Self {
        Self { req_id, enr_seq }
    }
}

impl RLPEncode for PingMessage {
    fn encode(&self, buf: &mut dyn BufMut) {
        Encoder::new(buf)
            .encode_field(&Bytes::from(self.req_id.clone()))
            .encode_field(&self.enr_seq)
            .finish();
    }
}

impl RLPDecode for PingMessage {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let ((req_id, enr_seq), remaining): ((Bytes, u64), &[u8]) =
            RLPDecode::decode_unfinished(rlp)?;
        let ping = PingMessage {
            req_id: req_id.to_vec(),
            enr_seq,
        };
        Ok((ping, remaining))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PongMessage {
    pub req_id: u64,
    pub enr_seq: u64,
    pub recipient_addr: IpAddr,
}

impl RLPEncode for PongMessage {
    fn encode(&self, buf: &mut dyn BufMut) {
        Encoder::new(buf)
            .encode_field(&self.req_id)
            .encode_field(&self.enr_seq)
            .encode_field(&self.recipient_addr)
            .finish();
    }
}

impl RLPDecode for PongMessage {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (req_id, decoder) = decoder.decode_field("req_id")?;
        let (enr_seq, decoder) = decoder.decode_field("enr_seq")?;
        let (recipient_addr, decoder) = decoder.decode_field("recipient_addr")?;

        Ok((
            Self {
                req_id,
                enr_seq,
                recipient_addr,
            },
            decoder.finish()?,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindNodeMessage {
    pub req_id: u64,
    pub distance: Vec<u64>,
}

impl RLPEncode for FindNodeMessage {
    fn encode(&self, buf: &mut dyn BufMut) {
        Encoder::new(buf)
            .encode_field(&self.req_id)
            .encode_field(&self.distance)
            .finish();
    }
}

impl RLPDecode for FindNodeMessage {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (req_id, decoder) = decoder.decode_field("req_id")?;
        let (distance, decoder) = decoder.decode_field("distance")?;

        Ok((Self { req_id, distance }, decoder.finish()?))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodesMessage {
    pub req_id: u64,
    pub total: u64,
    pub nodes: Vec<NodeRecord>,
}

impl RLPEncode for NodesMessage {
    fn encode(&self, buf: &mut dyn BufMut) {
        Encoder::new(buf)
            .encode_field(&self.req_id)
            .encode_field(&self.total)
            .encode_field(&self.nodes)
            .finish();
    }
}

impl RLPDecode for NodesMessage {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (req_id, decoder) = decoder.decode_field("req_id")?;
        let (total, decoder) = decoder.decode_field("total")?;
        let (nodes, decoder) = decoder.decode_field("nodes")?;

        Ok((
            Self {
                req_id,
                total,
                nodes,
            },
            decoder.finish()?,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TalkReqMessage {
    pub req_id: u64,
    pub protocol: Bytes,
    pub request: Bytes,
}

impl RLPEncode for TalkReqMessage {
    fn encode(&self, buf: &mut dyn BufMut) {
        Encoder::new(buf)
            .encode_field(&self.req_id)
            .encode_field(&self.protocol)
            .encode_field(&self.request)
            .finish();
    }
}

impl RLPDecode for TalkReqMessage {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (req_id, decoder) = decoder.decode_field("req_id")?;
        let (protocol, decoder) = decoder.decode_field("protocol")?;
        let (request, decoder) = decoder.decode_field("request")?;

        Ok((
            Self {
                req_id,
                protocol,
                request,
            },
            decoder.finish()?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        discv5::{
            codec::Discv5Codec,
            messages::{Message, Ordinary, Packet, PingMessage, WhoAreYou},
        },
        types::NodeRecordPairs,
        utils::{node_id, public_key_from_signing_key},
    };
    use bytes::BytesMut;
    use ethrex_common::H512;
    use hex_literal::hex;
    use secp256k1::SecretKey;
    use std::net::Ipv4Addr;
    use tokio_util::codec::Decoder as _;

    // node-a-key = 0xeef77acb6c6a6eebc5b363a475ac583ec7eccdb42b6481424c60f59aa326547f
    // node-b-key = 0x66fb62bfbd66b9177a138c1e5cddbe4f7c30c343e94e68df8769459cb1cde628
    // let node_a_key = SecretKey::from_byte_array(&hex!(
    //     "eef77acb6c6a6eebc5b363a475ac583ec7eccdb42b6481424c60f59aa326547f"
    // ))
    // .unwrap();
    // let node_b_key = SecretKey::from_byte_array(&hex!(
    //     "66fb62bfbd66b9177a138c1e5cddbe4f7c30c343e94e68df8769459cb1cde628"
    // ))
    // .unwrap();

    #[test]
    fn test_encode_whoareyou_packet() {
        // # src-node-id = 0xaaaa8419e9f49d0083561b48287df592939a8d19947d8c0ef88f2a4856a69fbb
        // # dest-node-id = 0xbbbb9d047f0488c0b5a93c1c3f2d8bafc7c8ff337024a55434a0d0555de64db9
        // # whoareyou.challenge-data = 0x000000000000000000000000000000006469736376350001010102030405060708090a0b0c00180102030405060708090a0b0c0d0e0f100000000000000000
        // # whoareyou.request-nonce = 0x0102030405060708090a0b0c
        // # whoareyou.id-nonce = 0x0102030405060708090a0b0c0d0e0f10
        // # whoareyou.enr-seq = 0
        //
        // 00000000000000000000000000000000088b3d434277464933a1ccc59f5967ad
        // 1d6035f15e528627dde75cd68292f9e6c27d6b66c8100a873fcbaed4e16b8d
        let node_b_key = SecretKey::from_byte_array(&hex!(
            "66fb62bfbd66b9177a138c1e5cddbe4f7c30c343e94e68df8769459cb1cde628"
        ))
        .unwrap();

        let packet = Packet::WhoAreYou(WhoAreYou {
            id_nonce: u128::from_be_bytes(
                hex!("0102030405060708090a0b0c0d0e0f10")
                    .to_vec()
                    .try_into()
                    .unwrap(),
            ),
            enr_seq: 0,
        });

        let dest_id = node_id(&public_key_from_signing_key(&node_b_key));
        let mut buf = Vec::new();

        let _ = packet.encode(
            &mut buf,
            0,
            hex!("0102030405060708090a0b0c").to_vec(),
            &dest_id,
        );
        let expected = &hex!(
            "00000000000000000000000000000000088b3d434277464933a1ccc59f5967ad1d6035f15e528627dde75cd68292f9e6c27d6b66c8100a873fcbaed4e16b8d"
        );

        assert_eq!(buf, expected);
    }

    #[test]
    fn test_decode_whoareyou_packet() {
        // # src-node-id = 0xaaaa8419e9f49d0083561b48287df592939a8d19947d8c0ef88f2a4856a69fbb
        // # dest-node-id = 0xbbbb9d047f0488c0b5a93c1c3f2d8bafc7c8ff337024a55434a0d0555de64db9
        // # whoareyou.challenge-data = 0x000000000000000000000000000000006469736376350001010102030405060708090a0b0c00180102030405060708090a0b0c0d0e0f100000000000000000
        // # whoareyou.request-nonce = 0x0102030405060708090a0b0c
        // # whoareyou.id-nonce = 0x0102030405060708090a0b0c0d0e0f10
        // # whoareyou.enr-seq = 0
        //
        // 00000000000000000000000000000000088b3d434277464933a1ccc59f5967ad
        // 1d6035f15e528627dde75cd68292f9e6c27d6b66c8100a873fcbaed4e16b8d
        let node_b_key = SecretKey::from_byte_array(&hex!(
            "66fb62bfbd66b9177a138c1e5cddbe4f7c30c343e94e68df8769459cb1cde628"
        ))
        .unwrap();

        let dest_id = node_id(&public_key_from_signing_key(&node_b_key));
        let mut codec = Discv5Codec::new(dest_id);

        let mut encoded = BytesMut::from(hex!(
            "00000000000000000000000000000000088b3d434277464933a1ccc59f5967ad1d6035f15e528627dde75cd68292f9e6c27d6b66c8100a873fcbaed4e16b8d"
        ).as_slice());
        let packet = codec.decode(&mut encoded).unwrap();
        let expected = Some(Packet::WhoAreYou(WhoAreYou {
            id_nonce: u128::from_be_bytes(
                hex!("0102030405060708090a0b0c0d0e0f10")
                    .to_vec()
                    .try_into()
                    .unwrap(),
            ),
            enr_seq: 0,
        }));

        assert_eq!(packet, expected);
    }

    #[test]
    fn test_encode_ping_message() {
        // TODO
    }

    #[test]
    fn test_decode_ping_packet() {
        // # src-node-id = 0xaaaa8419e9f49d0083561b48287df592939a8d19947d8c0ef88f2a4856a69fbb
        // # dest-node-id = 0xbbbb9d047f0488c0b5a93c1c3f2d8bafc7c8ff337024a55434a0d0555de64db9
        // # nonce = 0xffffffffffffffffffffffff
        // # read-key = 0x00000000000000000000000000000000
        // # ping.req-id = 0x00000001
        // # ping.enr-seq = 2
        //
        // 00000000000000000000000000000000088b3d4342774649325f313964a39e55
        // ea96c005ad52be8c7560413a7008f16c9e6d2f43bbea8814a546b7409ce783d3
        // 4c4f53245d08dab84102ed931f66d1492acb308fa1c6715b9d139b81acbdcc
        let node_b_key = SecretKey::from_byte_array(&hex!(
            "66fb62bfbd66b9177a138c1e5cddbe4f7c30c343e94e68df8769459cb1cde628"
        ))
        .unwrap();

        let dest_id = node_id(&public_key_from_signing_key(&node_b_key));

        let encoded = &hex!(
            "00000000000000000000000000000000088b3d4342774649325f313964a39e55ea96c005ad52be8c7560413a7008f16c9e6d2f43bbea8814a546b7409ce783d34c4f53245d08dab84102ed931f66d1492acb308fa1c6715b9d139b81acbdcc"
        );
        // # read-key = 0x00000000000000000000000000000000
        let read_key = [0; 16].to_vec();
        let packet = Packet::decode(&dest_id, &read_key, encoded).unwrap();
        let expected = Packet::Ordinary(Ordinary {
            message: Message::Ping(PingMessage {
                req_id: hex!("00000001").to_vec(),
                enr_seq: 2,
            }),
        });

        assert_eq!(packet, expected);
    }

    #[test]
    fn ping_packet_codec_roundtrip() {
        let pkt = PingMessage {
            req_id: [1, 2, 3, 4].to_vec(),
            enr_seq: 4321,
        };

        let buf = pkt.encode_to_vec();
        assert_eq!(PingMessage::decode(&buf).unwrap(), pkt);
    }

    // TODO: Test encode pong packet (with known good encoding).
    // TODO: Test decode pong packet (from known good encoding).
    #[test]
    fn pong_packet_codec_roundtrip() {
        let pkt = PongMessage {
            req_id: 1234,
            enr_seq: 4321,
            recipient_addr: Ipv4Addr::BROADCAST.into(),
        };

        let buf = pkt.encode_to_vec();
        assert_eq!(PongMessage::decode(&buf).unwrap(), pkt);
    }

    #[test]
    fn findnode_packet_codec_roundtrip() {
        let pkt = FindNodeMessage {
            req_id: 1234,
            distance: vec![1, 2, 3, 4],
        };

        let buf = pkt.encode_to_vec();
        assert_eq!(FindNodeMessage::decode(&buf).unwrap(), pkt);
    }

    #[test]
    fn nodes_packet_codec_roundtrip() {
        let pairs: Vec<(Bytes, Bytes)> = NodeRecordPairs {
            id: Some("id".to_string()),
            ..Default::default()
        }
        .into();

        let pkt = NodesMessage {
            req_id: 1234,
            total: 2,
            nodes: vec![NodeRecord {
                seq: 4321,
                pairs,
                signature: H512::random(),
            }],
        };

        let buf = pkt.encode_to_vec();
        assert_eq!(NodesMessage::decode(&buf).unwrap(), pkt);
    }

    #[test]
    fn talkreq_packet_codec_roundtrip() {
        let pkt = TalkReqMessage {
            req_id: 1234,
            protocol: Bytes::from_static(&[1, 2, 3, 4]),
            request: Bytes::from_static(&[1, 2, 3, 4]),
        };

        let buf = pkt.encode_to_vec();
        assert_eq!(TalkReqMessage::decode(&buf).unwrap(), pkt);
    }
}
