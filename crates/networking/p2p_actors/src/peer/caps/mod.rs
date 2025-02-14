use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};

pub mod p2p;

pub struct Capability {
    pub id: String,
    pub version: u8,
}

impl RLPEncode for Capability {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.id)
            .encode_field(&self.version)
            .finish();
    }
}

impl RLPDecode for Capability {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;

        let (id, decoder) = decoder.decode_field("capability_id")?;
        let (version, decoder) = decoder.decode_field("version")?;

        Ok((Capability { id, version }, decoder.finish_unchecked()))
    }
}
