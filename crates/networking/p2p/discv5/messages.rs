use std::array::TryFromSliceError;

use aes::cipher::{KeyIvInit, StreamCipher, StreamCipherError};
use bytes::BufMut;
use ethrex_common::H256;
use ethrex_rlp::{decode::RLPDecode, error::RLPDecodeError, structs::Decoder};
use secp256k1::SecretKey;

type Aes128Ctr64BE = ctr::Ctr64BE<aes::Aes128>;

// Max and min packet sizes as defined in
// https://github.com/ethereum/devp2p/blob/master/discv5/discv5-wire.md#udp-communication
// Used for package validation
const MIN_PACKET_SIZE: usize = 63;
const MAX_PACKET_SIZE: usize = 1280;
// protocol id for validation
const PROTOCOL_ID: &[u8] = b"discv5";
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
    #[error("Invalid protocol id: {0}")]
    InvalidProtocolId(String),
    #[error("Stream Cipher Error: {0}")]
    ChipherError(String),
    #[error("TryFromSliceError: {0}")]
    TryFromSliceError(#[from] TryFromSliceError),
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

impl Packet {
    pub fn decode(dest_id: &H256, encoded_packet: &[u8]) -> Result<Packet, PacketDecodeErr> {
        if encoded_packet.len() < MIN_PACKET_SIZE || encoded_packet.len() > MAX_PACKET_SIZE {
            return Err(PacketDecodeErr::InvalidSize);
        }

        // the packet structure is
        // masking-iv || masked-header || message
        // 16 bytes for an u128
        let masking_iv = &encoded_packet[..IV_MASKING_SIZE];

        let mut cipher = <Aes128Ctr64BE as KeyIvInit>::new(dest_id[..16].into(), masking_iv.into());

        let (static_header, flag, nonce, authdata, authdata_end) =
            Packet::decode_header(&mut cipher, encoded_packet)?;

        match flag {
            0x00 => Ok(Packet::Ordinary(Ordinary::decode(
                masking_iv,
                static_header,
                authdata,
                nonce,
                &encoded_packet[authdata_end..],
            )?)),
            0x01 => Ok(Packet::WhoAreYou(WhoAreYou::decode(
                masking_iv,
                static_header,
                authdata,
                nonce,
            )?)),
            _ => Err(RLPDecodeError::MalformedData)?,
        }
    }

    pub fn decode_header<T: StreamCipher>(
        cipher: &mut T,
        encoded_packet: &[u8],
    ) -> Result<(Vec<u8>, u8, Vec<u8>, Vec<u8>, usize), PacketDecodeErr> {
        // static header
        let mut static_header = encoded_packet[IV_MASKING_SIZE..STATIC_HEADER_END].to_vec();

        cipher.try_apply_keystream(&mut static_header)?;

        // static-header = protocol-id || version || flag || nonce || authdata-size

        //protocol_id check
        let protocol_id = &static_header[..6];
        if protocol_id != PROTOCOL_ID {
            return Err(PacketDecodeErr::InvalidProtocolId(
                match str::from_utf8(&protocol_id) {
                    Ok(result) => result.to_string(),
                    Err(_) => format!("{:?}", protocol_id),
                },
            ));
        }

        //let version = &static_header[6..8];
        let flag = static_header[8];
        let nonce = static_header[9..21].to_vec();
        let authdata_size = u16::from_be_bytes(static_header[21..23].try_into()?) as usize;
        let authdata_end = STATIC_HEADER_END + authdata_size;
        let authdata = &mut encoded_packet[STATIC_HEADER_END..authdata_end].to_vec();

        cipher.try_apply_keystream(authdata)?;

        Ok((static_header, flag, nonce, authdata.to_vec(), authdata_end))
    }

    pub fn encode(&self, buf: &mut dyn BufMut, signer: &SecretKey) {
        //self.message.encode(buf, signer);
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
        encrypted_message: &[u8],
    ) -> Result<Ordinary, PacketDecodeErr> {
        // message    = aesgcm_encrypt(initiator-key, nonce, message-pt, message-ad)
        // message-pt = message-type || message-data
        // message-ad = masking-iv || header
        let mut message_ad = masking_iv.to_vec();
        message_ad.extend_from_slice(&static_header);
        message_ad.extend_from_slice(&authdata);

        let message = Message::decode_with_type(1, encrypted_message)?;
        Ok(Ordinary { message })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhoAreYou {
    pub id_nonce: Vec<u8>,
    pub enr_seq: u64,
}

impl WhoAreYou {
    pub fn decode(
        masking_iv: &[u8],
        static_header: Vec<u8>,
        authdata: Vec<u8>,
        nonce: Vec<u8>,
    ) -> Result<WhoAreYou, PacketDecodeErr> {
        // message    = aesgcm_encrypt(initiator-key, nonce, message-pt, message-ad)
        // message-pt = message-type || message-data
        // message-ad = masking-iv || header
        let mut message_ad = masking_iv.to_vec();
        message_ad.extend_from_slice(&static_header);
        message_ad.extend_from_slice(&authdata);

        let id_nonce = vec![];
        let enr_seq = 0;

        Ok(WhoAreYou { id_nonce, enr_seq })
    }
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum Message {
    Ping(PingMessage),
    // TODO: add the other messages
}

impl Message {
    pub fn decode_with_type(packet_type: u8, msg: &[u8]) -> Result<Message, RLPDecodeError> {
        match packet_type {
            0x01 => {
                let (ping, _rest) = PingMessage::decode_unfinished(msg)?;
                Ok(Message::Ping(ping))
            }
            // 0x02 => {
            //     let (pong, _rest) = PongMessage::decode_unfinished(msg)?;
            //     Ok(Message::Pong(pong))
            // }
            // 0x03 => {
            //     let (find_node_msg, _rest) = FindNodeMessage::decode_unfinished(msg)?;
            //     Ok(Message::FindNode(find_node_msg))
            // }
            // 0x04 => {
            //     let (neighbors_msg, _rest) = NeighborsMessage::decode_unfinished(msg)?;
            //     Ok(Message::Neighbors(neighbors_msg))
            // }
            // 0x05 => {
            //     let (enr_request_msg, _rest) = ENRRequestMessage::decode_unfinished(msg)?;
            //     Ok(Message::ENRRequest(enr_request_msg))
            // }
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
    pub req_id: u64,
    /// The ENR sequence number of the sender.
    pub enr_seq: u64,
}

impl PingMessage {
    pub fn new(req_id: u64, enr_seq: u64) -> Self {
        Self { req_id, enr_seq }
    }
}

impl RLPDecode for PingMessage {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (req_id, decoder) = decoder.decode_field("req_id")?;
        let (enr_seq, decoder) = decoder.decode_field("enr_seq")?;

        let ping = PingMessage { req_id, enr_seq };
        // NOTE: as per the spec, any additional elements should be ignored.
        let remaining = decoder.finish_unchecked();
        Ok((ping, remaining))
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        discv5::messages::{Message, Ordinary, Packet, PingMessage, WhoAreYou},
        utils::{node_id, public_key_from_signing_key},
    };
    use hex_literal::hex;
    use secp256k1::SecretKey;

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
        let packet = Packet::decode(&dest_id, encoded).unwrap();
        let expected = Packet::Ordinary(Ordinary {
            message: Message::Ping(PingMessage {
                req_id: 0x00000001,
                enr_seq: 2,
            }),
        });

        assert_eq!(packet, expected);
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

        let encoded = &hex!(
            "00000000000000000000000000000000088b3d434277464933a1ccc59f5967ad1d6035f15e528627dde75cd68292f9e6c27d6b66c8100a873fcbaed4e16b8d"
        );
        let packet = Packet::decode(&dest_id, encoded).unwrap();
        let expected = Packet::WhoAreYou(WhoAreYou {
            id_nonce: (&hex!("0102030405060708090a0b0c0d0e0f10")).to_vec(),
            enr_seq: 0,
        });

        assert_eq!(packet, expected);
    }
}
