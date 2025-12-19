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

use crate::types::NodeRecord;

type Aes128Ctr64BE = ctr::Ctr64BE<aes::Aes128>;

// Max and min packet sizes as defined in
// https://github.com/ethereum/devp2p/blob/master/discv5/discv5-wire.md#udp-communication
// Used for package validation
const MIN_PACKET_SIZE: usize = 63;
const MAX_PACKET_SIZE: usize = 1280;
/// 32 src-id + 1 sig-size + 1 eph-key-size
const HANDSHAKE_AUTHDATA_HEAD: usize = 34;
// protocol data
const PROTOCOL_ID: &[u8] = b"discv5";
const PROTOCOL_VERSION: u16 = 0x0001;
// masking-iv size for a u128
const IV_MASKING_SIZE: usize = 16;
// static_header end limit: 23 bytes from static_header + 16 from iv_masking
const STATIC_HEADER_END: usize = IV_MASKING_SIZE + 23;

#[derive(Debug, thiserror::Error)]
pub enum PacketCodecError {
    #[error("RLP decoding error")]
    RLPDecodeError(#[from] RLPDecodeError),
    #[error("Invalid packet size")]
    InvalidSize,
    #[error("Session not established yet")]
    SessionNotEstablished,
    #[error("Invalid protocol: {0}")]
    InvalidProtocol(String),
    #[error("Stream Cipher Error: {0}")]
    CipherError(String),
    #[error("TryFromSliceError: {0}")]
    TryFromSliceError(#[from] TryFromSliceError),
    #[error("Io Error: {0}")]
    IoError(#[from] std::io::Error),
}

impl From<StreamCipherError> for PacketCodecError {
    fn from(error: StreamCipherError) -> Self {
        PacketCodecError::CipherError(error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Packet {
    Ordinary(Ordinary),
    WhoAreYou(WhoAreYou),
    Handshake(Handshake),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacketHeader {
    pub static_header: Vec<u8>,
    pub flag: u8,
    pub nonce: [u8; 12],
    pub authdata: Vec<u8>,
    /// Offset in the encoded packet where authdata ends, i.e where the header ends.
    pub header_end_offset: usize,
}

impl Packet {
    pub fn decode(
        dest_id: &H256,
        decrypt_key: &[u8; 16],
        encoded_packet: &[u8],
    ) -> Result<Packet, PacketCodecError> {
        if encoded_packet.len() < MIN_PACKET_SIZE || encoded_packet.len() > MAX_PACKET_SIZE {
            return Err(PacketCodecError::InvalidSize);
        }

        // the packet structure is
        // masking-iv || masked-header || message
        // 16 bytes for an u128
        let masking_iv = &encoded_packet[..IV_MASKING_SIZE];

        let mut cipher = <Aes128Ctr64BE as KeyIvInit>::new(dest_id[..16].into(), masking_iv.into());

        let packet_header = Packet::decode_header(&mut cipher, encoded_packet)?;
        let encrypted_message = &encoded_packet[packet_header.header_end_offset..];

        match packet_header.flag {
            0x00 => Ok(Packet::Ordinary(Ordinary::decode(
                masking_iv,
                packet_header.static_header,
                packet_header.authdata,
                packet_header.nonce,
                decrypt_key,
                encrypted_message,
            )?)),
            0x01 => Ok(Packet::WhoAreYou(WhoAreYou::decode(
                &packet_header.authdata,
            )?)),
            0x02 => Ok(Packet::Handshake(Handshake::decode(
                masking_iv,
                packet_header,
                decrypt_key,
                encrypted_message,
            )?)),
            _ => Err(RLPDecodeError::MalformedData)?,
        }
    }

    pub fn encode(
        &self,
        buf: &mut dyn BufMut,
        masking_iv: u128,
        nonce: &[u8; 12],
        dest_id: &H256,
        encrypt_key: &[u8],
    ) -> Result<(), PacketCodecError> {
        let masking_as_bytes = masking_iv.to_be_bytes();
        buf.put_slice(&masking_as_bytes);

        let mut cipher =
            <Aes128Ctr64BE as KeyIvInit>::new(dest_id[..16].into(), masking_as_bytes[..].into());

        match self {
            Packet::Ordinary(ordinary) => {
                let (mut static_header, mut authdata, encrypted_message) =
                    ordinary.encode(&nonce, &masking_as_bytes, encrypt_key)?;

                cipher.try_apply_keystream(&mut static_header)?;
                buf.put_slice(&static_header);
                cipher.try_apply_keystream(&mut authdata)?;
                buf.put_slice(&authdata);
                buf.put_slice(&encrypted_message);
            }
            Packet::WhoAreYou(who_are_you) => {
                who_are_you.encode_header(buf, &mut cipher, nonce)?;
            }
            Packet::Handshake(handshake) => {
                let (mut static_header, mut authdata, encrypted_message) =
                    handshake.encode(&nonce, &masking_as_bytes, encrypt_key)?;

                cipher.try_apply_keystream(&mut static_header)?;
                buf.put_slice(&static_header);
                cipher.try_apply_keystream(&mut authdata)?;
                buf.put_slice(&authdata);
                buf.put_slice(&encrypted_message);
            }
        }
        Ok(())
    }

    fn decode_header<T: StreamCipher>(
        cipher: &mut T,
        encoded_packet: &[u8],
    ) -> Result<PacketHeader, PacketCodecError> {
        // static header
        let mut static_header = encoded_packet[IV_MASKING_SIZE..STATIC_HEADER_END].to_vec();

        cipher.try_apply_keystream(&mut static_header)?;

        // static-header = protocol-id || version || flag || nonce || authdata-size
        //protocol check
        let protocol_id = &static_header[..6];
        let version = u16::from_be_bytes(static_header[6..8].try_into()?);
        if protocol_id != PROTOCOL_ID || version != PROTOCOL_VERSION {
            return Err(PacketCodecError::InvalidProtocol(
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
    pub src_id: H256,
    pub message: Message,
}

impl Ordinary {
    fn encode_authdata(&self, buf: &mut dyn BufMut) -> Result<(), PacketCodecError> {
        buf.put_slice(self.src_id.as_bytes());
        Ok(())
    }

    /// Encodes the ordinary packet returning the header, authdata and encrypted_message
    #[allow(clippy::type_complexity)]
    fn encode(
        &self,
        nonce: &[u8],
        masking_iv: &[u8],
        encrypt_key: &[u8],
    ) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>), PacketCodecError> {
        if encrypt_key.len() < 16 {
            return Err(PacketCodecError::InvalidSize);
        }

        let mut authdata = Vec::new();
        self.encode_authdata(&mut authdata)?;

        let authdata_size: u16 =
            u16::try_from(authdata.len()).map_err(|_| PacketCodecError::InvalidSize)?;

        let mut static_header = Vec::new();
        static_header.put_slice(PROTOCOL_ID);
        static_header.put_slice(&PROTOCOL_VERSION.to_be_bytes());
        static_header.put_u8(0x0);
        static_header.put_slice(nonce);
        static_header.put_slice(&authdata_size.to_be_bytes());

        let mut message = Vec::new();
        self.message.encode(&mut message);

        let mut message_ad = masking_iv.to_vec();
        message_ad.extend_from_slice(&static_header);
        message_ad.extend_from_slice(&authdata);

        let mut cipher = Aes128Gcm::new(encrypt_key[..16].into());
        cipher
            .encrypt_in_place(nonce.into(), &message_ad, &mut message)
            .map_err(|e| PacketCodecError::CipherError(e.to_string()))?;

        Ok((static_header, authdata, message))
    }

    pub fn decode(
        masking_iv: &[u8],
        static_header: Vec<u8>,
        authdata: Vec<u8>,
        nonce: Vec<u8>,
        decrypt_key: &[u8],
        encrypted_message: &[u8],
    ) -> Result<Ordinary, PacketCodecError> {
        if authdata.len() != 32 {
            return Err(PacketCodecError::InvalidSize);
        }
        if decrypt_key.len() < 16 {
            return Err(PacketCodecError::InvalidSize);
        }

        // message    = aesgcm_encrypt(initiator-key, nonce, message-pt, message-ad)
        // message-pt = message-type || message-data
        // message-ad = masking-iv || header
        let mut message_ad = masking_iv.to_vec();
        message_ad.extend_from_slice(&static_header);
        message_ad.extend_from_slice(&authdata);

        let mut message = encrypted_message.to_vec();
        Self::decrypt(decrypt_key, nonce, &mut message, message_ad)?;

        let src_id = H256::from_slice(&authdata);

        let message = Message::decode(&message)?;
        Ok(Ordinary { src_id, message })
    }

    fn decrypt(
        key: &[u8],
        nonce: Vec<u8>,
        message: &mut Vec<u8>,
        message_ad: Vec<u8>,
    ) -> Result<(), PacketCodecError> {
        let mut cipher = Aes128Gcm::new(key[..16].into());
        cipher
            .decrypt_in_place(nonce.as_slice().into(), &message_ad, message)
            .map_err(|e| PacketCodecError::CipherError(e.to_string()))?;
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
        nonce: &[u8],
    ) -> Result<(), PacketCodecError> {
        let mut static_header = Vec::new();
        static_header.put_slice(PROTOCOL_ID);
        static_header.put_slice(&PROTOCOL_VERSION.to_be_bytes());
        static_header.put_u8(0x01);
        static_header.put_slice(nonce);
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

    pub fn decode(authdata: &[u8]) -> Result<WhoAreYou, PacketCodecError> {
        let id_nonce = u128::from_be_bytes(authdata[..16].try_into()?);
        let enr_seq = u64::from_be_bytes(authdata[16..].try_into()?);

        Ok(WhoAreYou { id_nonce, enr_seq })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handshake {
    pub src_id: H256,
    pub id_signature: Vec<u8>,
    pub eph_pubkey: Vec<u8>,
    /// The record field may be omitted if the enr-seq of WHOAREYOU is recent enough, i.e. when it matches the current sequence number of the sending node.
    /// If enr-seq is zero, the record must be sent.
    pub record: Option<NodeRecord>,
    pub message: Message,
}

impl Handshake {
    fn encode_authdata(&self, buf: &mut dyn BufMut) -> Result<(), PacketCodecError> {
        let sig_size: u8 = self
            .id_signature
            .len()
            .try_into()
            .map_err(|_| PacketCodecError::InvalidSize)?;
        let eph_key_size: u8 = self
            .eph_pubkey
            .len()
            .try_into()
            .map_err(|_| PacketCodecError::InvalidSize)?;

        buf.put_slice(self.src_id.as_bytes());
        buf.put_u8(sig_size);
        buf.put_u8(eph_key_size);
        buf.put_slice(&self.id_signature);
        buf.put_slice(&self.eph_pubkey);
        if let Some(record) = &self.record {
            record.encode(buf);
        }

        Ok(())
    }

    /// Encodes the handshake returning the header, authdata and encrypted_message
    #[allow(clippy::type_complexity)]
    fn encode(
        &self,
        nonce: &[u8],
        masking_iv: &[u8],
        encrypt_key: &[u8],
    ) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>), PacketCodecError> {
        let mut authdata = Vec::new();
        self.encode_authdata(&mut authdata)?;

        let authdata_size =
            u16::try_from(authdata.len()).map_err(|_| PacketCodecError::InvalidSize)?;

        let mut static_header = Vec::new();
        static_header.put_slice(PROTOCOL_ID);
        static_header.put_slice(&PROTOCOL_VERSION.to_be_bytes());
        static_header.put_u8(0x02);
        static_header.put_slice(nonce);
        static_header.put_slice(&authdata_size.to_be_bytes());

        let mut message = Vec::new();
        self.message.encode(&mut message);

        if encrypt_key.len() < 16 {
            return Err(PacketCodecError::InvalidSize);
        }

        let mut message_ad = masking_iv.to_vec();
        message_ad.extend_from_slice(&static_header);
        message_ad.extend_from_slice(&authdata);

        let mut cipher = Aes128Gcm::new(encrypt_key[..16].into());
        cipher
            .encrypt_in_place(nonce.into(), &message_ad, &mut message)
            .map_err(|e| PacketCodecError::CipherError(e.to_string()))?;

        Ok((static_header, authdata, message))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn decode(
        masking_iv: &[u8],
        header: PacketHeader,
        decrypt_key: &[u8],
        encrypted_message: &[u8],
    ) -> Result<Handshake, PacketCodecError> {
        if decrypt_key.len() < 16 {
            return Err(PacketCodecError::InvalidSize);
        }
        let PacketHeader {
            static_header,
            nonce,
            authdata,
            ..
        } = header;

        if authdata.len() < HANDSHAKE_AUTHDATA_HEAD {
            return Err(PacketCodecError::InvalidSize);
        }

        let src_id = H256::from_slice(&authdata[..32]);
        let sig_size = authdata[32] as usize;
        let eph_key_size = authdata[33] as usize;

        let authdata_head = HANDSHAKE_AUTHDATA_HEAD + sig_size + eph_key_size;
        if authdata.len() < authdata_head {
            return Err(PacketCodecError::InvalidSize);
        }

        let id_signature =
            authdata[HANDSHAKE_AUTHDATA_HEAD..HANDSHAKE_AUTHDATA_HEAD + sig_size].to_vec();
        let eph_key_start = HANDSHAKE_AUTHDATA_HEAD + sig_size;
        let eph_pubkey = authdata[eph_key_start..authdata_head].to_vec();

        let record = if authdata.len() > authdata_head {
            let record_bytes = &authdata[authdata_head..];
            if record_bytes.is_empty() {
                None
            } else {
                Some(NodeRecord::decode(record_bytes)?)
            }
        } else {
            None
        };

        let mut message_ad = masking_iv.to_vec();
        message_ad.extend_from_slice(&static_header);
        message_ad.extend_from_slice(&authdata);

        let mut message = encrypted_message.to_vec();
        Ordinary::decrypt(decrypt_key, nonce, &mut message, message_ad)?;
        let message = Message::decode(&message)?;

        Ok(Handshake {
            src_id,
            id_signature,
            eph_pubkey,
            record,
            message,
        })
    }
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum Message {
    Ping(PingMessage),
    Pong(PongMessage),
    FindNode(FindNodeMessage),
    Nodes(NodesMessage),
    TalkReq(TalkReqMessage),
    TalkRes(TalkResMessage),
    Ticket(TicketMessage),
    // TODO: add the other messages
}

impl Message {
    fn msg_type(&self) -> u8 {
        match self {
            Message::Ping(_) => 0x01,
            Message::Pong(_) => 0x02,
            Message::FindNode(_) => 0x03,
            Message::Nodes(_) => 0x04,
            Message::TalkReq(_) => 0x05,
            Message::TalkRes(_) => 0x06,
            Message::Ticket(_) => 0x08,
        }
    }

    pub fn encode(&self, buf: &mut dyn BufMut) {
        buf.put_u8(self.msg_type());
        match self {
            Message::Ping(ping) => ping.encode(buf),
            Message::Pong(pong) => pong.encode(buf),
            Message::FindNode(find_node) => find_node.encode(buf),
            Message::Nodes(nodes) => nodes.encode(buf),
            Message::TalkReq(talk_req) => talk_req.encode(buf),
            Message::TalkRes(talk_res) => talk_res.encode(buf),
            Message::Ticket(ticket) => ticket.encode(buf),
        }
    }

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
            0x06 => {
                let enr_response_msg = TalkResMessage::decode(&encrypted_message[1..])?;
                Ok(Message::TalkRes(enr_response_msg))
            }
            0x08 => {
                let ticket_msg = TicketMessage::decode(&encrypted_message[1..])?;
                Ok(Message::Ticket(ticket_msg))
            }
            _ => Err(RLPDecodeError::MalformedData),
        }
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
    pub fn new(req_id: Vec<u8>, enr_seq: u64) -> Self {
        Self { req_id, enr_seq }
    }
}

impl RLPEncode for PingMessage {
    fn encode(&self, buf: &mut dyn BufMut) {
        Encoder::new(buf)
            .encode_field(self.req_id.as_slice())
            .encode_field(&self.enr_seq)
            .finish();
    }
}

impl RLPDecode for PingMessage {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let ((req_id, enr_seq), remaining): ((&[u8], u64), &[u8]) =
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TalkResMessage {
    pub req_id: u64,
    pub response: Vec<u8>,
}

impl RLPEncode for TalkResMessage {
    fn encode(&self, buf: &mut dyn BufMut) {
        Encoder::new(buf)
            .encode_field(&self.req_id)
            .encode_field(&Bytes::copy_from_slice(&self.response))
            .finish();
    }
}

impl RLPDecode for TalkResMessage {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let ((req_id, response), remaining) = <(u64, Bytes) as RLPDecode>::decode_unfinished(rlp)?;

        Ok((
            Self {
                req_id,
                response: response.to_vec(),
            },
            remaining,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketMessage {
    pub req_id: u64,
    pub ticket: Bytes,
    pub wait_time: u64,
}

impl RLPEncode for TicketMessage {
    fn encode(&self, buf: &mut dyn BufMut) {
        Encoder::new(buf)
            .encode_field(&self.req_id)
            .encode_field(&self.ticket)
            .encode_field(&self.wait_time)
            .finish();
    }
}

impl RLPDecode for TicketMessage {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (req_id, decoder) = decoder.decode_field("req_id")?;
        let (ticket, decoder) = decoder.decode_field("ticket")?;
        let (wait_time, decoder) = decoder.decode_field("wait_time")?;

        Ok((
            Self {
                req_id,
                ticket,
                wait_time,
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
    use aes_gcm::{Aes128Gcm, KeyInit, aead::AeadMutInPlace};
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
    fn test_aes_gcm_vector() {
        // https://github.com/ethereum/devp2p/blob/master/discv5/discv5-wire-test-vectors.md#encryptiondecryption
        let key = hex!("9f2d77db7004bf8a1a85107ac686990b");
        let nonce = hex!("27b5af763c446acd2749fe8e");
        let ad = hex!("93a7400fa0d6a694ebc24d5cf570f65d04215b6ac00757875e3f3a5f42107903");
        let mut pt = hex!("01c20101").to_vec();

        let mut cipher = Aes128Gcm::new_from_slice(&key).unwrap();
        cipher
            .encrypt_in_place(nonce.as_slice().into(), &ad, &mut pt)
            .unwrap();

        assert_eq!(
            pt,
            hex!("a5d12a2d94b8ccb3ba55558229867dc13bfa3648").to_vec()
        );
    }

    #[test]
    fn handshake_packet_roundtrip() {
        let node_a_key = SecretKey::from_byte_array(&hex!(
            "eef77acb6c6a6eebc5b363a475ac583ec7eccdb42b6481424c60f59aa326547f"
        ))
        .unwrap();
        let node_b_key = SecretKey::from_byte_array(&hex!(
            "66fb62bfbd66b9177a138c1e5cddbe4f7c30c343e94e68df8769459cb1cde628"
        ))
        .unwrap();

        let src_id = node_id(&public_key_from_signing_key(&node_a_key));
        let dest_id = node_id(&public_key_from_signing_key(&node_b_key));

        let handshake = Handshake {
            src_id,
            id_signature: vec![1; 64],
            eph_pubkey: vec![2; 33],
            record: None,
            message: Message::Ping(PingMessage {
                req_id: vec![3],
                enr_seq: 4,
            }),
        };

        let key = vec![0x10; 16];
        let nonce = hex!("000102030405060708090a0b").to_vec();
        let mut buf = Vec::new();
        let packet = Packet::Handshake(handshake.clone());
        packet.encode(&mut buf, 0, &nonce, &dest_id, &key).unwrap();

        let decoded = Packet::decode(&dest_id, &key, &buf).unwrap();
        assert_eq!(decoded, Packet::Handshake(handshake));
    }

    /// Ping handshake packet (flag 2) from https://github.com/ethereum/devp2p/blob/master/discv5/discv5-wire-test-vectors.md
    #[test]
    fn handshake_packet_vector_test_roundtrip() {
        /*
        # src-node-id = 0xaaaa8419e9f49d0083561b48287df592939a8d19947d8c0ef88f2a4856a69fbb
        # dest-node-id = 0xbbbb9d047f0488c0b5a93c1c3f2d8bafc7c8ff337024a55434a0d0555de64db9
        # nonce = 0xffffffffffffffffffffffff
        # read-key = 0x4f9fac6de7567d1e3b1241dffe90f662
        # ping.req-id = 0x00000001
        # ping.enr-seq = 1
        #
        # handshake inputs:
        #
        # whoareyou.challenge-data = 0x000000000000000000000000000000006469736376350001010102030405060708090a0b0c00180102030405060708090a0b0c0d0e0f100000000000000001
        # whoareyou.request-nonce = 0x0102030405060708090a0b0c
        # whoareyou.id-nonce = 0x0102030405060708090a0b0c0d0e0f10
        # whoareyou.enr-seq = 1
        # ephemeral-key = 0x0288ef00023598499cb6c940146d050d2b1fb914198c327f76aad590bead68b6
        # ephemeral-pubkey = 0x039a003ba6517b473fa0cd74aefe99dadfdb34627f90fec6362df85803908f53a5

        00000000000000000000000000000000088b3d4342774649305f313964a39e55
        ea96c005ad521d8c7560413a7008f16c9e6d2f43bbea8814a546b7409ce783d3
        4c4f53245d08da4bb252012b2cba3f4f374a90a75cff91f142fa9be3e0a5f3ef
        268ccb9065aeecfd67a999e7fdc137e062b2ec4a0eb92947f0d9a74bfbf44dfb
        a776b21301f8b65efd5796706adff216ab862a9186875f9494150c4ae06fa4d1
        f0396c93f215fa4ef524f1eadf5f0f4126b79336671cbcf7a885b1f8bd2a5d83
        9cf8
         */
        let node_b_key = SecretKey::from_byte_array(&hex!(
            "66fb62bfbd66b9177a138c1e5cddbe4f7c30c343e94e68df8769459cb1cde628"
        ))
        .unwrap();
        let dest_id = node_id(&public_key_from_signing_key(&node_b_key));

        let encoded = &hex!(
            "00000000000000000000000000000000088b3d4342774649305f313964a39e55ea96c005ad521d8c7560413a7008f16c9e6d2f43bbea8814a546b7409ce783d34c4f53245d08da4bb252012b2cba3f4f374a90a75cff91f142fa9be3e0a5f3ef268ccb9065aeecfd67a999e7fdc137e062b2ec4a0eb92947f0d9a74bfbf44dfba776b21301f8b65efd5796706adff216ab862a9186875f9494150c4ae06fa4d1f0396c93f215fa4ef524f1eadf5f0f4126b79336671cbcf7a885b1f8bd2a5d839cf8"
        );
        let read_key = hex!("4f9fac6de7567d1e3b1241dffe90f662").to_vec();

        let packet = Packet::decode(&dest_id, &read_key, encoded).unwrap();
        let handshake = match packet {
            Packet::Handshake(hs) => hs,
            other => panic!("unexpected packet {other:?}"),
        };

        assert_eq!(
            handshake.src_id,
            H256::from_slice(&hex!(
                "aaaa8419e9f49d0083561b48287df592939a8d19947d8c0ef88f2a4856a69fbb"
            ))
        );
        assert_eq!(handshake.record, None);
        assert_eq!(
            handshake.eph_pubkey,
            hex!("039a003ba6517b473fa0cd74aefe99dadfdb34627f90fec6362df85803908f53a5").to_vec()
        );
        assert_eq!(
            handshake.message,
            Message::Ping(PingMessage {
                req_id: hex!("00000001").to_vec(),
                enr_seq: 1,
            })
        );

        let masking_iv = u128::from_be_bytes(encoded[..16].try_into().unwrap());
        let nonce = hex!("ffffffffffffffffffffffff").to_vec();
        let mut buf = Vec::new();
        Packet::Handshake(handshake)
            .encode(&mut buf, masking_iv, &nonce, &dest_id, &read_key)
            .unwrap();

        assert_eq!(buf, encoded.to_vec());
    }

    /// Ping handshake message packet (flag 2, with ENR) from https://github.com/ethereum/devp2p/blob/master/discv5/discv5-wire-test-vectors.md
    #[test]
    fn handshake_packet_with_enr_vector_test_roundtrip() {
        let node_b_key = SecretKey::from_byte_array(&hex!(
            "66fb62bfbd66b9177a138c1e5cddbe4f7c30c343e94e68df8769459cb1cde628"
        ))
        .unwrap();
        let dest_id = node_id(&public_key_from_signing_key(&node_b_key));

        let encoded = &hex!(
            "00000000000000000000000000000000088b3d4342774649305f313964a39e55ea96c005ad539c8c7560413a7008f16c9e6d2f43bbea8814a546b7409ce783d34c4f53245d08da4bb23698868350aaad22e3ab8dd034f548a1c43cd246be98562fafa0a1fa86d8e7a3b95ae78cc2b988ded6a5b59eb83ad58097252188b902b21481e30e5e285f19735796706adff216ab862a9186875f9494150c4ae06fa4d1f0396c93f215fa4ef524e0ed04c3c21e39b1868e1ca8105e585ec17315e755e6cfc4dd6cb7fd8e1a1f55e49b4b5eb024221482105346f3c82b15fdaae36a3bb12a494683b4a3c7f2ae41306252fed84785e2bbff3b022812d0882f06978df84a80d443972213342d04b9048fc3b1d5fcb1df0f822152eced6da4d3f6df27e70e4539717307a0208cd208d65093ccab5aa596a34d7511401987662d8cf62b139471"
        );
        let nonce = hex!("ffffffffffffffffffffffff").to_vec();
        let read_key = hex!("53b1c075f41876423154e157470c2f48").to_vec();

        let packet = Packet::decode(&dest_id, &read_key, encoded).unwrap();
        let handshake = match packet {
            Packet::Handshake(hs) => hs,
            other => panic!("unexpected packet {other:?}"),
        };

        assert_eq!(
            handshake.src_id,
            H256::from_slice(&hex!(
                "aaaa8419e9f49d0083561b48287df592939a8d19947d8c0ef88f2a4856a69fbb"
            ))
        );
        assert_eq!(
            handshake.eph_pubkey,
            hex!("039a003ba6517b473fa0cd74aefe99dadfdb34627f90fec6362df85803908f53a5").to_vec()
        );
        assert_eq!(
            handshake.message,
            Message::Ping(PingMessage {
                req_id: hex!("00000001").to_vec(),
                enr_seq: 1,
            })
        );

        let record = handshake.record.clone().expect("expected ENR record");
        let pairs = record.decode_pairs();
        assert_eq!(pairs.id.as_deref(), Some("v4"));
        assert!(pairs.secp256k1.is_some());

        let masking_iv = u128::from_be_bytes(encoded[..16].try_into().unwrap());
        let mut buf = Vec::new();
        Packet::Handshake(handshake)
            .encode(&mut buf, masking_iv, &nonce, &dest_id, &read_key)
            .unwrap();

        assert_eq!(buf, encoded.to_vec());
    }

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
            &hex!("0102030405060708090a0b0c"),
            &dest_id,
            &[],
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

        let node_a_key = SecretKey::from_byte_array(&hex!(
            "eef77acb6c6a6eebc5b363a475ac583ec7eccdb42b6481424c60f59aa326547f"
        ))
        .unwrap();
        let node_b_key = SecretKey::from_byte_array(&hex!(
            "66fb62bfbd66b9177a138c1e5cddbe4f7c30c343e94e68df8769459cb1cde628"
        ))
        .unwrap();

        let src_id = node_id(&public_key_from_signing_key(&node_a_key));
        let dest_id = node_id(&public_key_from_signing_key(&node_b_key));

        let encoded = &hex!(
            "00000000000000000000000000000000088b3d4342774649325f313964a39e55ea96c005ad52be8c7560413a7008f16c9e6d2f43bbea8814a546b7409ce783d34c4f53245d08dab84102ed931f66d1492acb308fa1c6715b9d139b81acbdcc"
        );
        // # read-key = 0x00000000000000000000000000000000
        let read_key = [0; 16].to_vec();
        let packet = Packet::decode(&dest_id, &read_key, encoded).unwrap();
        let expected = Packet::Ordinary(Ordinary {
            src_id,
            message: Message::Ping(PingMessage {
                req_id: hex!("00000001").to_vec(),
                enr_seq: 2,
            }),
        });

        assert_eq!(packet, expected);
    }

    /// Ping message packet (flag 0) from https://github.com/ethereum/devp2p/blob/master/discv5/discv5-wire-test-vectors.md
    #[test]
    fn ordinary_ping_packet_vector_test_roundtrip() {
        let node_b_key = SecretKey::from_byte_array(&hex!(
            "66fb62bfbd66b9177a138c1e5cddbe4f7c30c343e94e68df8769459cb1cde628"
        ))
        .unwrap();
        let dest_id = node_id(&public_key_from_signing_key(&node_b_key));

        let encoded = &hex!(
            "00000000000000000000000000000000088b3d4342774649325f313964a39e55ea96c005ad52be8c7560413a7008f16c9e6d2f43bbea8814a546b7409ce783d34c4f53245d08dab84102ed931f66d1492acb308fa1c6715b9d139b81acbdcc"
        );
        let nonce = hex!("ffffffffffffffffffffffff").to_vec();
        let read_key = [0; 16].to_vec();

        let packet = Packet::decode(&dest_id, &read_key, encoded).unwrap();
        let expected = Packet::Ordinary(Ordinary {
            src_id: H256::from_slice(&hex!(
                "aaaa8419e9f49d0083561b48287df592939a8d19947d8c0ef88f2a4856a69fbb"
            )),
            message: Message::Ping(PingMessage {
                req_id: hex!("00000001").to_vec(),
                enr_seq: 2,
            }),
        });
        assert_eq!(packet, expected);

        let masking_iv = u128::from_be_bytes(encoded[..16].try_into().unwrap());
        let mut buf = Vec::new();
        packet
            .encode(&mut buf, masking_iv, &nonce, &dest_id, &read_key)
            .unwrap();
        assert_eq!(buf, encoded.to_vec());
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

    #[test]
    fn talk_res_packet_codec_roundtrip() {
        let pkt = TalkResMessage {
            req_id: 1234,
            response: b"\x00\x01\x02\x03".into(),
        };

        let buf = pkt.encode_to_vec();
        assert_eq!(TalkResMessage::decode(&buf).unwrap(), pkt);
    }

    #[test]
    fn ticket_packet_codec_roundtrip() {
        let pkt = TicketMessage {
            req_id: 1234,
            ticket: Bytes::from_static(&[1, 2, 3, 4]),
            wait_time: 5,
        };

        let buf = pkt.encode_to_vec();
        assert_eq!(TicketMessage::decode(&buf).unwrap(), pkt);
    }
}
