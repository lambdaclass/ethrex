use bytes::{BufMut, Bytes};
use ethrex_core::H512;
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};
use libsecp256k1::PublicKeyFormat;
use std::{
    fmt::Display,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
};

pub type NodeId = libsecp256k1::PublicKey;
pub type NodeIdHash = H512;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    V4 {
        ip: Ipv4Addr,
        udp_port: u16,
        tcp_port: u16,
    },
    V6 {
        ip: Ipv6Addr,
        udp_port: u16,
        tcp_port: u16,
    },
}

impl Endpoint {
    pub fn new(ip: IpAddr, udp_port: u16, tcp_port: u16) -> Self {
        match ip {
            IpAddr::V4(ip) => Endpoint::V4 {
                ip,
                udp_port,
                tcp_port,
            },
            IpAddr::V6(ip) => Endpoint::V6 {
                ip,
                udp_port,
                tcp_port,
            },
        }
    }

    pub fn ip(&self) -> IpAddr {
        match self {
            Endpoint::V4 { ip, .. } => IpAddr::V4(*ip),
            Endpoint::V6 { ip, .. } => IpAddr::V6(*ip),
        }
    }

    pub fn udp_port(&self) -> u16 {
        match self {
            Endpoint::V4 { udp_port, .. } => *udp_port,
            Endpoint::V6 { udp_port, .. } => *udp_port,
        }
    }

    pub fn tcp_port(&self) -> u16 {
        match self {
            Endpoint::V4 { tcp_port, .. } => *tcp_port,
            Endpoint::V6 { tcp_port, .. } => *tcp_port,
        }
    }

    pub fn udp_socket_addr(self) -> SocketAddr {
        match self {
            Endpoint::V4 {
                ip,
                udp_port,
                tcp_port: _,
            } => SocketAddr::new(IpAddr::V4(ip), udp_port),
            Endpoint::V6 {
                ip,
                udp_port,
                tcp_port: _,
            } => SocketAddr::new(IpAddr::V6(ip), udp_port),
        }
    }

    pub fn tcp_socket_addr(self) -> SocketAddr {
        match self {
            Endpoint::V4 {
                ip,
                udp_port: _,
                tcp_port,
            } => SocketAddr::new(IpAddr::V4(ip), tcp_port),
            Endpoint::V6 {
                ip,
                udp_port: _,
                tcp_port,
            } => SocketAddr::new(IpAddr::V6(ip), tcp_port),
        }
    }
}

impl Display for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Endpoint::V4 {
                ip,
                udp_port,
                tcp_port,
            } => {
                write!(f, "{}:{}:{}", ip, udp_port, tcp_port)
            }
            Endpoint::V6 {
                ip,
                udp_port,
                tcp_port,
            } => {
                write!(f, "[{}]:{}:{}", ip, udp_port, tcp_port)
            }
        }
    }
}

impl RLPEncode for Endpoint {
    fn encode(&self, buf: &mut dyn BufMut) {
        match self {
            Endpoint::V4 {
                ip,
                udp_port,
                tcp_port,
            } => Encoder::new(buf)
                .encode_field(ip)
                .encode_field(udp_port)
                .encode_field(tcp_port)
                .finish(),
            Endpoint::V6 {
                ip,
                udp_port,
                tcp_port,
            } => Encoder::new(buf)
                .encode_field(ip)
                .encode_field(udp_port)
                .encode_field(tcp_port)
                .finish(),
        }
    }
}

impl RLPDecode for Endpoint {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (ip, decoder) =
            if let Ok((ip, decoder)) = Decoder::new(rlp)?.decode_field::<Ipv4Addr>("ip") {
                (IpAddr::V4(ip), decoder)
            } else if let Ok((ip, decoder)) = Decoder::new(rlp)?.decode_field::<Ipv6Addr>("ip") {
                (IpAddr::V6(ip), decoder)
            } else {
                return Err(RLPDecodeError::MalformedData);
            };
        let (udp_port, decoder) = decoder.decode_field("udp_port")?;
        let (tcp_port, decoder) = decoder.decode_field("tcp_port")?;
        let remaining = decoder.finish()?;
        let endpoint = Endpoint::new(ip, udp_port, tcp_port);
        Ok((endpoint, remaining))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub endpoint: Endpoint,
    pub id: NodeId,
}

impl Node {
    pub fn new(ip: IpAddr, udp_port: u16, tcp_port: u16, id: NodeId) -> Self {
        Self {
            endpoint: Endpoint::new(ip, udp_port, tcp_port),
            id,
        }
    }
}

impl RLPEncode for Node {
    fn encode(&self, buf: &mut dyn BufMut) {
        Encoder::new(buf)
            .encode_field(&self.endpoint)
            .encode_field(&self.id.serialize())
            .finish();
    }
}

impl RLPDecode for Node {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (ip, decoder) = decoder.decode_field("ip")?;
        let (udp_port, decoder) = decoder.decode_field("udp_port")?;
        let (tcp_port, decoder) = decoder.decode_field("tcp_port")?;
        let (id, decoder) = decoder.decode_field::<[u8; 65]>("id")?;
        let remaining = decoder.finish()?;
        let node = Node {
            endpoint: Endpoint::new(ip, udp_port, tcp_port),
            id: NodeId::parse_slice(&id, Some(PublicKeyFormat::Raw))
                .map_err(|_error| RLPDecodeError::MalformedData)?,
        };
        Ok((node, remaining))
    }
}

pub const MAX_NODE_RECORD_ENCODED_SIZE: usize = 300;

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct NodeRecord {
    // Compressed secp256k1 public key
    signature: [u8; 33],
    seq: u64,
    id: String,
    pairs: Vec<(Bytes, Bytes)>,
}

impl NodeRecord {
    pub fn new(signature: [u8; 33], seq: u64, id: String, pairs: Vec<(Bytes, Bytes)>) -> Self {
        Self {
            signature,
            seq,
            id,
            pairs,
        }
    }
}

impl RLPEncode for NodeRecord {
    fn encode(&self, buf: &mut dyn BufMut) {
        Encoder::new(buf)
            .encode_field(&self.signature)
            .encode_field(&self.seq)
            .encode_key_value_list::<Bytes>(&self.pairs)
            .finish();
    }
}

impl RLPDecode for NodeRecord {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        if rlp.len() > MAX_NODE_RECORD_ENCODED_SIZE {
            dbg!(format!(
                "Node record size exceeds maximum allowed size: {}",
                rlp.len()
            ));
            return Err(RLPDecodeError::InvalidLength);
        }
        let decoder = Decoder::new(rlp)?;
        let (signature, decoder) = decoder.decode_field("signature")?;
        let (seq, decoder) = decoder.decode_field("seq")?;
        let (pairs, decoder) = decode_node_record_optional_fields(vec![], decoder)?;

        let id_pair =
            pairs
                .iter()
                .find(|(k, _v)| k.eq("id".as_bytes()))
                .ok_or(RLPDecodeError::Custom(
                    "Invalid node record, 'id' field missing".into(),
                ))?;
        let (_key, id) = id_pair;
        let node_record = NodeRecord {
            signature,
            seq,
            id: String::decode(id)?,
            pairs,
        };
        let remaining = decoder.finish()?;
        Ok((node_record, remaining))
    }
}

fn decode_node_record_optional_fields(
    mut pairs: Vec<(Bytes, Bytes)>,
    decoder: Decoder,
) -> Result<(Vec<(Bytes, Bytes)>, Decoder), RLPDecodeError> {
    let (key, decoder): (Option<Bytes>, Decoder) = decoder.decode_optional_field();
    if let Some(key) = key {
        let (value, decoder): (Vec<u8>, Decoder) = decoder.get_encoded_item()?;
        pairs.push((key, Bytes::from(value)));
        decode_node_record_optional_fields(pairs, decoder)
    } else {
        Ok((pairs, decoder))
    }
}

pub struct NodeData {
    pub endpoint: Endpoint,
    pub id: NodeId,
    pub state: NodeState,
}

pub enum NodeState {
    Proven,
}
