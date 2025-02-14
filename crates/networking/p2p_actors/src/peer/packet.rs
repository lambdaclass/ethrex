use ethrex_core::{H256, H512, H520};
use ethrex_rlp::{
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};

pub trait Packet: RLPEncode + Sized {
    fn try_rlp_decode(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError>;
}

pub struct Auth {
    pub signature: H520,
    pub initiator_pubkey: H512,
    pub nonce: H256,
    pub version: u8,
}

impl RLPEncode for Auth {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.signature)
            .encode_field(&self.initiator_pubkey)
            .encode_field(&self.nonce)
            .encode_field(&self.version)
            .finish()
    }
}

impl Packet for Auth {
    fn try_rlp_decode(rlp: &[u8]) -> Result<(Self, &[u8]), ethrex_rlp::error::RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (signature, decoder) = decoder.decode_field("signature")?;
        let (initiator_pubkey, decoder) = decoder.decode_field("initiator_pubkey")?;
        let (nonce, decoder) = decoder.decode_field("nonce")?;
        let (version, decoder) = decoder.decode_field("version")?;
        Ok((
            Auth {
                signature,
                initiator_pubkey,
                nonce,
                version,
            },
            decoder.finish_unchecked(),
        ))
    }
}

pub struct AuthAck {
    pub recipient_ephemeral_pubk: H512,
    pub recipient_nonce: H256,
    pub version: u8,
}

impl RLPEncode for AuthAck {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.recipient_ephemeral_pubk)
            .encode_field(&self.recipient_nonce)
            .encode_field(&self.version)
            .finish()
    }
}

impl Packet for AuthAck {
    fn try_rlp_decode(rlp: &[u8]) -> Result<(Self, &[u8]), ethrex_rlp::error::RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (recipient_ephemeral_pubk, decoder) =
            decoder.decode_field("recipient_ephemeral_pubk")?;
        let (recipient_nonce, decoder) = decoder.decode_field("recipient_nonce")?;
        let (version, decoder) = decoder.decode_field("version")?;
        Ok((
            Self {
                recipient_ephemeral_pubk,
                recipient_nonce,
                version,
            },
            decoder.finish_unchecked(),
        ))
    }
}
