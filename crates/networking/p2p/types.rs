use bytes::{BufMut, Bytes};
use ethereum_rust_core::H512;
use ethereum_rust_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{self, Decoder, Encoder},
};
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
#[derive(Debug, PartialEq, Eq, Default)]
pub struct NodeRecord {
    pub signature: H512,
    pub seq: u64,
    pub id: String,
    pub pairs: Vec<(Bytes, Bytes)>,
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
        if let Some((_key, id)) = id_pair {
            let node_record = NodeRecord {
                signature,
                seq,
                id: String::decode(id).unwrap(),
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
