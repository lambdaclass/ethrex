use aes::cipher::KeyIvInit;
use bytes::BufMut;
use secp256k1::SecretKey;

type Aes256Ctr64BE = ctr::Ctr64BE<aes::Aes256>;

#[derive(Debug, thiserror::Error)]
pub enum PacketDecodeErr {
    #[error("Invalid packet size")]
    InvalidSize,
}

#[derive(Debug, Clone)]
pub struct Packet {
    message: Message,
}

impl Packet {
    pub fn decode(signer: &SecretKey, encoded_packet: &[u8]) -> Result<Packet, PacketDecodeErr> {
        // the packet structure is
        // masking-iv || masked-header || message

        // 16 bytes for an u128
        let masking_iv = &encoded_packet[..16];
        // 23 bytes for static header
        let _static_header = &encoded_packet[16..39];

        let public_key = signer.public_key(secp256k1::SECP256K1);

        // TODO: implement proper decoding
        let _cipher = <Aes256Ctr64BE as KeyIvInit>::new(
            public_key.serialize_uncompressed()[..16].into(),
            masking_iv.into(),
        );

        Ok(Self {
            message: Message::Ping(PingMessage {
                req_id: 1,
                enr_seq: 1,
            }),
        })
    }

    pub fn encode(&self, buf: &mut dyn BufMut, signer: &SecretKey) {
        self.message.encode(buf, signer);
    }
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum Message {
    Ping(PingMessage),
    // TODO: add the other messages
}

impl Message {
    pub fn encode(&self, _buf: &mut dyn BufMut, _signer: &SecretKey) {
        //TODO
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
    pub fn new(req_id: u64, enr_seq: u64) -> Self {
        Self { req_id, enr_seq }
    }
}

#[cfg(test)]
mod tests {
    use crate::discv5::messages::{Message, Packet, PingMessage};
    use hex_literal::hex;
    use secp256k1::SecretKey;

    // node-a-key = 0xeef77acb6c6a6eebc5b363a475ac583ec7eccdb42b6481424c60f59aa326547f
    // node-b-key = 0x66fb62bfbd66b9177a138c1e5cddbe4f7c30c343e94e68df8769459cb1cde628

    #[test]
    fn test_encode_ping_message() {
        // TODO
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
        let node_b_key = SecretKey::from_slice(&hex!(
            "66fb62bfbd66b9177a138c1e5cddbe4f7c30c343e94e68df8769459cb1cde628"
        ))
        .unwrap();

        let encoded = &hex!(
            "00000000000000000000000000000000088b3d4342774649325f313964a39e55ea96c005ad52be8c7560413a7008f16c9e6d2f43bbea8814a546b7409ce783d34c4f53245d08dab84102ed931f66d1492acb308fa1c6715b9d139b81acbdcc"
        );
        let decoded = Packet::decode(&node_b_key, encoded).unwrap();
        let message = decoded.message;
        let expected = Message::Ping(PingMessage {
            req_id: 0x00000001,
            enr_seq: 2,
        });

        assert_eq!(message, expected);
    }
}
