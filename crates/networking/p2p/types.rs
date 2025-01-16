use bytes::{BufMut, Bytes};
use ethrex_core::{H264, H512};
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{self, Decoder, Encoder},
};
use k256::ecdsa::SigningKey;
use sha3::{Digest, Keccak256};
use std::net::{IpAddr, SocketAddr};

use crate::discv4::time_now_unix;

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

impl Node {
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
}

/// Reference: [ENR records](https://github.com/ethereum/devp2p/blob/master/enr.md)
#[derive(Debug, PartialEq, Clone, Eq, Default)]
pub struct NodeRecord {
    pub signature: H512,
    pub seq: u64,
    pub pairs: Vec<(Bytes, Bytes)>,
}

#[derive(Debug, Default)]
pub struct NodeRecordDecodedPairs {
    pub id: Option<String>,
    pub ip: Option<u32>,
    // the record structure reference says that tcp_port and udp_ports are big-endian integers
    // but they are actually encoded as 4 bytes, see geth for example: https://github.com/ethereum/go-ethereum/blob/master/p2p/enr/entries.go#L186-L196
    // I think the confusion comes from the fact that geth decodes the 4 bytes and then builds an IPV4 big-integer structure.
    pub tcp_port: Option<u32>,
    pub udp_port: Option<u32>,
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
            let construct_u32_from_value = || {
                if value.len() < 4 {
                    None
                } else {
                    Some(u32::from_be_bytes([value[0], value[1], value[2], value[3]]))
                }
            };
            match key.as_str() {
                "id" => decoded_pairs.id = String::from_utf8(value).ok(),
                "ip" => decoded_pairs.ip = construct_u32_from_value(),
                "tcp" => decoded_pairs.tcp_port = construct_u32_from_value(),
                "udp" => decoded_pairs.udp_port = construct_u32_from_value(),
                "secp256k1" => {
                    if value.len() < 33 {
                        continue;
                    }
                    decoded_pairs.secp256k1 = Some(H264::from_slice(&value.as_slice()))
                }
                _ => {}
            }
        }
        return decoded_pairs;
    }

    pub fn from_node(node: Node, signer: &SigningKey) -> Result<Self, ()> {
        let mut record = Self::default();
        record.seq = time_now_unix();
        record.pairs.push(("id".into(), "v4".into()));
        match node.ip {
            IpAddr::V4(ip) => record
                .pairs
                .push(("ip".into(), Bytes::copy_from_slice(&ip.octets()))),
            // TODO support ipv6
            IpAddr::V6(_) => {}
        }
        record.pairs.push((
            "secp256k1".into(),
            Bytes::copy_from_slice(signer.verifying_key().to_encoded_point(true).as_bytes()),
        ));
        record.pairs.push((
            "tcp".into(),
            Bytes::copy_from_slice(&(node.tcp_port as u32).to_be_bytes()),
        ));
        record.pairs.push((
            "udp".into(),
            Bytes::copy_from_slice(&(node.udp_port as u32).to_be_bytes()),
        ));

        if record.sign_record(signer).is_ok() {
            Ok(record)
        } else {
            Err(())
        }
    }

    pub fn sign_record(&mut self, signer: &SigningKey) -> Result<(), ()> {
        let mut rlp = vec![];
        self.seq.encode(&mut rlp);
        self.pairs.encode(&mut rlp);
        let digest = Keccak256::digest(&rlp);

        let Ok((signature, v)) = signer.sign_prehash_recoverable(&digest) else {
            return Err(());
        };
        let mut sign_bytes = signature.to_bytes().to_vec();
        sign_bytes.push(v.to_byte());

        self.signature = H512::from_slice(&sign_bytes);

        Ok(())
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
        let (pairs, decoder) = decode_node_record_optional_fields(vec![], decoder);

        // all fields in pairs are optional except for id
        let id_pair = pairs.iter().find(|(k, _v)| k.eq("id".as_bytes()));
        if let Some((_key, _)) = id_pair {
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
) -> (Vec<(Bytes, Bytes)>, Decoder) {
    let (key, decoder): (Option<Bytes>, Decoder) = decoder.decode_optional_field();
    if let Some(k) = key {
        let (value, decoder): (Vec<u8>, Decoder) = decoder.get_encoded_item().unwrap();
        pairs.push((k, Bytes::from(value)));
        decode_node_record_optional_fields(pairs, decoder)
    } else {
        (pairs, decoder)
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
