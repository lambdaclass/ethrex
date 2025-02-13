use bytes::{BufMut, Bytes};
use ethrex_common::{H264, H512};
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{self, Decoder, Encoder},
};
use k256::ecdsa::{SigningKey, VerifyingKey};
use sha3::{Digest, Keccak256};
use std::{
    fmt::Display,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    str::FromStr,
};

const MAX_NODE_RECORD_ENCODED_SIZE: usize = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Endpoint {
    pub ip: IpAddr,
    pub udp_port: u16,
    pub tcp_port: u16,
}

impl Endpoint {
    pub fn tcp_address(&self) -> Option<SocketAddr> {
        (self.tcp_port != 0).then_some(SocketAddr::new(self.ip, self.tcp_port))
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Node {
    pub ip: IpAddr,
    pub udp_port: u16,
    pub tcp_port: u16,
    pub node_id: H512,
}

impl RLPDecode for Node {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (ip, decoder) = decoder.decode_field("ip")?;
        let (udp_port, decoder) = decoder.decode_field("upd_port")?;
        let (tcp_port, decoder) = decoder.decode_field("tcp_port")?;
        let (node_id, decoder) = decoder.decode_field("node_id")?;
        let remaining = decoder.finish_unchecked();

        let node = Node {
            ip,
            udp_port,
            tcp_port,
            node_id,
        };
        Ok((node, remaining))
    }
}

impl<'de> serde::de::Deserialize<'de> for Node {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Node::from_str(&<String>::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl FromStr for Node {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            s if s.starts_with("enode://") => Self::from_enode_url(s),
            s if s.starts_with("enr:") => Self::from_enr_url(s),
            _ => Err("Invalid network address format".into()),
        }
    }
}

impl Node {
    pub fn from_enode_url(enode: &str) -> Result<Self, String> {
        let node_id = H512::from_str(&enode[8..136]).map_err(|_| "Could not parse node_id")?;

        let address_start = 137;
        let address_part = &enode[address_start..];

        // Remove `?discport=` if present
        let address_part = match address_part.find('?') {
            Some(pos) => &address_part[..pos],
            None => address_part,
        };

        let socket_address: SocketAddr = address_part
            .parse()
            .map_err(|_| "Could not parse socket address")?;
        let ip = socket_address.ip();
        let port = socket_address.port();

        let udp_port = match enode.find("?discport=") {
            Some(pos) => enode[pos + 10..]
                .parse()
                .map_err(|_| "Could not parse discport")?,
            None => port,
        };

        Ok(Self {
            node_id,
            ip,
            tcp_port: port,
            udp_port,
        })
    }

    pub fn from_enr_url(enr: &str) -> Result<Self, String> {
        let base64_decoded = ethrex_common::base64::decode(enr[4..].as_bytes());
        let record = NodeRecord::decode(&base64_decoded)
            .map_err(|_| "Could not build node record from enr")?;
        let pairs = record.decode_pairs();
        let public_key = pairs.secp256k1.ok_or("public key not found in record")?;
        let verifying_key = VerifyingKey::from_sec1_bytes(public_key.as_bytes())
            .map_err(|_| "public key could no be built from msg pub key bytes")?;
        let encoded = verifying_key.to_encoded_point(false);
        let node_id = H512::from_slice(&encoded.as_bytes()[1..]);

        let ip = pairs
            .ip
            .map(|p| IpAddr::from(Ipv4Addr::from_bits(p)))
            .ok_or("Ip not found in record, can't construct node")?;

        // both udp and tcp can be defined in the pairs or only one
        // in the latter case, we have to default both ports to the one provided
        let udp_port = pairs
            .udp_port
            .or(pairs.tcp_port)
            .ok_or("No port found in record")?;
        let tcp_port = pairs
            .tcp_port
            .or(pairs.udp_port)
            .ok_or("No port found in record")?;

        Ok(Self {
            ip,
            node_id,
            tcp_port,
            udp_port,
        })
    }

    pub fn enode_url(&self) -> String {
        let node_id = hex::encode(self.node_id);
        let node_ip = self.ip;
        let discovery_port = self.udp_port;
        let listener_port = self.tcp_port;
        if discovery_port != listener_port {
            format!("enode://{node_id}@{node_ip}:{listener_port}?discport={discovery_port}")
        } else {
            format!("enode://{node_id}@{node_ip}:{listener_port}")
        }
    }

    pub fn udp_addr(self) -> SocketAddr {
        SocketAddr::new(self.ip, self.udp_port)
    }

    pub fn tcp_addr(self) -> SocketAddr {
        SocketAddr::new(self.ip, self.tcp_port)
    }
}

impl Display for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!(
            "{0}({1}:{2})",
            self.node_id, self.ip, self.tcp_port
        ))
    }
}

/// Reference: [ENR records](https://github.com/ethereum/devp2p/blob/master/enr.md)
#[derive(Debug, PartialEq, Clone, Eq, Default)]
pub struct NodeRecord {
    pub signature: H512,
    pub seq: u64,
    // holds optional values in (key, value) format
    // value represents the rlp encoded bytes
    pub pairs: Vec<(Bytes, Bytes)>,
}

#[derive(Debug, Default, PartialEq)]
pub struct NodeRecordDecodedPairs {
    pub id: Option<String>,
    pub ip: Option<u32>,
    // the record structure reference says that tcp_port and udp_ports are big-endian integers
    // but they are actually encoded as 2 bytes, see geth for example: https://github.com/ethereum/go-ethereum/blob/f544fc3b4659aeca24a6de83f820dd61ea9b39db/p2p/enr/entries.go#L60-L78
    // I think the confusion comes from the fact that geth decodes the bytes and then builds an IPV4/6 big-integer structure.
    pub tcp_port: Option<u16>,
    pub udp_port: Option<u16>,
    pub secp256k1: Option<H264>,
    // TODO implement ipv6 addresses
}

impl NodeRecord {
    pub fn decode_pairs(&self) -> NodeRecordDecodedPairs {
        let mut decoded_pairs = NodeRecordDecodedPairs::default();
        for (key, value) in &self.pairs {
            let Ok(key) = String::from_utf8(key.to_vec()) else {
                continue;
            };
            let value = value.to_vec();
            match key.as_str() {
                "id" => decoded_pairs.id = String::decode(&value).ok(),
                "ip" => decoded_pairs.ip = u32::decode(&value).ok(),
                "tcp" => decoded_pairs.tcp_port = u16::decode(&value).ok(),
                "udp" => decoded_pairs.udp_port = u16::decode(&value).ok(),
                "secp256k1" => {
                    let Ok(bytes) = Bytes::decode(&value) else {
                        continue;
                    };
                    if bytes.len() < 33 {
                        continue;
                    }
                    decoded_pairs.secp256k1 = Some(H264::from_slice(&bytes))
                }
                _ => {}
            }
        }

        decoded_pairs
    }

    pub fn enr_url(&self) -> Result<String, String> {
        let rlp_encoded = self.encode_to_vec();
        let base64_encoded = ethrex_common::base64::encode(&rlp_encoded);
        let mut result: String = "enr:".into();
        let base64_encoded =
            String::from_utf8(base64_encoded).map_err(|_| "Could not base 64 encode enr record")?;
        result.push_str(&base64_encoded);
        Ok(result)
    }

    pub fn from_node(node: Node, seq: u64, signer: &SigningKey) -> Result<Self, String> {
        let mut record = NodeRecord {
            seq,
            ..Default::default()
        };
        record
            .pairs
            .push(("id".into(), "v4".encode_to_vec().into()));
        record
            .pairs
            .push(("ip".into(), node.ip.encode_to_vec().into()));
        record.pairs.push((
            "secp256k1".into(),
            signer
                .verifying_key()
                .to_encoded_point(true)
                .as_bytes()
                .encode_to_vec()
                .into(),
        ));
        record
            .pairs
            .push(("tcp".into(), node.tcp_port.encode_to_vec().into()));
        record
            .pairs
            .push(("udp".into(), node.udp_port.encode_to_vec().into()));

        record.signature = record.sign_record(signer)?;

        Ok(record)
    }

    fn sign_record(&mut self, signer: &SigningKey) -> Result<H512, String> {
        let digest = &self.get_signature_digest();
        let (signature, _recovery_id) = signer
            .sign_prehash_recoverable(digest)
            .map_err(|err| format!("Could not sign record: {err}"))?;
        let signature_bytes = signature.to_bytes().to_vec();

        Ok(H512::from_slice(&signature_bytes))
    }

    pub fn get_signature_digest(&self) -> Vec<u8> {
        let mut rlp = vec![];
        structs::Encoder::new(&mut rlp)
            .encode_field(&self.seq)
            .encode_key_value_list::<Bytes>(&self.pairs)
            .finish();
        let digest = Keccak256::digest(&rlp);
        digest.to_vec()
    }
}

impl RLPDecode for NodeRecord {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        if rlp.len() > MAX_NODE_RECORD_ENCODED_SIZE {
            return Err(RLPDecodeError::InvalidLength);
        }
        let decoder = Decoder::new(rlp)?;
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
            .encode_field(&self.node_id)
            .finish();
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        network::node_id_from_signing_key,
        types::{Node, NodeRecord},
    };
    use ethrex_common::H512;
    use k256::ecdsa::SigningKey;
    use std::{net::SocketAddr, str::FromStr};

    #[test]
    fn parse_node_from_enode_string() {
        let input = "enode://d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666@18.138.108.67:30303";
        let bootnode = Node::from_enode_url(input).unwrap();
        let node_id = H512::from_str(
            "d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666")
            .unwrap();
        let socket_address = SocketAddr::from_str("18.138.108.67:30303").unwrap();
        let expected_bootnode = Node {
            ip: socket_address.ip(),
            node_id,
            tcp_port: socket_address.port(),
            udp_port: socket_address.port(),
        };
        assert_eq!(bootnode, expected_bootnode);
    }

    #[test]
    fn parse_node_with_discport_from_enode_string() {
        let input = "enode://d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666@18.138.108.67:30303?discport=30305";
        let node = Node::from_enode_url(input).unwrap();
        let node_id = H512::from_str(
            "d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666")
            .unwrap();
        let socket_address = SocketAddr::from_str("18.138.108.67:30303").unwrap();
        let expected_bootnode = Node {
            ip: socket_address.ip(),
            node_id,
            tcp_port: socket_address.port(),
            udp_port: 30305,
        };
        assert_eq!(node, expected_bootnode);
    }

    #[test]
    fn parse_node_from_enr_string() {
        // https://github.com/ethereum/devp2p/blob/master/enr.md#test-vectors
        let enr_string = "enr:-IS4QHCYrYZbAKWCBRlAy5zzaDZXJBGkcnh4MHcBFZntXNFrdvJjX04jRzjzCBOonrkTfj499SZuOh8R33Ls8RRcy5wBgmlkgnY0gmlwhH8AAAGJc2VjcDI1NmsxoQPKY0yuDUmstAHYpMa2_oxVtw0RW_QAdpzBQA8yWM0xOIN1ZHCCdl8";
        let node = Node::from_enr_url(enr_string).unwrap();
        let node_id =
            H512::from_str("0xca634cae0d49acb401d8a4c6b6fe8c55b70d115bf400769cc1400f3258cd31387574077f301b421bc84df7266c44e9e6d569fc56be00812904767bf5ccd1fc7f")
                .unwrap();
        let socket_address = SocketAddr::from_str("127.0.0.1:30303").unwrap();
        let expected_node = Node {
            ip: socket_address.ip(),
            node_id,
            tcp_port: socket_address.port(),
            udp_port: socket_address.port(),
        };
        assert_eq!(node, expected_node);
    }

    #[test]
    fn encode_node_record_to_enr_url() {
        // https://github.com/ethereum/devp2p/blob/master/enr.md#test-vectors
        let signer = SigningKey::from_slice(&[
            16, 125, 177, 238, 167, 212, 168, 215, 239, 165, 77, 224, 199, 143, 55, 205, 9, 194,
            87, 139, 92, 46, 30, 191, 74, 37, 68, 242, 38, 225, 104, 246,
        ])
        .unwrap();
        let addr = std::net::SocketAddr::from_str("127.0.0.1:30303").unwrap();
        let node = Node {
            ip: addr.ip(),
            node_id: node_id_from_signing_key(&signer),
            tcp_port: addr.port(),
            udp_port: addr.port(),
        };
        let record = NodeRecord::from_node(node, 0, &signer).unwrap();
        let expected_enr_string = "enr:-Iu4QDOLZWVEdbtRUtrZ8PU1vxUJ0t_TUpVghJhJuakBUyYKE_ZfvhR2EKxDyJ8Z5wwoJE4mTSItAcYsErU0NrB7uzCAgmlkgnY0gmlwhH8AAAGJc2VjcDI1NmsxoQJtSDUljLLg3EYuRCp8QJvH8G2F9rmUAQtPKlZjq_O7loN0Y3CCdl-DdWRwgnZf";

        assert_eq!(record.enr_url().unwrap(), expected_enr_string);
    }
}
