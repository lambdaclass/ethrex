use crate::discovery::utils::is_expired;
use crate::types::{Endpoint, Node, NodeId, NodeRecord};
use ethrex_core::H256;
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};
use keccak_hash::keccak;
use libsecp256k1::{
    Message as Secp256k1Message, PublicKeyFormat, RecoveryId as Secp256k1RecoveryId,
    SecretKey as Secp256k1SecretKey, Signature as Secp256k1Signature,
};

pub const MAX_UDP_PAYLOAD_SIZE: usize = 1280;
pub const DEFAULT_UDP_PAYLOAD_BUF: [u8; MAX_UDP_PAYLOAD_SIZE] = [0u8; MAX_UDP_PAYLOAD_SIZE];
pub const HASH_LENGTH_IN_BYTES: usize = 32;
pub const HEADER_LENGTH_IN_BYTES: usize = HASH_LENGTH_IN_BYTES + 65;
pub const PACKET_TYPE_LENGTH_IN_BYTES: usize = 1;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Packet signature verification failed")]
    FailedToVerifySignature,
    #[error("Invalid recovery id")]
    InvalidRecoveryId,
    #[error("Invalid signature")]
    InvalidSignature,
    #[error("Failed to recover public key from signature: {0}")]
    FailedToRecoverPublicKey(libsecp256k1::Error),
    #[error("Invalid packet size: {0}")]
    InvalidPacketSize(usize),
    #[error("Invalid packet type: {0}. Must be in range 0x01..=0x06")]
    InvalidPacketType(u8),
    #[error("Incoming packet hash does not match the computed one: {0:#x} != {1:#x}")]
    PacketHashMismatch(H256, H256),
    #[error("RLP decode error: {0}")]
    FailedToRLPDecode(#[from] RLPDecodeError),
    #[error("Packet expired")]
    PacketExpired,
}

#[derive(Debug, Clone)]
pub struct Packet {
    pub data: PacketData,
    pub node_id: NodeId,
}

impl std::fmt::Display for Packet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Packet {{ data: {:?}, node_id: {} }}",
            self.data,
            hex::encode(&self.node_id.serialize()[1..])
        )
    }
}

impl Packet {
    pub fn new(data: PacketData, node_id: NodeId) -> Self {
        Self { data, node_id }
    }

    // packet = packet-header || packet-data
    // packet-header = hash || signature || packet-type
    // hash = keccak256(signature || packet-type || packet-data)
    // signature = sign(packet-type || packet-data)
    //
    // packet = hash || signature || packet-type || packet-data
    #[allow(clippy::result_large_err)]
    pub fn decode(encoded_packet: &[u8]) -> Result<Packet, Error> {
        if encoded_packet.len() < HEADER_LENGTH_IN_BYTES + 1 {
            return Err(Error::InvalidPacketSize(encoded_packet.len()));
        };

        let packet_type = encoded_packet
            .get(HEADER_LENGTH_IN_BYTES)
            .ok_or(Error::InvalidPacketSize(encoded_packet.len()))?;
        Self::validate_packet_type(*packet_type)?;

        let encoded_message = encoded_packet
            .get(HEADER_LENGTH_IN_BYTES..)
            .ok_or(Error::InvalidPacketSize(encoded_packet.len()))?;
        let packet_data = PacketData::decode(
            encoded_packet
                .get(HEADER_LENGTH_IN_BYTES + PACKET_TYPE_LENGTH_IN_BYTES..)
                .ok_or(Error::InvalidPacketSize(encoded_packet.len()))?,
        )?;

        let hash = H256::from_slice(
            encoded_packet
                .get(..HASH_LENGTH_IN_BYTES)
                .ok_or(Error::InvalidPacketSize(encoded_packet.len()))?,
        );
        let header_hash = keccak(
            encoded_packet
                .get(HASH_LENGTH_IN_BYTES..)
                .ok_or(Error::InvalidPacketSize(encoded_packet.len()))?,
        );
        if hash != header_hash {
            return Err(Error::PacketHashMismatch(header_hash, hash));
        }

        let signature_bytes = encoded_packet
            .get(HASH_LENGTH_IN_BYTES..HEADER_LENGTH_IN_BYTES)
            .ok_or(Error::InvalidPacketSize(encoded_packet.len()))?;

        let recovery_id = Secp256k1RecoveryId::parse(
            *signature_bytes
                .get(64)
                .ok_or(Error::InvalidPacketSize(encoded_packet.len()))?,
        )
        .map_err(|_err| Error::InvalidRecoveryId)?;

        let signature = Secp256k1Signature::parse_standard_slice(
            signature_bytes
                .get(0..64)
                .ok_or(Error::InvalidPacketSize(encoded_packet.len()))?,
        )
        .map_err(|_err| Error::InvalidSignature)?;

        let message = Secp256k1Message::parse(keccak(encoded_message).as_fixed_bytes());

        let peer_pk = libsecp256k1::recover(&message, &signature, &recovery_id)
            .map_err(Error::FailedToRecoverPublicKey)?;

        if !libsecp256k1::verify(&message, &signature, &peer_pk) {
            return Err(Error::FailedToVerifySignature);
        }

        let packet = Self {
            data: packet_data,
            node_id: peer_pk,
        };

        if is_expired(&packet) {
            return Err(Error::PacketExpired);
        }

        Ok(packet)
    }

    #[allow(clippy::result_large_err)]
    fn validate_packet_type(packet_type: u8) -> Result<(), Error> {
        if !(0x01..=0x06).contains(&packet_type) {
            return Err(Error::InvalidPacketType(packet_type));
        }
        Ok(())
    }

    // packet = packet-header || packet-data
    pub fn encode(&self, node_signer: &Secp256k1SecretKey) -> Vec<u8> {
        let packet_header = self.encode_header(node_signer);
        let packet_data = self.encode_data();
        [packet_header, packet_data].concat()
    }

    // packet-header = hash || signature || packet-type
    fn encode_header(&self, node_signer: &Secp256k1SecretKey) -> Vec<u8> {
        let signature = self.signature(node_signer);
        let hash = self.hash(node_signer);
        let packet_type = self.r#type();
        [hash.as_bytes(), &signature, &[packet_type]].concat()
    }

    // signature = sign(packet-type || packet-data)
    fn signature(&self, node_signer: &Secp256k1SecretKey) -> Vec<u8> {
        let (signature, recovery_id) = libsecp256k1::sign(&self.message(), node_signer);
        [signature.serialize().as_ref(), &[recovery_id.serialize()]].concat()
    }

    // packet-type || packet-data
    fn message(&self) -> Secp256k1Message {
        let message_bytes = [&[self.r#type()][..], &self.encode_data()].concat();
        Secp256k1Message::parse(keccak(message_bytes).as_fixed_bytes())
    }

    // hash = keccak256(signature || packet-type || packet-data)
    pub fn hash(&self, node_signer: &Secp256k1SecretKey) -> H256 {
        let signature = self.signature(node_signer);
        let packet_type = self.r#type();
        let packet_data = self.encode_data();
        keccak([&signature, &[packet_type][..], packet_data.as_slice()].concat())
    }

    fn r#type(&self) -> u8 {
        self.data.r#type()
    }

    fn encode_data(&self) -> Vec<u8> {
        self.data.encode()
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum PacketData {
    Ping {
        version: u8,
        from: Endpoint,
        to: Endpoint,
        expiration: u64,
        enr_seq: Option<u64>,
    },
    Pong {
        to: Endpoint,
        ping_hash: H256,
        expiration: u64,
        enr_seq: Option<u64>,
    },
    FindNode {
        target: NodeId,
        expiration: u64,
    },
    Neighbors {
        nodes: Vec<Node>,
        expiration: u64,
    },
    ENRRequest {
        expiration: u64,
    },
    ENRResponse {
        request_hash: H256,
        node_record: NodeRecord,
    },
}

impl PacketData {
    pub fn encode(&self) -> Vec<u8> {
        self.encode_to_vec()
    }

    #[allow(clippy::result_large_err)]
    pub fn decode(rlp: &[u8]) -> Result<Self, Error> {
        <PacketData as RLPDecode>::decode(rlp).map_err(Error::from)
    }

    pub fn r#type(&self) -> u8 {
        match self {
            PacketData::Ping { .. } => 0x01,
            PacketData::Pong { .. } => 0x02,
            PacketData::FindNode { .. } => 0x03,
            PacketData::Neighbors { .. } => 0x04,
            PacketData::ENRRequest { .. } => 0x05,
            PacketData::ENRResponse { .. } => 0x06,
        }
    }
}

impl RLPEncode for PacketData {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        match self {
            PacketData::Ping {
                version,
                from,
                to,
                expiration,
                enr_seq,
            } => {
                Encoder::new(buf)
                    .encode_field(version)
                    .encode_field(from)
                    .encode_field(to)
                    .encode_field(expiration)
                    .encode_optional_field(enr_seq)
                    .finish();
            }
            PacketData::Pong {
                to,
                ping_hash,
                expiration,
                enr_seq,
            } => {
                Encoder::new(buf)
                    .encode_field(to)
                    .encode_field(ping_hash)
                    .encode_field(expiration)
                    .encode_optional_field(enr_seq)
                    .finish();
            }
            PacketData::FindNode { target, expiration } => {
                Encoder::new(buf)
                    .encode_field(&target.serialize())
                    .encode_field(expiration)
                    .finish();
            }
            PacketData::Neighbors { nodes, expiration } => {
                Encoder::new(buf)
                    .encode_field(nodes)
                    .encode_field(expiration)
                    .finish();
            }
            PacketData::ENRRequest { expiration } => {
                Encoder::new(buf).encode_field(expiration).finish();
            }
            PacketData::ENRResponse {
                request_hash,
                node_record,
            } => {
                Encoder::new(buf)
                    .encode_field(request_hash)
                    .encode_field(node_record)
                    .finish();
            }
        }
    }
}

impl RLPDecode for PacketData {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        try_rlp_decode_ping(rlp)
            .or(try_rlp_decode_pong(rlp))
            .or(try_rlp_decode_find_node(rlp))
            .or(try_rlp_decode_neighbors(rlp))
            .or(try_rlp_decode_enr_request(rlp))
            .or(try_rlp_decode_enr_response(rlp))
    }
}

fn try_rlp_decode_ping(rlp: &[u8]) -> Result<(PacketData, &[u8]), RLPDecodeError> {
    let decoder = Decoder::new(rlp)?;
    let (version, decoder) = decoder.decode_field("version")?;
    let (from, decoder) = decoder.decode_field("from")?;
    let (to, decoder) = decoder.decode_field("to")?;
    let (expiration, decoder) = decoder.decode_field("expiration")?;
    let (enr_seq, decoder) = decoder.decode_optional_field();
    Ok((
        PacketData::Ping {
            version,
            from,
            to,
            expiration,
            enr_seq,
        },
        decoder.finish_unchecked(),
    ))
}

fn try_rlp_decode_pong(rlp: &[u8]) -> Result<(PacketData, &[u8]), RLPDecodeError> {
    let decoder = Decoder::new(rlp)?;
    let (to, decoder) = decoder.decode_field("to")?;
    let (ping_hash, decoder) = decoder.decode_field("ping_hash")?;
    let (expiration, decoder) = decoder.decode_field("expiration")?;
    let (enr_seq, decoder) = decoder.decode_optional_field();
    Ok((
        PacketData::Pong {
            to,
            ping_hash,
            expiration,
            enr_seq,
        },
        decoder.finish_unchecked(),
    ))
}

fn try_rlp_decode_find_node(rlp: &[u8]) -> Result<(PacketData, &[u8]), RLPDecodeError> {
    let decoder = Decoder::new(rlp)?;
    let (target, decoder) = decoder.decode_field::<[u8; 64]>("target")?;
    let (expiration, decoder) = decoder.decode_field("expiration")?;
    Ok((
        PacketData::FindNode {
            target: NodeId::parse_slice(&target, Some(PublicKeyFormat::Raw))
                .map_err(|_error| RLPDecodeError::MalformedData)?,
            expiration,
        },
        decoder.finish_unchecked(),
    ))
}

fn try_rlp_decode_neighbors(rlp: &[u8]) -> Result<(PacketData, &[u8]), RLPDecodeError> {
    let decoder = Decoder::new(rlp)?;
    let (nodes, decoder) = decoder.decode_field("nodes")?;
    let (expiration, decoder) = decoder.decode_field("expiration")?;
    Ok((
        PacketData::Neighbors { nodes, expiration },
        decoder.finish_unchecked(),
    ))
}

fn try_rlp_decode_enr_request(rlp: &[u8]) -> Result<(PacketData, &[u8]), RLPDecodeError> {
    let decoder = Decoder::new(rlp)?;
    let (expiration, decoder) = decoder.decode_field("expiration")?;
    Ok((
        PacketData::ENRRequest { expiration },
        decoder.finish_unchecked(),
    ))
}

fn try_rlp_decode_enr_response(rlp: &[u8]) -> Result<(PacketData, &[u8]), RLPDecodeError> {
    let decoder = Decoder::new(rlp)?;
    let (request_hash, decoder) = decoder.decode_field("request_hash")?;
    let (node_record, decoder) = decoder.decode_field("node_record")?;
    Ok((
        PacketData::ENRResponse {
            request_hash,
            node_record,
        },
        decoder.finish_unchecked(),
    ))
}
