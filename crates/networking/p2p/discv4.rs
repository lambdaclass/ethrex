use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::types::{Endpoint, Node, NodeRecord};
use bytes::BufMut;
use ethrex_core::{H256, H512, H520};
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{self, Decoder, Encoder},
};
use k256::ecdsa::{RecoveryId, Signature, SigningKey, VerifyingKey};
use sha3::{Digest, Keccak256};

//todo add tests
pub fn get_expiration(seconds: u64) -> u64 {
    (SystemTime::now() + Duration::from_secs(seconds))
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

pub fn is_expired(expiration: u64) -> bool {
    // this cast to a signed integer is needed as the rlp decoder doesn't take into account the sign
    // otherwise a potential negative expiration would pass since it would take 2^64.
    (expiration as i64)
        < SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
}

pub fn time_since_in_hs(time: u64) -> u64 {
    let time = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(time);
    SystemTime::now().duration_since(time).unwrap().as_secs() / 3600
}

pub fn time_now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[derive(Debug)]
pub enum PacketDecodeErr {
    #[allow(unused)]
    RLPDecodeError(RLPDecodeError),
    InvalidSize,
    HashMismatch,
    InvalidSignature,
}

#[allow(unused)]
#[derive(Debug)]
pub struct Packet {
    hash: H256,
    signature: H520,
    message: Message,
    node_id: H512,
}

impl Packet {
    pub fn decode(encoded_packet: &[u8]) -> Result<Packet, PacketDecodeErr> {
        // the packet structure is
        // hash || signature || packet-type || packet-data
        let hash_len = 32;
        let signature_len = 65;
        let header_size = hash_len + signature_len; // 97

        if encoded_packet.len() < header_size + 1 {
            return Err(PacketDecodeErr::InvalidSize);
        };

        let hash = H256::from_slice(&encoded_packet[..hash_len]);
        let signature_bytes = &encoded_packet[hash_len..header_size];
        let packet_type = encoded_packet[header_size];
        let encoded_msg = &encoded_packet[header_size..];

        let head_digest = Keccak256::digest(&encoded_packet[hash_len..]);
        let header_hash = H256::from_slice(&head_digest);

        if hash != header_hash {
            return Err(PacketDecodeErr::HashMismatch);
        }

        let digest = Keccak256::digest(encoded_msg);
        let signature = &Signature::from_slice(&signature_bytes[0..64]).unwrap();
        let rid = RecoveryId::from_byte(signature_bytes[64]).unwrap();

        let peer_pk = VerifyingKey::recover_from_prehash(&digest, signature, rid)
            .map_err(|_| PacketDecodeErr::InvalidSignature)?;
        let encoded = peer_pk.to_encoded_point(false);

        let node_id = H512::from_slice(&encoded.as_bytes()[1..]);
        let signature = H520::from_slice(signature_bytes);
        let message = Message::decode_with_type(packet_type, &encoded_msg[1..])
            .map_err(PacketDecodeErr::RLPDecodeError)?;

        Ok(Self {
            hash,
            signature,
            message,
            node_id,
        })
    }

    pub fn get_hash(&self) -> H256 {
        self.hash
    }

    pub fn get_message(&self) -> &Message {
        &self.message
    }

    #[allow(unused)]
    pub fn get_signature(&self) -> H520 {
        self.signature
    }

    pub fn get_node_id(&self) -> H512 {
        self.node_id
    }
}

#[derive(Debug, Eq, PartialEq)]
// NOTE: All messages could have more fields than specified by the spec.
// Those additional fields should be ignored, and the message must be accepted.
// TODO: remove when all variants are used
#[allow(dead_code)]
pub(crate) enum Message {
    /// A ping message. Should be responded to with a Pong message.
    Ping(PingMessage),
    Pong(PongMessage),
    FindNode(FindNodeMessage),
    Neighbors(NeighborsMessage),
    ENRRequest(ENRRequestMessage),
    ENRResponse(ENRResponseMessage),
}

impl Message {
    pub fn encode_with_header(&self, buf: &mut dyn BufMut, node_signer: &SigningKey) {
        let signature_size = 65_usize;
        let mut data: Vec<u8> = Vec::with_capacity(signature_size.next_power_of_two());
        data.resize(signature_size, 0);

        self.encode_with_type(&mut data);

        let digest = Keccak256::digest(&data[signature_size..]);

        let (signature, recovery_id) = node_signer
            .sign_prehash_recoverable(&digest)
            .expect("failed to sign");
        let b = signature.to_bytes();

        data[..signature_size - 1].copy_from_slice(&b);
        data[signature_size - 1] = recovery_id.to_byte();

        let hash = Keccak256::digest(&data[..]);
        buf.put_slice(&hash);
        buf.put_slice(&data[..]);
    }

    fn encode_with_type(&self, buf: &mut dyn BufMut) {
        buf.put_u8(self.packet_type());
        match self {
            Message::Ping(msg) => msg.encode(buf),
            Message::Pong(msg) => msg.encode(buf),
            Message::FindNode(msg) => msg.encode(buf),
            Message::ENRRequest(msg) => msg.encode(buf),
            Message::ENRResponse(msg) => msg.encode(buf),
            Message::Neighbors(msg) => msg.encode(buf),
        }
    }

    pub fn decode_with_type(packet_type: u8, msg: &[u8]) -> Result<Message, RLPDecodeError> {
        // NOTE: extra elements inside the message should be ignored, along with extra data
        // after the message.
        match packet_type {
            0x01 => {
                let (ping, _rest) = PingMessage::decode_unfinished(msg)?;
                Ok(Message::Ping(ping))
            }
            0x02 => {
                let (pong, _rest) = PongMessage::decode_unfinished(msg)?;
                Ok(Message::Pong(pong))
            }
            0x03 => {
                let (find_node_msg, _rest) = FindNodeMessage::decode_unfinished(msg)?;
                Ok(Message::FindNode(find_node_msg))
            }
            0x04 => {
                let (neighbors_msg, _rest) = NeighborsMessage::decode_unfinished(msg)?;
                Ok(Message::Neighbors(neighbors_msg))
            }
            0x05 => {
                let (enr_request_msg, _rest) = ENRRequestMessage::decode_unfinished(msg)?;
                Ok(Message::ENRRequest(enr_request_msg))
            }
            0x06 => {
                let (enr_response_msg, _rest) = ENRResponseMessage::decode_unfinished(msg)?;
                Ok(Message::ENRResponse(enr_response_msg))
            }
            _ => Err(RLPDecodeError::MalformedData),
        }
    }

    fn packet_type(&self) -> u8 {
        match self {
            Message::Ping(_) => 0x01,
            Message::Pong(_) => 0x02,
            Message::FindNode(_) => 0x03,
            Message::Neighbors(_) => 0x04,
            Message::ENRRequest(_) => 0x05,
            Message::ENRResponse(_) => 0x06,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PingMessage {
    /// The Ping message version. Should be set to 4, but mustn't be enforced.
    version: u8,
    /// The endpoint of the sender.
    pub from: Endpoint,
    /// The endpoint of the receiver.
    pub to: Endpoint,
    /// The expiration time of the message. If the message is older than this time,
    /// it shouldn't be responded to.
    pub expiration: u64,
    /// The ENR sequence number of the sender. This field is optional.
    pub enr_seq: Option<u64>,
}

impl PingMessage {
    pub fn new(from: Endpoint, to: Endpoint, expiration: u64) -> Self {
        Self {
            version: 4,
            from,
            to,
            expiration,
            enr_seq: None,
        }
    }

    // TODO: remove when used
    #[allow(unused)]
    pub fn with_enr_seq(self, enr_seq: u64) -> Self {
        Self {
            enr_seq: Some(enr_seq),
            ..self
        }
    }
}

impl RLPEncode for PingMessage {
    fn encode(&self, buf: &mut dyn BufMut) {
        structs::Encoder::new(buf)
            .encode_field(&self.version)
            .encode_field(&self.from)
            .encode_field(&self.to)
            .encode_field(&self.expiration)
            .encode_optional_field(&self.enr_seq)
            .finish();
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct FindNodeMessage {
    /// The target is a 64-byte secp256k1 public key.
    pub target: H512,
    /// The expiration time of the message. If the message is older than this time,
    /// it shouldn't be responded to.
    pub expiration: u64,
}

impl FindNodeMessage {
    #[allow(unused)]
    pub fn new(target: H512, expiration: u64) -> Self {
        Self { target, expiration }
    }
}

impl RLPEncode for FindNodeMessage {
    fn encode(&self, buf: &mut dyn BufMut) {
        structs::Encoder::new(buf)
            .encode_field(&self.target)
            .encode_field(&self.expiration)
            .finish();
    }
}

impl RLPDecode for FindNodeMessage {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (target, decoder) = decoder.decode_field("target")?;
        let (expiration, decoder) = decoder.decode_field("expiration")?;
        let remaining = decoder.finish_unchecked();
        let msg = FindNodeMessage { target, expiration };
        Ok((msg, remaining))
    }
}

#[derive(Debug, Clone)]
pub struct FindNodeRequest {
    /// the number of nodes sent
    /// we keep track of this number since we will accept neighbor messages until the max_per_bucket
    pub nodes_sent: usize,
    /// unix timestamp tracking when we have sent the request
    pub sent_at: u64,
    /// if present, server will send the nodes through this channel when receiving neighbors
    /// useful to wait for the response in lookups
    pub tx: Option<tokio::sync::mpsc::UnboundedSender<Vec<Node>>>,
}

impl Default for FindNodeRequest {
    fn default() -> Self {
        Self {
            nodes_sent: 0,
            sent_at: time_now_unix(),
            tx: None,
        }
    }
}

impl FindNodeRequest {
    pub fn new_with_sender(sender: tokio::sync::mpsc::UnboundedSender<Vec<Node>>) -> Self {
        Self {
            tx: Some(sender),
            ..Self::default()
        }
    }
}

impl RLPDecode for PingMessage {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (version, decoder): (u8, Decoder) = decoder.decode_field("version")?;
        let (from, decoder) = decoder.decode_field("from")?;
        let (to, decoder) = decoder.decode_field("to")?;
        let (expiration, decoder) = decoder.decode_field("expiration")?;
        let (enr_seq, decoder) = decoder.decode_optional_field();

        let ping = PingMessage {
            version,
            from,
            to,
            expiration,
            enr_seq,
        };
        // NOTE: as per the spec, any additional elements should be ignored.
        let remaining = decoder.finish_unchecked();
        Ok((ping, remaining))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PongMessage {
    /// The endpoint of the receiver.
    pub to: Endpoint,
    /// The hash of the corresponding ping packet.
    pub ping_hash: H256,
    /// The expiration time of the message. If the message is older than this time,
    /// it shouldn't be responded to.
    pub expiration: u64,
    /// The ENR sequence number of the sender. This field is optional.
    pub enr_seq: Option<u64>,
}

impl PongMessage {
    #[allow(unused)]
    pub fn new(to: Endpoint, ping_hash: H256, expiration: u64) -> Self {
        Self {
            to,
            ping_hash,
            expiration,
            enr_seq: None,
        }
    }

    // TODO: remove when used
    #[allow(unused)]
    pub fn with_enr_seq(self, enr_seq: u64) -> Self {
        Self {
            enr_seq: Some(enr_seq),
            ..self
        }
    }
}

impl RLPEncode for PongMessage {
    fn encode(&self, buf: &mut dyn BufMut) {
        Encoder::new(buf)
            .encode_field(&self.to)
            .encode_field(&self.ping_hash)
            .encode_field(&self.expiration)
            .encode_optional_field(&self.enr_seq)
            .finish();
    }
}

impl RLPDecode for PongMessage {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (to, decoder) = decoder.decode_field("to")?;
        let (ping_hash, decoder) = decoder.decode_field("ping_hash")?;
        let (expiration, decoder) = decoder.decode_field("expiration")?;
        let (enr_seq, decoder) = decoder.decode_optional_field();

        let pong = PongMessage {
            to,
            ping_hash,
            expiration,
            enr_seq,
        };
        // NOTE: as per the spec, any additional elements should be ignored.
        let remaining = decoder.finish_unchecked();
        Ok((pong, remaining))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NeighborsMessage {
    // nodes is the list of neighbors
    pub nodes: Vec<Node>,
    pub expiration: u64,
}

impl NeighborsMessage {
    pub fn new(nodes: Vec<Node>, expiration: u64) -> Self {
        Self { nodes, expiration }
    }
}

impl RLPDecode for NeighborsMessage {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (nodes, decoder) = decoder.decode_field("nodes")?;
        let (expiration, decoder) = decoder.decode_field("expiration")?;
        let remaining = decoder.finish_unchecked();

        let neighbors = NeighborsMessage::new(nodes, expiration);
        Ok((neighbors, remaining))
    }
}

impl RLPEncode for NeighborsMessage {
    fn encode(&self, buf: &mut dyn BufMut) {
        structs::Encoder::new(buf)
            .encode_field(&self.nodes)
            .encode_field(&self.expiration)
            .finish();
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ENRResponseMessage {
    pub request_hash: H256,
    pub node_record: NodeRecord,
}

impl RLPDecode for ENRResponseMessage {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (request_hash, decoder) = decoder.decode_field("request_hash")?;
        let (node_record, decoder) = decoder.decode_field("node_record")?;
        let remaining = decoder.finish_unchecked();
        let response = ENRResponseMessage {
            request_hash,
            node_record,
        };
        Ok((response, remaining))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ENRRequestMessage {
    expiration: u64,
}

impl RLPDecode for ENRRequestMessage {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (expiration, decoder) = decoder.decode_field("expiration")?;
        let remaining = decoder.finish_unchecked();
        let enr_request = ENRRequestMessage { expiration };
        Ok((enr_request, remaining))
    }
}

impl RLPEncode for ENRRequestMessage {
    fn encode(&self, buf: &mut dyn BufMut) {
        structs::Encoder::new(buf)
            .encode_field(&self.expiration)
            .finish();
    }
}

impl RLPEncode for ENRResponseMessage {
    fn encode(&self, buf: &mut dyn BufMut) {
        structs::Encoder::new(buf)
            .encode_field(&self.request_hash)
            .encode_field(&self.node_record)
            .finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use ethrex_core::{H256, H264};
    use std::fmt::Write;
    use std::net::IpAddr;
    use std::num::ParseIntError;
    use std::str::FromStr;

    fn to_hex(bytes: &[u8]) -> String {
        bytes.iter().fold(String::new(), |mut buf, b| {
            let _ = write!(&mut buf, "{b:02x}");
            buf
        })
    }

    #[test]
    fn test_encode_ping_message() {
        let expiration: u64 = 17195043770;

        let from = Endpoint {
            ip: IpAddr::from_str("1.2.3.4").unwrap(),
            udp_port: 1613,
            tcp_port: 6363,
        };
        let to = Endpoint {
            ip: IpAddr::from_str("255.255.2.5").unwrap(),
            udp_port: 3063,
            tcp_port: 0,
        };

        let msg = Message::Ping(PingMessage::new(from, to, expiration));

        let key_bytes =
            H256::from_str("577d8278cc7748fad214b5378669b420f8221afb45ce930b7f22da49cbc545f3")
                .unwrap();
        let signer = SigningKey::from_slice(key_bytes.as_bytes()).unwrap();

        let mut buf = Vec::new();

        msg.encode_with_header(&mut buf, &signer);
        let result = to_hex(&buf);
        let hash = "d2821841963050aa505c00d8e4fd2d016f95eff739b784e0e26587a58226738e";
        let signature = "8a73f13d613c0ba5148787bb52fd04eb984c3dae486bac19433adf658d29bbb352f3acf2d55f2bdae3afff5298723114581e3f34c37815b32b9195a3326dd68700";
        let pkt_type = "01";
        let encoded_message = "dd04cb840102030482064d8218dbc984ffff0205820bf780850400e78bba";
        let expected = [hash, signature, pkt_type, encoded_message].concat();
        assert_eq!(result, expected);
    }

    #[test]
    fn test_encode_pong_message_with_enr_seq() {
        let to = Endpoint {
            ip: IpAddr::from_str("190.191.188.57").unwrap(),
            udp_port: 30303,
            tcp_port: 0,
        };
        let expiration: u64 = 1719507696;
        let ping_hash: H256 =
            H256::from_str("3e1bf98f025f98d54ed2f61bbef63b6b46f50e12d7b937d6bdea19afd640be23")
                .unwrap();
        let enr_seq = 1704896740573;
        let msg = Message::Pong(PongMessage::new(to, ping_hash, expiration).with_enr_seq(enr_seq));

        let key_bytes =
            H256::from_str("577d8278cc7748fad214b5378669b420f8221afb45ce930b7f22da49cbc545f3")
                .unwrap();
        let signer = SigningKey::from_slice(key_bytes.as_bytes()).unwrap();

        let mut buf = Vec::new();

        msg.encode_with_header(&mut buf, &signer);
        let result = to_hex(&buf);
        let hash = "9657e4e2db33b51cbbeb503bd195efcf081d6a83befbb42b4be95d0f7bf27ffe";
        let signature = "b1a91caa6105b941d3ecce052dcfea5e4f4290c9e6a89ff72707a8b5116ee87a1ea3fa0086990cd862a8a2347f346f1b118122a28bf2ed2ca371d2c493a86bde01";
        let pkt_type = "02";
        let msg = "f7c984bebfbc3982765f80a03e1bf98f025f98d54ed2f61bbef63b6b46f50e12d7b937d6bdea19afd640be2384667d9af086018cf3c3bcdd";
        let expected = [hash, signature, pkt_type, msg].concat();

        assert_eq!(result, expected);
    }

    #[test]
    fn test_encode_pong_message() {
        let to = Endpoint {
            ip: IpAddr::from_str("190.191.188.57").unwrap(),
            udp_port: 30303,
            tcp_port: 0,
        };
        let expiration: u64 = 1719507696;
        let ping_hash: H256 =
            H256::from_str("3e1bf98f025f98d54ed2f61bbef63b6b46f50e12d7b937d6bdea19afd640be23")
                .unwrap();
        let msg = Message::Pong(PongMessage::new(to, ping_hash, expiration));
        let key_bytes =
            H256::from_str("577d8278cc7748fad214b5378669b420f8221afb45ce930b7f22da49cbc545f3")
                .unwrap();
        let signer = SigningKey::from_slice(key_bytes.as_bytes()).unwrap();

        let mut buf = Vec::new();

        msg.encode_with_header(&mut buf, &signer);
        let result = to_hex(&buf);
        let hash = "58a1d0ea66afd9617c198b60a7417637ae27b847b004dbebc1e29d4067327e35";
        let signature = "e1988832d7d7b73925ec584ff818ff3a7bffe1a84fe3835923c3ab17af40071f7c9263176203c80c6ed77f0586479b78884e9e47fdb3287d2aafa92348e5c16700";
        let pkt_type = "02";
        let msg = "f0c984bebfbc3982765f80a03e1bf98f025f98d54ed2f61bbef63b6b46f50e12d7b937d6bdea19afd640be2384667d9af0";
        let expected = [hash, signature, pkt_type, msg].concat();
        assert_eq!(result, expected);
    }

    #[test]
    fn test_encode_find_node_message() {
        let target: H512 = H512::from_str("d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666").unwrap();
        let expiration: u64 = 17195043770;

        let msg = Message::FindNode(FindNodeMessage::new(target, expiration));

        let key_bytes =
            H256::from_str("577d8278cc7748fad214b5378669b420f8221afb45ce930b7f22da49cbc545f3")
                .unwrap();
        let signer = SigningKey::from_slice(key_bytes.as_bytes()).unwrap();

        let mut buf = Vec::new();

        msg.encode_with_header(&mut buf, &signer);
        let result = to_hex(&buf);
        let hash = "23770430fc208bdc78bc77052bf7ec2e928b38c13c085b87491c15ebebb2050f";
        let signature = "7c98bb4759569117031a9fbbeb00314d018eba55135c65ee98dbf6871aaebe61225f36b36e4f60da5b5d6c917e3589dd235acfacc6de4dade116c4bb851b884b01";
        let pkt_type = "03";
        let encoded_message = "f848b840d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666850400e78bba";
        let expected = [hash, signature, pkt_type, encoded_message].concat();

        assert_eq!(result, expected);
    }

    #[test]
    fn test_encode_neighbors_message() {
        let expiration: u64 = 17195043770;
        let node_id_1 = H512::from_str("d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666").unwrap();
        let node_1 = Node {
            ip: "127.0.0.1".parse().unwrap(),
            udp_port: 30303,
            tcp_port: 30303,
            node_id: node_id_1,
        };

        let node_id_2 = H512::from_str("11f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f50").unwrap();
        let node_2 = Node {
            ip: "190.191.188.57".parse().unwrap(),
            udp_port: 30303,
            tcp_port: 30303,
            node_id: node_id_2,
        };
        let key_bytes =
            H256::from_str("577d8278cc7748fad214b5378669b420f8221afb45ce930b7f22da49cbc545f3")
                .unwrap();
        let signer = SigningKey::from_slice(key_bytes.as_bytes()).unwrap();

        let mut buf = Vec::new();
        let msg = Message::Neighbors(NeighborsMessage::new(vec![node_1, node_2], expiration));
        msg.encode_with_header(&mut buf, &signer);
        let result = to_hex(&buf);

        let hash = "a009d1dae92e9b3f6e48811ba70c1fec1a9d6f818139604b0e3abcaeabb74850";
        let signature = "f996d31ba3a409ba3c64121d8afa70ef10553d4da327594ac63225b53a34906d1e4d45312771d7bcf6390ef541157e688c7db946295c2d0712c50698a0fb8c9b00";
        let packet_type = "04";
        let encoded_msg = "f8a6f89ef84d847f00000182765f82765fb840d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666f84d84bebfbc3982765f82765fb84011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f50850400e78bba";
        let expected = [hash, signature, packet_type, encoded_msg].concat();

        assert_eq!(result, expected);
    }

    #[test]
    fn test_encode_enr_request_message() {
        let expiration: u64 = 17195043770;
        let msg = Message::ENRRequest(ENRRequestMessage { expiration });
        let key_bytes =
            H256::from_str("577d8278cc7748fad214b5378669b420f8221afb45ce930b7f22da49cbc545f3")
                .unwrap();
        let signer = SigningKey::from_slice(key_bytes.as_bytes()).unwrap();
        let mut buf = Vec::new();
        msg.encode_with_header(&mut buf, &signer);
        let result = to_hex(&buf);
        let hash = "ddb4faf81ed7bee047e42088a0efd01650c2191988c08c71dd10635573bee31f";
        let signature = "ec86b35edf60470d81e9796bc4fad68c1d187266492662d91f56b7e42ed46b9317444a72172f13aa91af41ca7a4fec49d5619de9abc0be6c79da0d92bc1c9f3201";
        let pkt_type = "05";
        let encoded_message = "c6850400e78bba";
        let expected = [hash, signature, pkt_type, encoded_message].concat();

        assert_eq!(result, expected);
    }

    #[test]
    fn test_encode_enr_response() {
        let request_hash =
            H256::from_str("ebc0a41dfdf5499552fb7e61799c577360a442170dbed4cb0745d628f06d9f98")
                .unwrap();
        let signature = H512::from_str("131d8cbc28a2dee4cae36ee3c268c44877e77eb248758d5a204df36b29a13ee53100fd47d3d6fd498ea48349d822d0965904fabcdeeecd9f5133a6062abdfbe3").unwrap();
        let seq = 0x018cf3c3bd18;

        // define optional fields
        let eth: Vec<Vec<u32>> = vec![vec![0x88cf81d9, 0]];
        let id = String::from("v4");
        let ip = IpAddr::from_str("138.197.51.181").unwrap();
        let secp256k1 =
            H264::from_str("034e5e92199ee224a01932a377160aa432f31d0b351f84ab413a8e0a42f4f36476")
                .unwrap();
        let tcp: u16 = 30303;
        let udp: u16 = 30303;
        let snap: Vec<u32> = vec![];

        // declare buffers for optional fields encoding
        let mut eth_rlp = Vec::new();
        let mut id_rlp = Vec::new();
        let mut ip_rlp = Vec::new();
        let mut secp256k1_rlp = Vec::new();
        let mut tcp_rlp = Vec::new();
        let mut udp_rlp = Vec::new();
        let mut snap_rlp = Vec::new();

        // encode optional fields
        eth.encode(&mut eth_rlp);
        id.encode(&mut id_rlp);
        ip.encode(&mut ip_rlp);
        secp256k1.encode(&mut secp256k1_rlp);
        tcp.encode(&mut tcp_rlp);
        udp.encode(&mut udp_rlp);
        snap.encode(&mut snap_rlp);

        // initialize vector with (key, value) pairs
        let pairs: Vec<(Bytes, Bytes)> = vec![
            (String::from("eth").into(), eth_rlp.into()),
            (String::from("id").into(), id_rlp.into()),
            (String::from("ip").into(), ip_rlp.into()),
            (String::from("secp256k1").into(), secp256k1_rlp.into()),
            (String::from("snap").into(), snap_rlp.into()),
            (String::from("tcp").into(), tcp_rlp.clone().into()),
            (String::from("udp").into(), udp_rlp.clone().into()),
        ];
        let node_record = NodeRecord {
            signature,
            seq,
            id: String::from("v4"),
            pairs,
        };
        let msg = Message::ENRResponse(ENRResponseMessage {
            request_hash,
            node_record,
        });

        let key_bytes =
            H256::from_str("2e6a09427ba14acc853cbbff291c75c3cb57754ac1e3df8df9cac086b3a83aa4")
                .unwrap();
        let signer = SigningKey::from_slice(key_bytes.as_bytes()).unwrap();
        let mut buf = Vec::new();
        msg.encode_with_header(&mut buf, &signer);
        let result = to_hex(&buf);

        let hash = "85e7d3ee8494d23694e2cbcc495be900bb035969366c4b3267ba80eef6cc9b2a";
        let signature = "7b714d79b4f8ec780b27329a6a8cb8188b882ecf99be0f89feeab33ebbb76ecb3dcb5ab53a1c7f27a4fc9e6e70220e614de9a351c3f39e100f40b5d0e2a7331501";
        let packet_type = "06";
        let encoded_msg = "f8c6a0ebc0a41dfdf5499552fb7e61799c577360a442170dbed4cb0745d628f06d9f98f8a3b840131d8cbc28a2dee4cae36ee3c268c44877e77eb248758d5a204df36b29a13ee53100fd47d3d6fd498ea48349d822d0965904fabcdeeecd9f5133a6062abdfbe386018cf3c3bd1883657468c7c68488cf81d980826964827634826970848ac533b589736563703235366b31a1034e5e92199ee224a01932a377160aa432f31d0b351f84ab413a8e0a42f4f3647684736e6170c08374637082765f8375647082765f";
        let expected = [hash, signature, packet_type, encoded_msg].concat();

        assert_eq!(result, expected);
    }

    #[test]
    fn test_decode_pong_message_with_enr_seq() {
        let hash = "2e1fc2a02ad95a1742f6dd41fb7cbff1e08548ba87f63a72221e44026ab1c347";
        let signature = "34f486e4e92f2fdf592912aa77ad51db532dd7f9b426092384c9c2e9919414fd480d57f4f3b2b1964ed6eb1c94b1e4b9f6bfe9b44b1d1ac3d94c38c4cce915bc01";
        let pkt_type = "02";
        let msg = "f7c984bebfbc3982765f80a03e1bf98f025f98d54ed2f61bbef63b6b46f50e12d7b937d6bdea19afd640be2384667d9af086018cf3c3bcdd";
        let encoded_packet = [hash, signature, pkt_type, msg].concat();

        let decoded_packet = Packet::decode(&decode_hex(&encoded_packet).unwrap()).unwrap();
        let decoded_msg = decoded_packet.get_message();
        let to = Endpoint {
            ip: IpAddr::from_str("190.191.188.57").unwrap(),
            udp_port: 30303,
            tcp_port: 0,
        };
        let expiration: u64 = 1719507696;
        let ping_hash: H256 =
            H256::from_str("3e1bf98f025f98d54ed2f61bbef63b6b46f50e12d7b937d6bdea19afd640be23")
                .unwrap();
        let enr_seq = 1704896740573;
        let expected =
            Message::Pong(PongMessage::new(to, ping_hash, expiration).with_enr_seq(enr_seq));
        assert_eq!(decoded_msg, &expected);
    }

    #[test]
    fn test_decode_pong_message() {
        // in this case the pong message does not contain the `enr_seq` field
        let hash = "65603d1ee62b03a0c2bf31549910f7bd5a783d82e9b83f5d4709083a7a4932f2";
        let signature = "34f486e4e92f2fdf592912aa77ad51db532dd7f9b426092384c9c2e9919414fd480d57f4f3b2b1964ed6eb1c94b1e4b9f6bfe9b44b1d1ac3d94c38c4cce915bc01";
        let pkt_type = "02";
        let msg = "f0c984bebfbc3982765f80a03e1bf98f025f98d54ed2f61bbef63b6b46f50e12d7b937d6bdea19afd640be2384667d9af0";
        let encoded_packet = [hash, signature, pkt_type, msg].concat();

        let decoded_packet = Packet::decode(&decode_hex(&encoded_packet).unwrap()).unwrap();
        let decoded_msg = decoded_packet.get_message();

        let to = Endpoint {
            ip: IpAddr::from_str("190.191.188.57").unwrap(),
            udp_port: 30303,
            tcp_port: 0,
        };
        let expiration: u64 = 1719507696;
        let ping_hash: H256 =
            H256::from_str("3e1bf98f025f98d54ed2f61bbef63b6b46f50e12d7b937d6bdea19afd640be23")
                .unwrap();
        let expected = Message::Pong(PongMessage::new(to, ping_hash, expiration));
        assert_eq!(decoded_msg, &expected);
    }

    #[test]
    fn test_decode_enr_response() {
        let encoded = "f8c6a0ebc0a41dfdf5499552fb7e61799c577360a442170dbed4cb0745d628f06d9f98f8a3b840131d8cbc28a2dee4cae36ee3c268c44877e77eb248758d5a204df36b29a13ee53100fd47d3d6fd498ea48349d822d0965904fabcdeeecd9f5133a6062abdfbe386018cf3c3bd1883657468c7c68488cf81d980826964827634826970848ac533b589736563703235366b31a1034e5e92199ee224a01932a377160aa432f31d0b351f84ab413a8e0a42f4f3647684736e6170c08374637082765f8375647082765f";
        let decoded = Message::decode_with_type(0x06, &decode_hex(encoded).unwrap()).unwrap();
        let request_hash =
            H256::from_str("ebc0a41dfdf5499552fb7e61799c577360a442170dbed4cb0745d628f06d9f98")
                .unwrap();
        let signature = H512::from_str("131d8cbc28a2dee4cae36ee3c268c44877e77eb248758d5a204df36b29a13ee53100fd47d3d6fd498ea48349d822d0965904fabcdeeecd9f5133a6062abdfbe3").unwrap();
        let seq = 0x018cf3c3bd18;

        // define optional fields
        let eth: Vec<Vec<u32>> = vec![vec![0x88cf81d9, 0]];
        let id = String::from("v4");
        let ip = IpAddr::from_str("138.197.51.181").unwrap();
        let secp256k1 =
            H264::from_str("034e5e92199ee224a01932a377160aa432f31d0b351f84ab413a8e0a42f4f36476")
                .unwrap();
        let tcp: u16 = 30303;
        let udp: u16 = 30303;
        let snap: Vec<u32> = vec![];

        // declare buffers for optional fields encoding
        let mut eth_rlp = Vec::new();
        let mut id_rlp = Vec::new();
        let mut ip_rlp = Vec::new();
        let mut secp256k1_rlp = Vec::new();
        let mut tcp_rlp = Vec::new();
        let mut udp_rlp = Vec::new();
        let mut snap_rlp = Vec::new();

        // encode optional fields
        eth.encode(&mut eth_rlp);
        id.encode(&mut id_rlp);
        ip.encode(&mut ip_rlp);
        secp256k1.encode(&mut secp256k1_rlp);
        tcp.encode(&mut tcp_rlp);
        udp.encode(&mut udp_rlp);
        snap.encode(&mut snap_rlp);

        // initialize vector with (key, value) pairs
        let pairs: Vec<(Bytes, Bytes)> = vec![
            (String::from("eth").into(), eth_rlp.into()),
            (String::from("id").into(), id_rlp.into()),
            (String::from("ip").into(), ip_rlp.into()),
            (String::from("secp256k1").into(), secp256k1_rlp.into()),
            (String::from("snap").into(), snap_rlp.into()),
            (String::from("tcp").into(), tcp_rlp.clone().into()),
            (String::from("udp").into(), udp_rlp.clone().into()),
        ];
        let node_record = NodeRecord {
            signature,
            seq,
            id: String::from("v4"),
            pairs,
        };
        let expected = Message::ENRResponse(ENRResponseMessage {
            request_hash,
            node_record,
        });

        assert_eq!(decoded, expected);
    }

    pub fn decode_hex(s: &str) -> Result<Vec<u8>, ParseIntError> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16))
            .collect()
    }

    #[test]
    fn test_decode_ping_message() {
        let expiration: u64 = 17195043770;

        let from = Endpoint {
            ip: IpAddr::from_str("1.2.3.4").unwrap(),
            udp_port: 1613,
            tcp_port: 6363,
        };
        let to = Endpoint {
            ip: IpAddr::from_str("255.255.2.5").unwrap(),
            udp_port: 3063,
            tcp_port: 0,
        };

        let msg = Message::Ping(PingMessage::new(from, to, expiration));

        let key_bytes =
            H256::from_str("577d8278cc7748fad214b5378669b420f8221afb45ce930b7f22da49cbc545f3")
                .unwrap();
        let signer = SigningKey::from_slice(key_bytes.as_bytes()).unwrap();

        let mut buf = Vec::new();

        msg.encode_with_header(&mut buf, &signer);
        let decoded_packet = Packet::decode(&buf).unwrap();
        let decoded_msg = decoded_packet.get_message();
        assert_eq!(decoded_msg, &msg);
    }

    #[test]
    fn test_decode_ping_message_with_enr_seq() {
        let expiration: u64 = 17195043770;

        let from = Endpoint {
            ip: IpAddr::from_str("1.2.3.4").unwrap(),
            udp_port: 1613,
            tcp_port: 6363,
        };
        let to = Endpoint {
            ip: IpAddr::from_str("255.255.2.5").unwrap(),
            udp_port: 3063,
            tcp_port: 0,
        };

        let enr_seq = 1704896740573;
        let msg = Message::Ping(PingMessage::new(from, to, expiration).with_enr_seq(enr_seq));

        let key_bytes =
            H256::from_str("577d8278cc7748fad214b5378669b420f8221afb45ce930b7f22da49cbc545f3")
                .unwrap();
        let signer = SigningKey::from_slice(key_bytes.as_bytes()).unwrap();

        let mut buf = Vec::new();

        msg.encode_with_header(&mut buf, &signer);
        let decoded_packet = Packet::decode(&buf).unwrap();
        let decoded_msg = decoded_packet.get_message();
        assert_eq!(decoded_msg, &msg);
    }

    #[test]
    fn test_decode_find_node_message() {
        let target: H512 = H512::from_str("d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666").unwrap();
        let expiration: u64 = 17195043770;
        let msg = Message::FindNode(FindNodeMessage::new(target, expiration));

        let key_bytes =
            H256::from_str("577d8278cc7748fad214b5378669b420f8221afb45ce930b7f22da49cbc545f3")
                .unwrap();
        let signer = SigningKey::from_slice(key_bytes.as_bytes()).unwrap();

        let mut buf = Vec::new();

        msg.encode_with_header(&mut buf, &signer);
        let decoded_packet = Packet::decode(&buf).unwrap();
        let decoded_msg = decoded_packet.get_message();
        assert_eq!(decoded_msg, &msg);
    }

    #[test]
    fn test_decode_neighbors_message() {
        let encoded = "f857f84ff84d847f00000182765f82765fb840d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666850400e78bba";
        let decoded = Message::decode_with_type(0x04, &decode_hex(encoded).unwrap()).unwrap();
        let expiration: u64 = 17195043770;
        let node_id = H512::from_str("d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666").unwrap();
        let node = Node {
            ip: "127.0.0.1".parse().unwrap(),
            udp_port: 30303,
            tcp_port: 30303,
            node_id,
        };

        let expected = Message::Neighbors(NeighborsMessage::new(vec![node], expiration));
        assert_eq!(decoded, expected);
    }

    #[test]
    fn test_decode_enr_request_message() {
        let encoded = "c6850400e78bba";
        let decoded = Message::decode_with_type(0x05, &decode_hex(encoded).unwrap()).unwrap();
        let expiration = 0x400E78BBA;
        let expected = Message::ENRRequest(ENRRequestMessage { expiration });
        assert_eq!(decoded, expected);
    }

    #[test]
    fn test_decode_endpoint() {
        let endpoint = Endpoint {
            ip: IpAddr::from_str("255.255.2.5").unwrap(),
            udp_port: 3063,
            tcp_port: 0,
        };

        let encoded = {
            let mut buf = vec![];
            endpoint.encode(&mut buf);
            buf
        };
        let decoded = Endpoint::decode(&encoded).expect("Failed decoding Endpoint");
        assert_eq!(endpoint, decoded);
    }
}
