use bytes::{BufMut, Bytes};
use ethrex_common::types::ForkId;
use ethrex_common::{H256, H264, H512};
use ethrex_crypto::keccak::keccak_hash;
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{self, Decoder, Encoder},
};
use secp256k1::{PublicKey, SecretKey, ecdsa::Signature};
use serde::{Deserialize, Serialize, ser::Serializer};
use std::net::Ipv6Addr;
use std::{
    fmt::Display,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    str::FromStr,
    sync::OnceLock,
};
use thiserror::Error;

use crate::utils::node_id;

/// Holds the local node's network addressing configuration, separating the
/// socket bind addresses from the externally-announced addresses, and
/// separating the UDP discovery channel from the TCP RLPx channel.
///
/// This supports two independent axes of configuration:
/// - NAT traversal: bind to `0.0.0.0` but announce a public IP (`--nat.extip`).
/// - Split transports: run UDP discovery on a different address than TCP RLPx
///   (`--discovery.addr`), enabling e.g. IPv4 discv4 with IPv6 RLPx.
#[derive(Debug, Clone)]
pub struct NetworkConfig {
    /// Address to bind the UDP discovery socket to.
    pub discovery_bind_addr: IpAddr,
    /// IP address announced to peers via discv4 Ping/Pong and ENR.
    pub discovery_external_addr: IpAddr,
    /// Addresses to bind TCP RLPx listeners to. One entry per IP family for
    /// dual-stack (e.g. `[0.0.0.0, ::]`); single entry for single-stack.
    pub rlpx_bind_addrs: Vec<IpAddr>,
    /// IP addresses announced to peers for RLPx connections and ENR.
    /// Mirrors `rlpx_bind_addrs` but holds the externally-reachable IPs
    /// (relevant behind NAT or for specific interface binding).
    pub rlpx_external_addrs: Vec<IpAddr>,
    pub tcp_port: u16,
    pub udp_port: u16,
}

impl NetworkConfig {
    /// Returns one socket address per RLPx bind address to listen on.
    pub fn bind_tcp_addrs(&self) -> Vec<SocketAddr> {
        self.rlpx_bind_addrs
            .iter()
            .map(|ip| SocketAddr::new(*ip, self.tcp_port))
            .collect()
    }

    /// Returns the socket address to bind the UDP discovery socket to.
    pub fn bind_udp_addr(&self) -> SocketAddr {
        SocketAddr::new(self.discovery_bind_addr, self.udp_port)
    }

    /// Returns the primary external RLPx address (first entry). Used for
    /// single-stack code paths and enode URL construction.
    pub fn primary_rlpx_external_addr(&self) -> IpAddr {
        self.rlpx_external_addrs
            .first()
            .copied()
            .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
    }

    /// Builds a `NetworkConfig` where all addresses are taken from `node`.
    /// Useful for tests or when no NAT/split-transport mapping is needed.
    pub fn from_node(node: &Node) -> Self {
        Self {
            discovery_bind_addr: node.ip,
            discovery_external_addr: node.ip,
            rlpx_bind_addrs: vec![node.ip],
            rlpx_external_addrs: vec![node.ip],
            tcp_port: node.tcp_port,
            udp_port: node.udp_port,
        }
    }
}

#[derive(Debug, Error)]
pub enum NodeError {
    #[error("Invalid format: {0}")]
    InvalidFormat(String),
    #[error("Parse error: {0}")]
    ParseError(String),
    #[error("RLP decode error: {0}")]
    RLPDecodeError(#[from] RLPDecodeError),
    #[error("Missing field: {0}")]
    MissingField(String),
    #[error("Signature error: {0}")]
    SignatureError(String),
}

const MAX_NODE_RECORD_ENCODED_SIZE: usize = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Endpoint {
    pub ip: IpAddr,
    pub udp_port: u16,
    pub tcp_port: u16,
}

impl RLPEncode for Endpoint {
    fn encode(&self, buf: &mut dyn BufMut) {
        Encoder::new(buf)
            .encode_field(&self.ip)
            .encode_field(&self.udp_port)
            .encode_field(&self.tcp_port)
            .finish();
    }
}

impl RLPDecode for Endpoint {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (ip, decoder) = decoder.decode_field("ip")?;
        let (udp_port, decoder) = decoder.decode_field("udp_port")?;
        let (tcp_port, decoder) = decoder.decode_field("tcp_port")?;
        let remaining = decoder.finish()?;
        let endpoint = Endpoint {
            ip,
            udp_port,
            tcp_port,
        };
        Ok((endpoint, remaining))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub ip: IpAddr,
    pub udp_port: u16,
    pub tcp_port: u16,
    pub public_key: H512,
    pub version: Option<String>,
    node_id: OnceLock<H256>,
}

impl RLPDecode for Node {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (ip, decoder) = decoder.decode_field("ip")?;
        let (udp_port, decoder) = decoder.decode_field("upd_port")?;
        let (tcp_port, decoder) = decoder.decode_field("tcp_port")?;
        let (public_key, decoder) = decoder.decode_field("public_key")?;
        let remaining = decoder.finish_unchecked();

        let node = Node::new(ip, udp_port, tcp_port, public_key);
        Ok((node, remaining))
    }
}

impl<'de> serde::de::Deserialize<'de> for Node {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Node::from_str(&<String>::deserialize(deserializer)?)
            .map_err(|e| serde::de::Error::custom(format!("{}", e)))
    }
}

impl serde::Serialize for Node {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.enode_url())
    }
}

impl FromStr for Node {
    type Err = NodeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            s if s.starts_with("enode://") => Self::from_enode_url(s),
            s if s.starts_with("enr:") => Self::from_enr_url(s),
            _ => Err(NodeError::InvalidFormat(
                "Invalid network address format".into(),
            )),
        }
    }
}

impl Node {
    pub fn new(ip: IpAddr, udp_port: u16, tcp_port: u16, public_key: H512) -> Self {
        Self {
            ip,
            udp_port,
            tcp_port,
            public_key,
            version: None,
            node_id: OnceLock::new(),
        }
    }

    pub fn client_name(&self) -> &str {
        self.version
            .as_deref()
            .and_then(|version| {
                let base = version
                    .split_once('/')
                    .map(|(name, _)| name.trim())
                    .unwrap_or_else(|| version.trim());
                if base.is_empty() { None } else { Some(base) }
            })
            .unwrap_or("unknown")
    }

    pub fn from_enode_url(enode: &str) -> Result<Self, NodeError> {
        let public_key = H512::from_str(&enode[8..136])
            .map_err(|_| NodeError::ParseError("Could not parse public_key".into()))?;

        let address_start = 137;
        let address_part = &enode[address_start..];

        // Remove `?discport=` if present
        let address_part = match address_part.find('?') {
            Some(pos) => &address_part[..pos],
            None => address_part,
        };

        let socket_address: SocketAddr = address_part
            .parse()
            .map_err(|_| NodeError::ParseError("Could not parse socket address".into()))?;
        let ip = socket_address.ip();
        let port = socket_address.port();

        let udp_port = match enode.find("?discport=") {
            Some(pos) => enode[pos + 10..]
                .parse()
                .map_err(|_| NodeError::ParseError("Could not parse discport".into()))?,
            None => port,
        };

        Ok(Self::new(ip, udp_port, port, public_key))
    }

    pub fn from_enr_url(enr: &str) -> Result<Self, NodeError> {
        let base64_decoded = ethrex_common::base64::decode(&enr.as_bytes()[4..]);
        let record = NodeRecord::decode(&base64_decoded).map_err(NodeError::from)?;
        Node::from_enr(&record)
    }

    pub fn from_enr(record: &NodeRecord) -> Result<Self, NodeError> {
        let pairs = record.decode_pairs();
        let public_key = pairs.secp256k1.ok_or(NodeError::MissingField(
            "public key not found in record".into(),
        ))?;
        let verifying_key = PublicKey::from_slice(public_key.as_bytes()).map_err(|_| {
            NodeError::ParseError("public key could not be built from msg pub key bytes".into())
        })?;
        let encoded = verifying_key.serialize_uncompressed();
        let public_key = H512::from_slice(&encoded[1..]);

        let ip: IpAddr = match (pairs.ip, pairs.ip6) {
            (None, None) => {
                return Err(NodeError::MissingField(
                    "Ip not found in record, can't construct node".into(),
                ));
            }
            (None, Some(ipv6)) => IpAddr::from(ipv6),
            (Some(ipv4), None) => IpAddr::from(ipv4),
            (Some(ipv4), Some(_ipv6)) => IpAddr::from(ipv4),
        };

        // both udp and tcp can be defined in the pairs or only one
        // in the latter case, we have to default both ports to the one provided
        let udp_port = pairs
            .udp_port
            .or(pairs.tcp_port)
            .ok_or(NodeError::MissingField("No port found in record".into()))?;
        let tcp_port = pairs
            .tcp_port
            .or(pairs.udp_port)
            .ok_or(NodeError::MissingField("No port found in record".into()))?;

        Ok(Self::new(ip, udp_port, tcp_port, public_key))
    }

    pub fn enode_url(&self) -> String {
        let public_key = hex::encode(self.public_key);
        let node_ip = self.ip;
        let discovery_port = self.udp_port;
        let listener_port = self.tcp_port;
        if discovery_port != listener_port {
            format!("enode://{public_key}@{node_ip}:{listener_port}?discport={discovery_port}")
        } else {
            format!("enode://{public_key}@{node_ip}:{listener_port}")
        }
    }

    pub fn udp_addr(&self) -> SocketAddr {
        // Nodes that use ipv6 currently are only ipv4 masked addresses, so we can convert it to an ipv4 address.
        // If in the future we have real ipv6 nodes, we will need to handle them differently.
        SocketAddr::new(self.ip.to_canonical(), self.udp_port)
    }

    pub fn tcp_addr(&self) -> SocketAddr {
        SocketAddr::new(self.ip, self.tcp_port)
    }

    pub fn node_id(&self) -> H256 {
        *self.node_id.get_or_init(|| node_id(&self.public_key))
    }
}

impl Display for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!(
            "{0} #{1}({2}:{3})",
            self.client_name(),
            self.node_id(),
            self.ip,
            self.tcp_port
        ))
    }
}

/// Reference: [ENR records](https://github.com/ethereum/devp2p/blob/master/enr.md)
#[derive(Debug, PartialEq, Clone, Eq, Default, Serialize, Deserialize)]
pub struct NodeRecord {
    pub signature: H512,
    pub seq: u64,
    // holds optional values in (key, value) format
    // value represents the rlp encoded bytes
    // The key/value pairs must be sorted by key and must be unique
    pub pairs: Vec<(Bytes, Bytes)>,
}

#[derive(Debug, Default, PartialEq)]
pub struct NodeRecordPairs {
    /// The ID of the identity scheme: https://github.com/ethereum/devp2p/blob/master/enr.md#v4-identity-scheme
    /// This is always "v4".
    pub id: Option<String>,
    pub ip: Option<Ipv4Addr>,
    pub ip6: Option<Ipv6Addr>,
    // the record structure reference says that tcp_port and udp_ports are big-endian integers
    // but they are actually encoded as 2 bytes, see geth for example: https://github.com/ethereum/go-ethereum/blob/f544fc3b4659aeca24a6de83f820dd61ea9b39db/p2p/enr/entries.go#L60-L78
    // I think the confusion comes from the fact that geth decodes the bytes and then builds an IPV4/6 big-integer structure.
    pub tcp_port: Option<u16>,
    pub udp_port: Option<u16>,
    /// TCP port for IPv6 RLPx connections (ENR key `tcp6`).
    pub tcp6_port: Option<u16>,
    /// UDP port for IPv6 discovery (ENR key `udp6`).
    pub udp6_port: Option<u16>,
    pub secp256k1: Option<H264>,
    // https://github.com/ethereum/devp2p/blob/master/enr-entries/eth.md
    pub eth: Option<ForkId>,
}

impl NodeRecordPairs {
    /// Returns the best TCP connection address for this peer, preferring IPv4
    /// over IPv6. Returns `None` if neither `ip`+`tcp` nor `ip6`+`tcp6` are
    /// present in the ENR.
    pub fn connection_addr(&self) -> Option<(IpAddr, u16)> {
        if let (Some(ip), Some(port)) = (self.ip, self.tcp_port) {
            return Some((IpAddr::V4(ip), port));
        }
        if let (Some(ip6), Some(port)) = (self.ip6, self.tcp6_port) {
            return Some((IpAddr::V6(ip6), port));
        }
        None
    }
}

impl NodeRecord {
    pub fn decode_pairs(&self) -> NodeRecordPairs {
        let mut decoded_pairs = NodeRecordPairs::default();
        for (key, value) in &self.pairs {
            let Ok(key) = String::from_utf8(key.to_vec()) else {
                continue;
            };
            let value = value.to_vec();
            match key.as_str() {
                "id" => decoded_pairs.id = String::decode(&value).ok(),
                "ip" => decoded_pairs.ip = Ipv4Addr::decode(&value).ok(),
                "ip6" => decoded_pairs.ip6 = Ipv6Addr::decode(&value).ok(),
                "tcp" => decoded_pairs.tcp_port = u16::decode(&value).ok(),
                "tcp6" => decoded_pairs.tcp6_port = u16::decode(&value).ok(),
                "udp" => decoded_pairs.udp_port = u16::decode(&value).ok(),
                "udp6" => decoded_pairs.udp6_port = u16::decode(&value).ok(),
                "secp256k1" => {
                    let Ok(bytes) = Bytes::decode(&value) else {
                        continue;
                    };
                    if bytes.len() != 33 {
                        continue;
                    }
                    decoded_pairs.secp256k1 = Some(H264::from_slice(&bytes))
                }
                "eth" => {
                    // https://github.com/ethereum/devp2p/blob/master/enr-entries/eth.md
                    // entry-value = [[ forkHash, forkNext ], ...]
                    let Ok(decoder) = Decoder::new(&value) else {
                        continue;
                    };
                    // Here we decode fork-id = [ forkHash, forkNext ]
                    // TODO(#3494): here we decode as optional to ignore any errors,
                    // but we should return an error if we can't decode it
                    let (fork_id, decoder) = decoder.decode_optional_field();

                    // As per the spec, we should ignore any additional list elements in entry-value
                    decoder.finish_unchecked();
                    decoded_pairs.eth = fork_id;
                }
                _ => {}
            }
        }

        decoded_pairs
    }

    pub fn enr_url(&self) -> Result<String, NodeError> {
        let rlp_encoded = self.encode_to_vec();
        let base64_encoded = ethrex_common::base64::encode(&rlp_encoded);
        let mut result: String = "enr:".into();
        let base64_encoded = String::from_utf8(base64_encoded)
            .map_err(|_| NodeError::ParseError("Could not base 64 encode enr record".into()))?;
        result.push_str(&base64_encoded);
        Ok(result)
    }

    pub fn from_node(node: &Node, seq: u64, signer: &SecretKey) -> Result<Self, NodeError> {
        let mut record = NodeRecord {
            seq,
            ..Default::default()
        };
        record
            .pairs
            .push(("id".into(), "v4".encode_to_vec().into()));
        record.pairs.push((
            "secp256k1".into(),
            PublicKey::from_secret_key(secp256k1::SECP256K1, signer)
                .serialize()
                .encode_to_vec()
                .into(),
        ));
        match node.ip {
            IpAddr::V4(ipv4) => {
                record
                    .pairs
                    .push(("ip".into(), ipv4.encode_to_vec().into()));
                record
                    .pairs
                    .push(("tcp".into(), node.tcp_port.encode_to_vec().into()));
                record
                    .pairs
                    .push(("udp".into(), node.udp_port.encode_to_vec().into()));
            }
            IpAddr::V6(ipv6) => {
                record
                    .pairs
                    .push(("ip6".into(), ipv6.encode_to_vec().into()));
                record
                    .pairs
                    .push(("tcp6".into(), node.tcp_port.encode_to_vec().into()));
                record
                    .pairs
                    .push(("udp6".into(), node.udp_port.encode_to_vec().into()));
            }
        }

        // ENR pairs must be sorted by key (spec requirement).
        record.pairs.sort_by(|a, b| a.0.cmp(&b.0));

        record.signature = record.sign_record(signer)?;

        Ok(record)
    }

    /// Builds a dual-stack ENR from a [`NetworkConfig`].
    ///
    /// For every address in `network_config.rlpx_external_addrs`:
    /// - An IPv4 address contributes `ip` / `tcp` / `udp` ENR pairs.
    /// - An IPv6 address contributes `ip6` / `tcp6` / `udp6` ENR pairs.
    ///
    /// If both families are present the resulting record advertises both,
    /// allowing peers to reach the node over whichever family they support.
    pub fn from_network_config(
        network_config: &NetworkConfig,
        seq: u64,
        signer: &SecretKey,
    ) -> Result<Self, NodeError> {
        let mut record = NodeRecord {
            seq,
            ..Default::default()
        };
        record
            .pairs
            .push(("id".into(), "v4".encode_to_vec().into()));
        record.pairs.push((
            "secp256k1".into(),
            PublicKey::from_secret_key(secp256k1::SECP256K1, signer)
                .serialize()
                .encode_to_vec()
                .into(),
        ));

        for addr in &network_config.rlpx_external_addrs {
            match addr {
                IpAddr::V4(ipv4) => {
                    record
                        .pairs
                        .push(("ip".into(), ipv4.encode_to_vec().into()));
                    record
                        .pairs
                        .push(("tcp".into(), network_config.tcp_port.encode_to_vec().into()));
                    record
                        .pairs
                        .push(("udp".into(), network_config.udp_port.encode_to_vec().into()));
                }
                IpAddr::V6(ipv6) => {
                    record
                        .pairs
                        .push(("ip6".into(), ipv6.encode_to_vec().into()));
                    record.pairs.push((
                        "tcp6".into(),
                        network_config.tcp_port.encode_to_vec().into(),
                    ));
                    record.pairs.push((
                        "udp6".into(),
                        network_config.udp_port.encode_to_vec().into(),
                    ));
                }
            }
        }

        // ENR pairs must be sorted by key and unique; deduplicate in case
        // both families map to the same family (e.g. two IPv4 addrs).
        record.pairs.sort_by(|a, b| a.0.cmp(&b.0));
        record.pairs.dedup_by(|a, b| a.0 == b.0);

        record.signature = record.sign_record(signer)?;

        Ok(record)
    }

    pub fn set_fork_id(&mut self, fork_id: ForkId, signer: &SecretKey) -> Result<(), NodeError> {
        // Without the Vec wrapper, RLP encoding fork_id directly would produce:
        // [forkHash, forkNext]
        // But the spec requires nested lists:
        // [[forkHash, forkNext]]
        let eth = vec![fork_id];
        self.pairs.push(("eth".into(), eth.encode_to_vec().into()));

        //Pairs need to be sorted by their key.
        //The keys are Bytes which implements Ord, so they can be compared directly. The sorting
        //will be lexicographic (alphabetical for string keys like "eth", "id", "ip", etc.).
        self.pairs.sort_by(|a, b| a.0.cmp(&b.0));

        self.signature = self.sign_record(signer)?;
        Ok(())
    }

    pub fn sign_record(&self, signer: &SecretKey) -> Result<H512, NodeError> {
        let digest = &self.get_signature_digest();
        let msg = secp256k1::Message::from_digest_slice(digest)
            .map_err(|_| NodeError::SignatureError("Invalid message digest".into()))?;
        let (_recovery_id, signature_bytes) = secp256k1::SECP256K1
            .sign_ecdsa_recoverable(&msg, signer)
            .serialize_compact();

        Ok(H512::from_slice(&signature_bytes))
    }

    pub fn get_signature_digest(&self) -> [u8; 32] {
        let mut rlp = vec![];
        structs::Encoder::new(&mut rlp)
            .encode_field(&self.seq)
            .encode_key_value_list::<Bytes>(&self.pairs)
            .finish();
        keccak_hash(&rlp)
    }

    /// Verifies the ENR signature using the embedded public key.
    /// Returns true if the signature is valid, false otherwise.
    pub fn verify_signature(&self) -> bool {
        let pairs = self.decode_pairs();
        let Some(pubkey_bytes) = pairs.secp256k1 else {
            return false;
        };

        let Ok(pubkey) = PublicKey::from_slice(pubkey_bytes.as_bytes()) else {
            return false;
        };

        let digest = self.get_signature_digest();
        let Ok(message) = secp256k1::Message::from_digest_slice(&digest) else {
            return false;
        };

        let Ok(signature) = Signature::from_compact(self.signature.as_bytes()) else {
            return false;
        };

        secp256k1::SECP256K1
            .verify_ecdsa(&message, &signature, &pubkey)
            .is_ok()
    }
}

impl From<NodeRecordPairs> for Vec<(Bytes, Bytes)> {
    fn from(value: NodeRecordPairs) -> Self {
        let mut pairs = vec![];
        if let Some(eth) = value.eth {
            // Without the Vec wrapper, RLP encoding fork_id directly would produce:
            // [forkHash, forkNext]
            // But the spec requires nested lists:
            // [[forkHash, forkNext]]
            let eth = vec![eth];
            pairs.push(("eth".into(), eth.encode_to_vec().into()));
        }
        if let Some(id) = value.id {
            pairs.push(("id".into(), id.encode_to_vec().into()));
        }
        if let Some(ip) = value.ip {
            pairs.push(("ip".into(), ip.encode_to_vec().into()));
        }
        if let Some(ip6) = value.ip6 {
            pairs.push(("ip6".into(), ip6.encode_to_vec().into()));
        }
        if let Some(secp256k1) = value.secp256k1 {
            pairs.push(("secp256k1".into(), secp256k1.encode_to_vec().into()));
        }
        if let Some(tcp) = value.tcp_port {
            pairs.push(("tcp".into(), tcp.encode_to_vec().into()));
        }
        if let Some(tcp6) = value.tcp6_port {
            pairs.push(("tcp6".into(), tcp6.encode_to_vec().into()));
        }
        if let Some(udp) = value.udp_port {
            pairs.push(("udp".into(), udp.encode_to_vec().into()));
        }
        if let Some(udp6) = value.udp6_port {
            pairs.push(("udp6".into(), udp6.encode_to_vec().into()));
        }
        pairs
    }
}

impl RLPDecode for NodeRecord {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        if decoder.get_payload_len() > MAX_NODE_RECORD_ENCODED_SIZE {
            return Err(RLPDecodeError::InvalidLength);
        }
        let (signature, decoder) = decoder.decode_field("signature")?;
        let (seq, decoder) = decoder.decode_field("seq")?;
        let (pairs, decoder) = decode_node_record_optional_fields(vec![], decoder)?;

        // all fields in pairs are optional except for id
        let id_pair = pairs.iter().find(|(k, _v)| k.eq("id".as_bytes()));
        if id_pair.is_some() {
            let node_record = NodeRecord {
                signature,
                seq,
                pairs,
            };
            let remaining = decoder.finish()?;
            Ok((node_record, remaining))
        } else {
            Err(RLPDecodeError::Custom(
                "Invalid node record, 'id' field missing".into(),
            ))
        }
    }
}

/// The NodeRecord optional fields are encoded as key/value pairs, according to the documentation
/// <https://github.com/ethereum/devp2p/blob/master/enr.md#record-structure>
/// This function returns a vector with (key, value) tuples. Both keys and values are stored as Bytes.
/// Each value is the actual RLP encoding of the field including its prefix so it can be decoded as T::decode(value)
fn decode_node_record_optional_fields(
    mut pairs: Vec<(Bytes, Bytes)>,
    decoder: Decoder,
) -> Result<(Vec<(Bytes, Bytes)>, Decoder), RLPDecodeError> {
    let (key, decoder): (Option<Bytes>, Decoder) = decoder.decode_optional_field();
    if let Some(k) = key {
        let (value, decoder): (Vec<u8>, Decoder) = decoder.get_encoded_item()?;
        pairs.push((k, Bytes::from(value)));
        decode_node_record_optional_fields(pairs, decoder)
    } else {
        Ok((pairs, decoder))
    }
}

impl RLPEncode for NodeRecord {
    fn encode(&self, buf: &mut dyn BufMut) {
        structs::Encoder::new(buf)
            .encode_field(&self.signature)
            .encode_field(&self.seq)
            .encode_key_value_list::<Bytes>(&self.pairs)
            .finish();
    }
}

impl RLPEncode for Node {
    fn encode(&self, buf: &mut dyn BufMut) {
        structs::Encoder::new(buf)
            .encode_field(&self.ip)
            .encode_field(&self.udp_port)
            .encode_field(&self.tcp_port)
            .encode_field(&self.public_key)
            .finish();
    }
}
