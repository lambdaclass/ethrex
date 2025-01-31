use bytes::{BufMut, Bytes};
use ethrex_core::{H256, H512};
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};
use keccak_hash::keccak;
use libsecp256k1::{Message, PublicKeyFormat, SecretKey, Signature};
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
    signature: [u8; 64],
    seq: u64,
    id: String,
    pairs: Vec<(Bytes, Bytes)>,
}

impl Default for NodeRecord {
    fn default() -> Self {
        Self {
            signature: [0; 64],
            seq: u64::default(),
            id: String::default(),
            pairs: Vec::default(),
        }
    }
}

impl NodeRecord {
    pub fn new(signature: Signature, seq: u64, id: String, pairs: Vec<(Bytes, Bytes)>) -> Self {
        Self {
            signature: signature.serialize(),
            seq,
            id,
            pairs,
        }
    }

    pub fn from_node(node: &Node, seq: u64, signer: &SecretKey) -> Self {
        let mut record = NodeRecord {
            seq,
            ..Default::default()
        };
        record
            .pairs
            .push(("id".into(), "v4".encode_to_vec().into()));
        record
            .pairs
            .push(("ip".into(), node.endpoint.ip().encode_to_vec().into()));
        record.pairs.push((
            "secp256k1".into(),
            node.id.serialize_compressed().encode_to_vec().into(),
        ));
        record.pairs.push((
            "tcp".into(),
            node.endpoint.tcp_port().encode_to_vec().into(),
        ));
        record.pairs.push((
            "udp".into(),
            node.endpoint.udp_port().encode_to_vec().into(),
        ));

        let mut rlp_encoded_record = Vec::new();
        Encoder::new(&mut rlp_encoded_record)
            .encode_field(&record.seq)
            .encode_key_value_list::<Bytes>(&record.pairs)
            .finish();

        let message = Message::parse(keccak(&rlp_encoded_record).as_fixed_bytes());
        let (signature, _recovery_id) = libsecp256k1::sign(&message, signer);
        record.signature = signature.serialize();

        record
    }

    pub fn signature(&self) -> Signature {
        Signature::parse_standard(&self.signature).unwrap()
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

#[derive(Debug, Clone)]
pub struct PeerData {
    pub id: NodeId,
    pub endpoint: Endpoint,
    pub record: Option<NodeRecord>,
    pub last_ping_hash: Option<H256>,
    pub last_ping: Option<u64>,
    pub state: NodeState,
    // pub find_node_request: Option<FindNodeRequest>,
    // pub enr_request_hash: Option<H256>,
    // pub supported_capabilities: Vec<Capability>,
    // pub revalidated: Option<bool>,
    // pub channels: Option<PeerChannels>,
}

impl PeerData {
    pub fn new_known(id: NodeId, endpoint: Endpoint) -> Self {
        Self {
            id,
            endpoint,
            record: None,
            last_ping_hash: None,
            last_ping: None,
            state: NodeState::Known,
        }
    }
}

#[derive(Debug, Clone)]
pub enum NodeState {
    /// The node is known to us, but we haven't pinged it yet.
    /// These are neighbors of our neighbors that we did not know.
    Known,
    /// The node has been pinged.
    /// We are waiting for a pong message.
    Pinged,
    /// A node turns into proven in either of these cases:
    /// * We received a pong message from the node (as a response to our ping).
    /// * We replied to a ping message from the node.
    Proven { last_pong: u64 },
    /// A node is considered connected if we have established an RLPx connection with it.
    Connected { last_pong: u64 },
}

impl std::fmt::Display for NodeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NodeState::Known => write!(f, "known"),
            NodeState::Pinged => write!(f, "pinged"),
            NodeState::Proven { .. } => write!(f, "proven"),
            NodeState::Connected { .. } => write!(f, "connected"),
        }
    }
}
