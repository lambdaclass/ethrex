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

        match record.sign_record(signer) {
            Ok(sig) => record.signature = sig,
            Err(e) => return Err(e),
        };

        Ok(record)
    }

    fn sign_record(&mut self, signer: &SigningKey) -> Result<H512, String> {
        // note: v is ignored
        let digest = &self.get_signature_digest();
        let Ok((signature, _)) = signer.sign_prehash_recoverable(digest) else {
            return Err("Could not sign record.".to_string());
        };
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
